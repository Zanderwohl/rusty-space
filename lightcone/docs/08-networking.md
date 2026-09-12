# Networking

## Shape of the problem

The usual MMO problem is sending frequent state updates to many clients with low latency.
This game has a different one:

- In-system, positions are analytic. The server never sends a position; it sends the
  parameters that generate one, and only when they change.
- Interstellar, the information a client is allowed to have is already hours old. There is
  no latency requirement at all for that traffic.
- The hard part is **filtering**: computing which client is entitled to know which event,
  at which coordinate time.

So bandwidth is low and correctness of the filter is everything. A bug that leaks an event
to a client before its light arrives is not a rendering glitch; it deletes the game.

## Authority

Server authoritative, without exception. Clients send intents:

```
Intent { ship_id, order, issued_at_client_t }
```

The server validates, assigns the event its coordinate, and persists it. `issued_at_client_t`
is advisory and is clamped: an intent may not be stamped earlier than the last event the
client has provably received, nor later than the server's current `t`.

Clients predict locally by running the same `lc_world::apply` over the events they have, so
a local order appears to take effect immediately and is reconciled when the server's version
of the event arrives. Because the client's ship is co-located with the player, the
round-trip is real network latency only — tens of milliseconds — and misprediction is rare.

## Filtering

Every outbound message passes one gate:

```
send(client, event) requires:
    exists an instrument the client owns, with worldline W, such that
    the event's light cone has reached W at or before the server's current t
```

Implemented as the `deliveries` table from [02-event-store.md](02-event-store.md): arrival
times are computed when the event is written, so the tick-time check is a B-tree range scan
on `(observer_id, arrive_t)`.

Make this one function, in one place, with tests that assert the negative case. Every other
send path calls it. Do not allow a second code path to emit to a socket.

## Transport

| target | primary | fallback |
|---|---|---|
| native desktop | QUIC | TCP |
| browser | WebTransport (HTTP/3) | WebSocket |

WebTransport gives the browser unreliable datagrams and multiple streams over QUIC, matching
what the native client gets, so one protocol implementation covers both. WebSocket is the
fallback where WebTransport is unavailable; it is reliable-ordered only, which is acceptable
because almost all of this game's traffic wants reliable delivery anyway.

Raw UDP is not available in the browser, so it is not an option for any tier. Do not design
a protocol that needs it.

Channels:

| channel | delivery | contents |
|---|---|---|
| control | reliable ordered | login, subscription, world parameters |
| events | reliable ordered | the delivery stream, the game's substance |
| local state | unreliable | the player's own ship, high rate, superseded immediately |
| bulk | reliable, out of band | system snapshots, baked shells, catalogue chunks |

## Tick model

The server advances coordinate time continuously at 8766x and processes in fixed steps.

```
real tick     = 50 ms  (20 Hz)
coordinate dt = 50 ms * 8766 = 438 s of in-game time
```

Per tick:

1. Advance `t`.
2. Drain intents, validate, write events, schedule deliveries.
3. Query `deliveries` for `arrive_t` in `(t_prev, t]`, per connected observer.
4. Push receptions.
5. Run scheduled world updates whose time has come: burns completing, construction
   finishing, probes arriving.

Step 1 is the only thing 438 s of coordinate time per tick affects, and it affects nothing,
because bodies are propagated analytically. The step size does not enter an integrator and
so does not accumulate error. A server under load that drops to 10 Hz produces identical
world state, only coarser event timestamps.

Ships under thrust are the exception: their arcs are analytic within a burn, but a burn's
*start* and *end* are events, and those must be placed at their exact coordinate time rather
than snapped to a tick. Schedule them; do not round them.

## Interest management

Three levels, coarsest first:

1. **Light cone.** The hard gate. Nothing passes that has not arrived.
2. **Detection threshold.** Arrival is not detection. A signal below a receiver's noise floor
   arrives and is not sent. This prunes far more than the cone does.
3. **Subscription.** A client asks for the systems and objects it is currently displaying.
   Purely a bandwidth optimisation, never a correctness mechanism.

Levels 1 and 2 are server-side and mandatory. Level 3 is a client hint and the server may
ignore it.

## Sharding

Start with one process. The design admits sharding later because the Oort shell is already a
hard boundary (see [03-world-model.md](03-world-model.md)).

| unit | owner |
|---|---|
| one system, inside its shell | one writer process |
| interstellar space | one writer process, or partitioned by an octree of the playable volume |
| the event store | shared Postgres, partitioned by `t` |

Cross-shard messages are shell crossings, and a shell crossing is already an event with a
coordinate. A shard that lags is not a consistency bug; it is a shard whose events arrive
late, and late arrival is the game's normal condition. This is the one respect in which the
premise makes the engineering easier rather than harder.

Event IDs must be locally generatable if shards exist: a snowflake layout of
`(shard_id, coordinate_time, sequence)` keeps them unique, time-ordered and allocation-free.

## Reconnection and catch-up

A client that disconnects for an hour missed 8766 in-game hours of deliveries. On reconnect:

1. Client sends the coordinate time of the last reception it has.
2. Server replays `deliveries` from that point, ordered by `arrive_t`, paginated.
3. Client folds them through `lc_world::apply` to rebuild its known-world snapshot.

The same path serves a brand-new client, from `t = 0` or from a compacted snapshot. Store a
periodic fold per observer so replay does not grow without bound.

## Security

| risk | mitigation |
|---|---|
| client requests events it cannot see | the single send gate; no second emit path |
| client fabricates observations | discoveries that gate rules are confirmed server-side |
| client claims an earlier intent time | clamp against the last provable reception |
| traffic analysis reveals unsent events | pad or batch; message size should not correlate with what was withheld |
| god view request from an unprivileged client | capability check server-side; disconnect on request |

The traffic-analysis item is not theoretical here. If a client can tell from packet timing
that *something* happened that it was not told about, it has learned faster-than-light
information. Batch deliveries on a fixed schedule rather than emitting them as they are
computed.

## Open

- WebTransport support and stability across browsers; if it is not ready, WebSocket-only for
  the browser and QUIC for native, with one protocol over two transports.
- Serialisation format. `postcard` is compact and `no_std`; `bincode` is simpler; both need
  an explicit versioning scheme since clients will lag server deploys.
- Whether the client ever runs `lc-store`. It should not — the client has no database — but
  the light-cone cursor's traversal logic is wanted on both sides, so it may need to split
  out of `lc-store` into `lc-spacetime`.
- Rate limiting and the cost of an intent, given that a player with a scripted client could
  emit thousands per second.
