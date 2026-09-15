-- Retention: taking old partitions out of the live table without destroying them.
--
-- An event is unreachable once its light cone has swept past every possible observer, which
-- with a bounded world is a fixed lag behind `t` -- the light-crossing time of the playable
-- volume. Past that it is still wanted: replays and the god view read it. So a partition is
-- *detached*, never dropped, and every detach is recorded well enough to undo.
--
-- See lightcone/docs/02-event-store.md.

-- What has been taken out of a live table, and everything needed to put it back.
CREATE TABLE IF NOT EXISTS lc_archived_partitions (
    child       text PRIMARY KEY,
    parent      text   NOT NULL,
    from_t      bigint NOT NULL,
    to_t        bigint NOT NULL,
    detached_at timestamptz NOT NULL DEFAULT now(),
    -- Where the bytes went, once something has shipped them. NULL means detached and still
    -- sitting in this database, which is the only state this crate can reach on its own:
    -- moving them to object storage is a deployment's job and needs a bucket.
    location    text
);

-- Every live partition of a table, with the bounds it was created with.
--
-- Read out of `relpartbound` rather than parsed from the name. The names are generated and
-- could be trusted, but the bound is what Postgres will actually route rows by, and a
-- disagreement between the two is exactly the kind of thing that only shows up as lost rows.
CREATE OR REPLACE FUNCTION lc_partitions(table_name text)
RETURNS TABLE (child text, from_t bigint, to_t bigint)
LANGUAGE sql STABLE AS $$
    SELECT * FROM (
        SELECT c.relname::text AS child,
               (regexp_match(pg_get_expr(c.relpartbound, c.oid),
                             'FROM [(]''?(-?[0-9]+)'))[1]::bigint AS from_t,
               (regexp_match(pg_get_expr(c.relpartbound, c.oid),
                             'TO [(]''?(-?[0-9]+)'))[1]::bigint AS to_t
          FROM pg_class c
          JOIN pg_inherits i ON c.oid = i.inhrelid
         WHERE i.inhparent = table_name::regclass
    ) bounded
    -- Anything whose bound could not be read is a partition on its way out of the table, and
    -- is not a live one for this purpose. The filter is on the *result* rather than on
    -- `relpartbound`: `pg_get_expr` looks the object up again and returns NULL if it has gone
    -- since the scan, so a detach running alongside this makes the columns NULL rather than
    -- making the row disappear.
     WHERE from_t IS NOT NULL AND to_t IS NOT NULL
     ORDER BY from_t;
$$;

-- Take a partition out of its table. The table it leaves behind still holds every row.
CREATE OR REPLACE FUNCTION lc_detach_partition(table_name text, child_name text)
RETURNS boolean LANGUAGE plpgsql AS $$
DECLARE
    lo bigint;
    hi bigint;
BEGIN
    SELECT from_t, to_t INTO lo, hi FROM lc_partitions(table_name) WHERE child = child_name;
    IF lo IS NULL THEN
        RETURN false;
    END IF;
    EXECUTE format('ALTER TABLE %I DETACH PARTITION %I', table_name, child_name);
    INSERT INTO lc_archived_partitions (child, parent, from_t, to_t)
         VALUES (child_name, table_name, lo, hi)
    ON CONFLICT (child) DO UPDATE SET detached_at = now(), location = NULL;
    RETURN true;
END $$;

-- Put one back, from the record made when it was detached.
--
-- The whole reason detach is not drop. A replay asks for a year nobody has looked at since it
-- happened, and the answer has to be to re-attach it rather than to explain that it is gone.
CREATE OR REPLACE FUNCTION lc_attach_partition(child_name text)
RETURNS boolean LANGUAGE plpgsql AS $$
DECLARE
    record lc_archived_partitions%ROWTYPE;
BEGIN
    SELECT * INTO record FROM lc_archived_partitions WHERE child = child_name;
    IF record.child IS NULL THEN
        RETURN false;
    END IF;
    EXECUTE format('ALTER TABLE %I ATTACH PARTITION %I FOR VALUES FROM (%s) TO (%s)',
                   record.parent, record.child, record.from_t, record.to_t);
    DELETE FROM lc_archived_partitions WHERE child = child_name;
    RETURN true;
END $$;

-- The event identifiers a coordinate-time range can contain.
--
-- The identifier packs the coordinate second above the shard and the sequence, so a time range
-- is a contiguous identifier range and a question about *when* an event happened can be asked
-- of a table that only stores *which* event it was. `lc-store`'s `id` module is the other half
-- of this; the two must agree about the field widths: 24 bits below the second, for a shard
-- and a sequence.
--
-- Clamped at zero because an identifier cannot carry a negative second -- `EventId::new`
-- refuses one -- so no event before the world origin has one to be found by.
CREATE OR REPLACE FUNCTION lc_id_range(from_t bigint, to_t bigint)
RETURNS TABLE (lo bigint, hi bigint)
LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$
    SELECT greatest(from_t / 1000000, 0) * 16777216,
           (greatest(to_t / 1000000, 0) + 1) * 16777216 - 1;
$$;

-- Whether any delivery still to be read refers to an event in this range.
--
-- The second retention tier. Light can be in flight for years, so an event long past the
-- horizon can still be the thing a delivery row is about; archiving it would leave a tick
-- holding an identifier with nothing behind it. Scoped to `arrive_t >= now` so partition
-- pruning leaves only the deliveries that have not happened yet, which is a small set.
CREATE OR REPLACE FUNCTION lc_has_pending_deliveries(from_t bigint, to_t bigint, now_t bigint)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        SELECT 1 FROM deliveries d, lc_id_range(from_t, to_t) r
         WHERE d.arrive_t >= now_t AND d.event_id BETWEEN r.lo AND r.hi
    );
$$;
