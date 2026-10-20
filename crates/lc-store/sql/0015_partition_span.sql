-- Partitions 360 days wide rather than thirty.
--
-- At the design rate thirty days of coordinate time pass in five real minutes, so the tick made
-- a partition of every partitioned table every five minutes, and a month of running left eight
-- thousand of each: enough that `pg_dump`, and any query planned against all of them, ran out of
-- lock space. A tick's window is minutes of coordinate time inside one partition and retention
-- takes them away whole, so neither wants them narrow.
--
-- 360 days rather than a Julian year because it is twelve of the old spans exactly: a slot holds
-- old partitions whole or not at all.
CREATE OR REPLACE FUNCTION lc_partition_span() RETURNS bigint
LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$ SELECT 31104000000000::bigint $$;

CREATE OR REPLACE FUNCTION lc_child_name(table_name text, kind text, slot bigint) RETURNS text
LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$
    SELECT format('%s_%s%s', table_name, kind, replace(slot::text, '-', 'm'));
$$;

-- One partition, unless it is already there. True when it was made.
CREATE OR REPLACE FUNCTION lc_make_partition(table_name text, child text, from_t bigint, to_t bigint)
RETURNS boolean LANGUAGE plpgsql AS $$
BEGIN
    EXECUTE format('CREATE TABLE %I PARTITION OF %I FOR VALUES FROM (%s) TO (%s)',
                   child, table_name, from_t, to_t);
    RETURN true;
EXCEPTION
    WHEN duplicate_table THEN RETURN false;
END $$;

-- Create whatever partitions of `table_name` the range [from_t, to_t] needs, and say how many.
--
-- A slot that already holds one of the old thirty-day partitions, named `_p`, is filled out at
-- that width, because a whole one would overlap it. Retention takes the old ones away in time,
-- after which every slot is made whole and named `_s`.
CREATE OR REPLACE FUNCTION lc_ensure_partitions(table_name text, from_t bigint, to_t bigint)
RETURNS integer LANGUAGE plpgsql AS $$
DECLARE
    span   bigint := lc_partition_span();
    legacy bigint := 2592000000000;
    per    bigint := lc_partition_span() / 2592000000000;
    first  bigint := lc_partition_of(from_t);
    last   bigint := lc_partition_of(to_t);
    made   integer := 0;
    slot   bigint;
    piece  bigint;
BEGIN
    IF last < first THEN
        RAISE EXCEPTION 'lc_ensure_partitions: % is before %', to_t, from_t;
    END IF;
    FOR slot IN first..last LOOP
        IF EXISTS (
            SELECT 1 FROM pg_inherits i JOIN pg_class c ON c.oid = i.inhrelid
             WHERE i.inhparent = table_name::regclass
               AND c.relname IN (SELECT lc_child_name(table_name, 'p', p)
                                   FROM generate_series(slot * per, slot * per + per - 1) p)
        ) THEN
            FOR piece IN slot * per .. slot * per + per - 1 LOOP
                IF lc_make_partition(table_name, lc_child_name(table_name, 'p', piece),
                                     piece * legacy, (piece + 1) * legacy) THEN
                    made := made + 1;
                END IF;
            END LOOP;
        ELSIF lc_make_partition(table_name, lc_child_name(table_name, 's', slot),
                                slot * span, (slot + 1) * span) THEN
            made := made + 1;
        END IF;
    END LOOP;
    RETURN made;
END $$;
