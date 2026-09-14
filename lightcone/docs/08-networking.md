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

**In practice the floor is the client's delivery cursor, which is stronger.** A delivery is
matched against a cursor that advances every tick, so an event stamped *behind* that cursor
gets written, scheduled, and then never looked at again — the client's own transmission simply
vanishes. The last reception is always at or behind the cursor, so flooring at the cursor
implies the rule above and adds the part that keeps the delivery findable. It took several
ticks of accumulated cursor to show at all, which is why acting on the first tick never
revealed it.

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
on `(observer_id, arrive_t)`. The server reads it through exactly that scan, and a second
server started against the same database delivers what the first one scheduled — the tick loop
holds no events of its own.

Storage sits behind a `Journal` trait with two implementations. The filter's own tests run
against the in-memory one, because they are about causality and must not need a database to
say anything; the Postgres one has its own, which skip when there is none.

Make this one function, in one place, with tests that assert the negative case. Every other
send path calls it. Do not allow a second code path to emit to a socket.

**Built as a type, not a convention.** `Cleared<Sighting>` has a private field and one
constructor, `Cleared::clear`, which is the gate; `Outbound::Sightings` can hold nothing else.
"There is no second path to a socket" is then a fact the compiler enforces rather than a rule a
reviewer has to notice being broken, and the two `compile_fail` doctests on the type are what
say so — one for a struct literal, one for a destructuring pattern.

Two details the gate turns on:

- **The causality test comes first.** Below the noise floor and still in flight are both
  withheld, but a signal that is *both* is reported as in flight. The order matters because the
  noise floor is a detection rule that a feature could one day want to relax, and the light cone
  is not.
- **Arrival times round up, never down.** Rounding a solved arrival down would put it a
  microsecond before its true time and the gate would release it a microsecond early, which is
  the only error this whole mechanism exists to prevent.

## Transport

**Decided: WebSocket, for now, with one protocol behind a `Transport` trait.**

| target | built | later |
|---|---|---|
| native desktop | WebSocket | QUIC |
| browser | WebSocket | WebTransport (HTTP/3) |

WebTransport gives the browser unreliable datagrams and multiple streams over QUIC, matching
what the native client gets, so one protocol implementation covers both — and it is where this
should end up. It is not where it starts, for three reasons. It needs HTTP/3 and valid TLS in
development, where a self-signed certificate means a hash pin the browser expires in a
fortnight. Its browser support has to be checked rather than assumed. And the traffic that
would use its datagrams is the player's own ship state, which the client predicts locally and
does not need sent at all.

WebSocket reaches every target today with no ceremony, and it is reliable-ordered only, which
is acceptable because almost all of this game's traffic wants reliable delivery anyway. What it
costs is head-of-line blocking across the channels: one stream, so a bulk transfer stalls
events. Bulk is already out of band and belongs on its own connection when it exists.

The `Transport` trait is what makes this reversible, and a second implementation later will
prove the abstraction rather than fight it. The IO is async and the tick is not: reader and
writer tasks own the sockets and the tick only ever touches channel ends, so it never awaits on
a peer.

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

A client's cursor starts *before* everything rather than at the current time. "Told everything
up to now" would swallow an event stamped at exactly now — a ship's own act, on the tick it
acts — and the same start is what serves a brand-new client, which is the catch-up path run
from the beginning.

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

The tick also keeps the store's partitions made two spans ahead of itself. Doc 02 is explicit
that a store creating them on demand would hide a stalled maintenance job until the disk filled
— so the server does it on purpose, at a point where failing to is visible. The server *is* the
job that pre-creates the window.

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

**These are deferred by decision, to be settled when the features they belong to are designed
rather than in advance.** Each is local to one component and none of them constrains anything
else.

- ~~Transport~~. **Decided: WebSocket first**, behind a `Transport` trait, with WebTransport
  and QUIC to follow when the datagram channel has a consumer and the TLS story is settled.
- ~~Serialisation format~~. **Decided: `postcard`.** Both ends are Rust, including the WASM
  client, so a self-describing format buys nothing — and it would cost something: it lets a
  stale client half-understand a message, and a client misreading a sighting is not a degraded
  experience but a wrong one. A strict version handshake refusing the connection is the honest
  failure. The versioning scheme is that handshake plus golden bytes: `lc_proto::golden` pins
  what a version encodes to, and changing any message's shape without bumping
  `PROTOCOL_VERSION` fails a test. That is the only thing that can notice a moved field in a
  format that is not self-describing.
- Whether the client ever runs `lc-store`. It should not — the client has no database — but
  the light-cone cursor's traversal logic is wanted on both sides, so it may need to split
  out of `lc-store` into `lc-spacetime`.
- ~~Rate limiting~~. **Decided: a token bucket per connection, as a safety valve, with the
  number treated as provisional and measured.**

  Two separate things were being conflated. The *cost of an action* is a game rule — reaction
  mass for a burn, energy for a transmission, a cooldown on an instrument — and doc 02 already
  says where it lives: *a transmitter that raises its power raises its fan-out cost, which is a
  fair place for a game-balance knob to live.* That is phase 10's business and is not this.

  What this is, is a ceiling that stops a scripted client consuming the server, set far above
  any plausible legitimate rate. Two per second sustained, thirty at once. The sustained figure
  is not taste: doc 02 sizes the event table on *ten thousand players producing one event per
  real second*, so one per second is the rate the storage was designed for and this is double
  it.

  It is deliberately **not frame-coupled**. A burn is one intent that then runs for days of
  coordinate time; an order is one intent; the motion in between is analytic and predicted on
  the client. This is a strategy game's input rate, not a shooter's, and a per-frame allowance
  would be both far too generous and the wrong shape.

  Nobody has measured what a real player sends, because there is not yet a game to measure. So
  the server counts: `Server::usage` reports accepted, refused, and the peak any client reached
  in a tick and in a second. If real sessions never approach the ceiling, it can stop being
  provisional; if a legitimate client trips it, that is a bug report with the measurement
  already attached.

  A refusal is `Outbound::Throttled`, carrying how many ticks to wait. Not a disconnection: a
  client that hits it has a bug and should be told. It reveals nothing, being a fact about the
  client's own sending rather than about the world. The charge is taken *before* the message is
  read, so rubbish costs its sender as much as a valid intent, and connections that have never
  been given a ship are limited too.
