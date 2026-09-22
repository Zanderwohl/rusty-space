# The event store

## What is an event

An event is a discrete, unpredictable change of world state, stamped with a server-frame
coordinate. Examples: a ship fires an engine, a structure completes, a transmitter emits a
pulse, a swarm element changes its orbit.

What is **not** an event:

- A planet's position at time `t`. Analytic from its elements.
- A star's brightness at time `t`. Analytic from its variability model and its occluders.
- A ship's position mid-coast. Analytic from its last burn event.

This distinction carries the entire storage budget. Continuous state is a function; only
the parameters of that function change, and those changes are the events. A world with
10 000 players producing one event per real second for a month is 2.6e10 events; a world
that stored positions per tick would produce that in a day.

## Why light-cone queries do not go against the event table

The obvious query — "which events lie on this observer's past light cone?" — has no useful
index. The predicate is `dt^2 = dr^2` with `dt > 0`: a three-dimensional surface in a
four-dimensional space, and its shape depends on the observer's own coordinate. No B-tree,
GiST or BRIN index answers it without scanning.

It is also the wrong question. An event's reachability is a property of **where its source
was**, and sources are structured:

- Sources number ~1e5 (stars, ships, transmitters, structures). Events number ~1e10.
- A source's events all share one worldline, so once the retarded time for that source is
  known, its reachable events are a contiguous range in `(source_id, t)`.

So the query decomposes:

```
1. spatial index over sources      -> candidate sources within max range
2. retarded_time() per candidate   -> t_r for each   (01-spacetime.md)
3. btree on (source_id, t)         -> that source's events in [t_r_prev, t_r]
```

Step 3 is a range scan on a composite B-tree. Steps 1 and 2 are over a set small enough to
hold in server memory as a BVH, with the database index as the durable copy.

## Schema

PostgreSQL 16+. Extensions: `cube` (contrib, for the 3D GiST index).

```sql
-- Every object with a worldline. Small table, fully cached.
CREATE TABLE sources (
    source_id    bigint PRIMARY KEY,
    kind         smallint NOT NULL,           -- star | ship | structure | transmitter
    system_id    bigint REFERENCES systems,   -- NULL for interstellar-only objects
    -- Global grid coordinates of the source's current position. Index only; the
    -- authoritative worldline lives in `worldlines`.
    gx           bigint NOT NULL,
    gy           bigint NOT NULL,
    gz           bigint NOT NULL,
    valid_from   bigint NOT NULL,             -- coordinate time, us
    valid_to     bigint                       -- NULL = still live
);

-- Lossy float8 cube for index pruning. Exact tests happen after, on the bigints.
CREATE INDEX sources_pos ON sources
    USING gist (cube(ARRAY[gx::float8, gy::float8, gz::float8]));

CREATE TABLE events (
    event_id   bigint  NOT NULL,
    source_id  bigint  NOT NULL,
    t          bigint  NOT NULL,              -- coordinate time, us
    gx         bigint  NOT NULL,              -- global grid position of the event
    gy         bigint  NOT NULL,
    gz         bigint  NOT NULL,
    system_id  bigint,                        -- NULL if outside any Oort shell
    lx         double precision,              -- local meters, when system_id is set
    ly         double precision,
    lz         double precision,
    lt         double precision,              -- local seconds from the system epoch
    kind       smallint NOT NULL,
    payload    jsonb    NOT NULL,
    PRIMARY KEY (event_id, t)
) PARTITION BY RANGE (t);

-- The only index the hot path needs.
CREATE INDEX events_source_t ON events (source_id, t);
-- Append-only in near-t order, so BRIN is cheap and effective for range sweeps.
CREATE INDEX events_t_brin ON events USING brin (t) WITH (pages_per_range = 32);
```

Partitions are monthly in coordinate time. At 8766x, one in-game month is 3.6 real hours,
so partition maintenance is frequent and must be automated (`pg_partman` or a cron job that
pre-creates a window ahead).

`lx/ly/lz/lt` carry the exact local-frame values. The global `gx/gy/gz/t` are the index.
Both are stored because recomputing one from the other loses the 150 m of the grid.

## Causality functions

```sql
-- Invariant interval squared. numeric, because bigint squares overflow int8 and
-- Postgres has no 128-bit integer. The 2^60 coordinate bound keeps this exact.
CREATE FUNCTION lc_interval2(
    t1 bigint, x1 bigint, y1 bigint, z1 bigint,
    t2 bigint, x2 bigint, y2 bigint, z2 bigint
) RETURNS numeric
LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$
    SELECT (t2-t1)::numeric^2
         - ((x2-x1)::numeric^2 + (y2-y1)::numeric^2 + (z2-z1)::numeric^2);
$$;

CREATE TYPE lc_separation AS ENUM ('timelike','lightlike','spacelike');

CREATE FUNCTION lc_classify(...) RETURNS lc_separation ...;

-- True when event 1 can have influenced event 2.
CREATE FUNCTION lc_precedes(...) RETURNS boolean
    AS $$ SELECT t2 >= t1 AND lc_interval2(...) >= 0 $$;
```

`lc_precedes` is frame-independent and safe to build rules on. There is deliberately no
`lc_simultaneous`: equal `t` is a `WHERE t1 = t2`, and dressing it up as a physics predicate
would invite code that treats a frame convention as a fact. Simultaneity of spacelike pairs
is not defined. See [01-spacetime.md](01-spacetime.md).

`numeric` exponentiation is slow (roughly 100x a float8 multiply). It is only for the exact
test after an index prune, never inside a scan.

## Delivery scheduling

For discrete signals with known receivers, do the work at write time instead of read time.

```sql
CREATE TABLE deliveries (
    observer_id bigint NOT NULL,
    arrive_t    bigint NOT NULL,   -- coordinate time the signal reaches the observer
    event_id    bigint NOT NULL,
    strength    real   NOT NULL,   -- for SNR gating at arrival
    PRIMARY KEY (observer_id, arrive_t, event_id)
) PARTITION BY RANGE (arrive_t);
```

The server tick then reads:

```sql
SELECT * FROM deliveries
 WHERE observer_id = $1 AND arrive_t > $2 AND arrive_t <= $3
 ORDER BY arrive_t;
```

A single B-tree range scan. This is the mechanism that makes the game's core loop O(log n)
instead of O(events).

Fan-out is finite because signal strength falls as `1/r^2` and every receiver has a noise
floor, so each emission has a maximum detection radius. That radius, not the world size,
bounds the number of `deliveries` rows per emission. A transmitter that raises its power
raises its fan-out cost, which is a fair place for a game-balance knob to live.

| phenomenon | strategy | why |
|---|---|---|
| radio pulse, tight beam | write-time delivery rows | receivers known, fan-out bounded |
| ship arrival, burn | write-time delivery rows | few observers care |
| starlight | read-time, analytic | continuous; a function, not an event stream |
| transit dimming | read-time, analytic | derived from the star's occluder list |
| swarm reconfiguration | event, then read-time | the event changes the function's parameters |
| mining a belt or cloud | event, then read-time | the same thing with the sign reversed: extraction lowers the population's count, so its transit probability falls |

## The light-cone cursor

The user-facing read API, in `lc-store`, over the same decomposition:

```rust
/// Streams what an observer can see, ordered by arrival time, cheapest first.
pub struct LightConeCursor { /* ... */ }

impl LightConeCursor {
    pub fn new(observer: &dyn Worldline, from_t: i64, to_t: i64) -> Self;
}

impl Iterator for LightConeCursor {
    type Item = Reception;   // (event, arrival coordinate, direction, strength)
}
```

Implementation: a binary heap of source-BVH nodes keyed by the **earliest possible** arrival
time from that node's bounding box — a lower bound, so popping in heap order yields
receptions in arrival order. Pop a node; if it is interior, push its children; if it is a
leaf, solve the retarded time exactly and push the source's event range.

Properties that matter:

- Ordered output without sorting the whole result.
- Early termination. `take_while(|r| r.arrival < deadline)` stops the traversal, and a UI
  that only draws the nearest 500 sources pays for 500.
- Pruning by strength as well as time, since the bounding box also bounds `1/r^2`.

### What the bounds can and cannot do

A node is dropped on three tests: its first light has not arrived by the end of the window, its
last light went past before the start of it, or the nearest it could be is still too far to clear
the strength floor. Each needs a bound that errs the safe way — the near distance must never come
out *larger* than the truth and the far distance never *smaller*, because a wrong bound drops a
reception rather than merely costing a visit. The observer contributes a bounding ball over the
window; the default is the one `c` alone gives, and a worldline that knows its own shape gives a
tighter one.

Two things about the heap key are worth knowing before trusting it.

**It orders by `earliest emission + nearest distance`, and that is only informative while the
window opens before the earliest possible arrival.** Streaming from the beginning and stopping
early — the case the property is claimed for — prunes hard: twenty receptions out of two
thousand open a fraction of the tree. A window that opens *after* the earliest possible arrival
flattens the bound onto its own start for nearly every node, because every node genuinely could
deliver at that instant. That is a limit of what a summary of a subtree can know.

**A source that emits continuously is a candidate for any window its own span reaches.** So a
tick does not read fewer *sources*; it reads a narrower range out of each, which is exactly the
`(source_id, t)` B-tree the decomposition ends on. Measured on a tick one per cent of the world's
span: the same sources, an eighth of the events. Time pruning at the source level pays for bursty
sources — a ship that burned once — and distance and strength pruning pay for the rest.

## Conversations are not a light-cone query

Transmissions are events and go in the tables above like everything else. What a player reads
back is a different question, and `sql/0005_chat.sql` is a second, much smaller set of tables
for it: `lc_messages`, `lc_message_receipts`, `lc_keyring`.

Two reasons, and the second is the load-bearing one.

**Retention runs the wrong way.** A partition is detached once its light cone has swept past
every observer, because nothing can still be *learning* of it. A conversation is the opposite:
it is read long after everything in it has arrived, by the two ships in it, whenever either of
them signs in. The tiering below would take the transcript away at exactly the moment it became
history.

**A light cone is not a conversation.** "Every event that reached this ship" is a question about
geometry. "Everything this ship has said and been told" is a question about who was talking, and
the second has to be answerable without walking the first.

Everything in those tables is stamped with the time the light **lands**, never the time it left,
and every read is gated on the clock having reached it. That is the same bargain the deliveries
table makes — work it out at write time, gate it at read time — in a place where it happens to
be the whole of a game mechanic rather than an optimization:

| row | written | readable |
|---|---|---|
| `lc_message_receipts` | when the message is transmitted | `arrive_t <= now` |
| `lc_keyring` | when the key offer is transmitted | `learned_t <= now`, which is what makes a key take four years to cross four light-years |

Writing them at transmission rather than at arrival is also what makes them work for a craft
nobody is flying. Arrivals are only walked for connected clients, because there is nobody to tell
otherwise — and a key that only landed when its owner happened to be signed in would be a
mechanic that depended on who was watching.

## Retention

The event table grows without bound and most of it is never read again. An event is
unreachable once its light cone has swept past every possible observer; with a bounded world
that is a fixed lag behind `t`, about the light-crossing time of the playable volume.

Three tiers:

| age | storage |
|---|---|
| within the playable light-crossing time | live partitions |
| older, still referenced by a delivery row | live partitions |
| older than both | detach partition, compress, archive to object storage |

Replays and the god view (see [07-rendering.md](07-rendering.md)) read archived partitions,
so archival must be reversible, not destructive.

### How it is done

`lc_detach_partition` takes a partition out of its table and records everything needed to put
it back; `lc_attach_partition` puts it back. Nothing is ever dropped. Shipping the detached
bytes to object storage is a deployment's job — it needs a bucket — so the crate records *where
they went* and stops there, because that is the part a restore needs to survive a restart.

The middle tier is the one a date comparison would get wrong. Light is in flight for years, so
an event far past the horizon can still be the thing a scheduled delivery is *about*, and taking
it away would leave a tick holding an identifier with nothing behind it. The check is a range
query on `event_id` rather than a join: **the identifier packs the coordinate second above the
shard and the sequence, so a time range is a contiguous identifier range** and a question about
*when* can be asked of a table that only stores *which*. Scoped to `arrive_t >= now`, partition
pruning leaves only the deliveries that have not happened yet.

One hazard worth knowing, because it is invisible until it bites: **listing partitions races
with detaching them.** `pg_get_expr` looks the object up again rather than reading the tuple in
hand, so a detach running alongside a listing yields a row whose bounds come back NULL rather
than a row that is absent. Filtering on `relpartbound IS NOT NULL` does not fix it; the filter
has to be on the result. A partition whose bound cannot be read is one on its way out, and is
not a live partition for any purpose.

## Open

- `cube` versus PostGIS 3D. `cube` is contrib and sufficient for bounding-box pruning;
  PostGIS brings real spatial operators and a much larger dependency. Decide once the
  source count is known.
**Decided: event IDs are `(shard, coordinate_time, sequence)`**, snowflake-style. Locally
generated, time-ordered, no coordination, and it works unchanged whether or not sharding ever
happens.

Still open:

- Whether the source BVH lives in Postgres at all, or is rebuilt in server memory at start
  with Postgres holding only the durable positions. Under discussion; not blocking, because
  the durable positions are needed either way.

## Building it

`lc-store` applies its own schema. The SQL is compiled into the binary rather than read from
disk, so a server carries the schema it expects and a test needs no working directory. Each step
runs once, inside a transaction, and is recorded in `lc_schema_steps`.

Migration takes a **session advisory lock** first. Without it two processes starting together
both find the step table missing, both create it, and one fails on a duplicate key in `pg_type`.
That is what two servers coming up at once looks like — and what three tests running in parallel
found before any server existed.

`lc_interval2` multiplies rather than raising to a power. `numeric ^ 2` is exact, but it goes
through `numeric_power`, which picks a display scale: a separation of one microsecond comes back
as `-1.0000000000000000`. Two scale-zero numerics multiplied are scale zero, so the result reads
as the integer it is, and it is the cheaper of the two besides.

### What the plan actually says

The delivery lookup is checked against the planner rather than asserted, because "a single
B-tree range scan" is a claim about what Postgres *chooses* to do and it is free to choose
otherwise. Two things had to be true before it did.

**The index has to cover the query.** `strength` was not in the primary key, so the planner had
to visit the heap for one float — and it does that with a bitmap scan, which reads the heap in
physical order, loses the index's ordering, and sorts the whole result before yielding the first
row. `PRIMARY KEY (observer_id, arrive_t, event_id) INCLUDE (strength)` removes the heap visit.

**Autovacuum has to keep up.** An index-only scan still checks the heap for every row until a
vacuum marks the pages all-visible, and until then the planner correctly prices it as no better
than a bitmap scan. Cost on twenty thousand rows: 362 before, 39 after. *The hot path is
index-only only while autovacuum keeps up* — a deployment requirement, not a schema one.

With both, the plan is one `Index Only Scan` over one partition, no sort and no heap access.
Partition pruning does the rest: a tick's window falls inside one thirty-day span, so the other
partitions are never opened.

A developer needs `createdb lc_store` and nothing else; `LC_STORE_URL` points elsewhere. Every
test that needs the database **skips** when it cannot reach one, because the suite has to pass on
a machine without Postgres and a test that cannot run is not a test that failed.
