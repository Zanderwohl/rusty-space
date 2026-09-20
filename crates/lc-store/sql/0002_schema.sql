-- The tables.
--
-- Continuous state is a function; only the parameters change, and those changes are the events.
-- See lightcone/docs/02-event-store.md for why the light-cone query does not go against the
-- event table, and what it goes against instead.

CREATE EXTENSION IF NOT EXISTS cube;

-- Where a system's origin sits on the global grid. Small, and fully cached.
CREATE TABLE IF NOT EXISTS systems (
    system_id bigint PRIMARY KEY,
    name      text   NOT NULL,
    gx        bigint NOT NULL,
    gy        bigint NOT NULL,
    gz        bigint NOT NULL
);

-- Every object with a worldline.
--
-- Sources number about 1e5 and events about 1e10, which is the whole reason the light-cone
-- query decomposes through this table: a source's events all share one worldline, so once the
-- retarded time for the source is known its reachable events are a contiguous range in
-- (source_id, t).
CREATE TABLE IF NOT EXISTS sources (
    source_id  bigint PRIMARY KEY,
    kind       smallint NOT NULL,
    system_id  bigint REFERENCES systems,
    -- Current position on the global grid. An index, not the truth: the authoritative
    -- worldline is reconstructed from events.
    gx         bigint NOT NULL,
    gy         bigint NOT NULL,
    gz         bigint NOT NULL,
    valid_from bigint NOT NULL,
    valid_to   bigint
);

-- Lossy float8 cube, for pruning only. Every exact test happens afterwards on the bigints:
-- a float8 cannot hold 2^60 microseconds, and the 150 meters the grid resolves would be lost.
CREATE INDEX IF NOT EXISTS sources_pos ON sources
    USING gist (cube(ARRAY[gx::float8, gy::float8, gz::float8]));

CREATE TABLE IF NOT EXISTS events (
    event_id  bigint NOT NULL,
    source_id bigint NOT NULL,
    t         bigint NOT NULL,
    gx        bigint NOT NULL,
    gy        bigint NOT NULL,
    gz        bigint NOT NULL,
    system_id bigint,
    -- The exact local-frame values, when the event happened inside a system. Both these and
    -- the global grid are stored because deriving one from the other loses the grid's 150 m.
    lx        double precision,
    ly        double precision,
    lz        double precision,
    lt        double precision,
    kind      smallint NOT NULL,
    payload   jsonb    NOT NULL,
    PRIMARY KEY (event_id, t)
) PARTITION BY RANGE (t);

CREATE INDEX IF NOT EXISTS events_source_t ON events (source_id, t);
-- Append-only in near-t order, so a BRIN summary is small and effective for range sweeps.
CREATE INDEX IF NOT EXISTS events_t_brin ON events USING brin (t) WITH (pages_per_range = 32);

-- What a given observer will receive, and when.
--
-- Written when the event is written rather than worked out when it is read. This is the
-- mechanism that makes the core loop a B-tree range scan instead of a pass over the events.
CREATE TABLE IF NOT EXISTS deliveries (
    observer_id bigint NOT NULL,
    arrive_t    bigint NOT NULL,
    event_id    bigint NOT NULL,
    strength    real   NOT NULL,
    -- `INCLUDE (strength)`, so the index carries everything the tick asks for and the read is
    -- an index-only scan. Without it the planner has to visit the heap for one float, and it
    -- does that with a bitmap scan -- which reads the heap in physical order, loses the index's
    -- ordering, and has to sort the whole result before it can yield the first row. Ordered
    -- output without sorting is the property the light-cone cursor is built on.
    PRIMARY KEY (observer_id, arrive_t, event_id) INCLUDE (strength)
) PARTITION BY RANGE (arrive_t);

-- Partition width, microseconds of coordinate time: thirty days.
--
-- A fixed span rather than a calendar month, so which partition an event belongs to is a
-- division and not a date calculation. Coordinate time has no calendar; naming these after
-- months would be naming them after a convention the world does not have.
CREATE OR REPLACE FUNCTION lc_partition_span() RETURNS bigint
LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$ SELECT 2592000000000::bigint $$;

-- Which partition a coordinate time falls in. Floor division, so it is right below zero too.
CREATE OR REPLACE FUNCTION lc_partition_of(t bigint) RETURNS bigint
LANGUAGE sql IMMUTABLE PARALLEL SAFE STRICT AS $$
    SELECT CASE WHEN t >= 0 THEN t / lc_partition_span()
                ELSE ((t + 1) / lc_partition_span()) - 1 END;
$$;

-- Create whatever partitions of `table_name` the range [from_t, to_t] needs, and say how many.
--
-- Idempotent, and safe to call from several processes: the create is guarded and a loser sees
-- `duplicate_table`. At the design rate one span is four and a half real days, so this runs on
-- a schedule forever rather than once at install.
CREATE OR REPLACE FUNCTION lc_ensure_partitions(table_name text, from_t bigint, to_t bigint)
RETURNS integer LANGUAGE plpgsql AS $$
DECLARE
    span   bigint := lc_partition_span();
    first  bigint := lc_partition_of(from_t);
    last   bigint := lc_partition_of(to_t);
    made   integer := 0;
    slot   bigint;
    child  text;
BEGIN
    IF last < first THEN
        RAISE EXCEPTION 'lc_ensure_partitions: % is before %', to_t, from_t;
    END IF;
    FOR slot IN first..last LOOP
        child := format('%s_p%s', table_name, replace(slot::text, '-', 'm'));
        BEGIN
            EXECUTE format(
                'CREATE TABLE %I PARTITION OF %I FOR VALUES FROM (%s) TO (%s)',
                child, table_name, slot * span, (slot + 1) * span);
            made := made + 1;
        EXCEPTION
            WHEN duplicate_table THEN NULL;
        END;
    END LOOP;
    RETURN made;
END $$;
