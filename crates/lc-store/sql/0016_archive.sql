-- Detaching a partition folds its rows into one archive table per parent and drops it: replays
-- need the rows, and a table left standing per detach is a table an hour for `pg_dump` to lock.
CREATE OR REPLACE FUNCTION lc_detach_partition(table_name text, child_name text)
RETURNS boolean LANGUAGE plpgsql AS $$
DECLARE
    lo bigint;
    hi bigint;
    archive text := table_name || '_archive';
BEGIN
    SELECT from_t, to_t INTO lo, hi FROM lc_partitions(table_name) WHERE child = child_name;
    IF lo IS NULL THEN
        RETURN false;
    END IF;
    EXECUTE format('CREATE TABLE IF NOT EXISTS %I (LIKE %I)', archive, table_name);
    EXECUTE format('INSERT INTO %I SELECT * FROM %I', archive, child_name);
    EXECUTE format('DROP TABLE %I', child_name);
    INSERT INTO lc_archived_partitions (child, parent, from_t, to_t)
         VALUES (child_name, table_name, lo, hi)
    ON CONFLICT (child) DO UPDATE SET detached_at = now(), location = NULL;
    RETURN true;
END $$;

-- Put one back from the archive. A table still standing under the name was detached by 0003 and is
-- attached as it is.
CREATE OR REPLACE FUNCTION lc_attach_partition(child_name text)
RETURNS boolean LANGUAGE plpgsql AS $$
DECLARE
    record lc_archived_partitions%ROWTYPE;
    archive text;
    key text;
BEGIN
    SELECT * INTO record FROM lc_archived_partitions WHERE child = child_name;
    IF record.child IS NULL THEN
        RETURN false;
    END IF;
    IF to_regclass(child_name) IS NULL THEN
        EXECUTE format('CREATE TABLE %I PARTITION OF %I FOR VALUES FROM (%s) TO (%s)',
                       record.child, record.parent, record.from_t, record.to_t);
    ELSIF NOT EXISTS (SELECT 1 FROM pg_inherits WHERE inhrelid = to_regclass(child_name)) THEN
        EXECUTE format('ALTER TABLE %I ATTACH PARTITION %I FOR VALUES FROM (%s) TO (%s)',
                       record.parent, record.child, record.from_t, record.to_t);
    END IF;
    archive := record.parent || '_archive';
    IF to_regclass(archive) IS NOT NULL THEN
        key := (regexp_match(pg_get_partkeydef(record.parent::regclass), '[(](\w+)[)]'))[1];
        EXECUTE format(
            'WITH moved AS (DELETE FROM %I WHERE %I >= %s AND %I < %s RETURNING *)
             INSERT INTO %I SELECT * FROM moved',
            archive, key, record.from_t, key, record.to_t, record.parent);
    END IF;
    DELETE FROM lc_archived_partitions WHERE child = child_name;
    RETURN true;
END $$;

-- Drop one partition outright. For deliveries, which the events they are about make redundant.
CREATE OR REPLACE FUNCTION lc_drop_partition(table_name text, child_name text)
RETURNS boolean LANGUAGE plpgsql AS $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_inherits
                    WHERE inhparent = table_name::regclass
                      AND inhrelid = to_regclass(child_name)) THEN
        RETURN false;
    END IF;
    EXECUTE format('DROP TABLE %I', child_name);
    RETURN true;
END $$;

-- Whether any delivery still kept refers to an event in this range. Retention passes its cutoff as
-- `now_t`: a delivery kept for a client catching up needs its event as much as one in flight.
CREATE OR REPLACE FUNCTION lc_has_pending_deliveries(from_t bigint, to_t bigint, now_t bigint)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        SELECT 1 FROM deliveries d, lc_id_range(from_t, to_t) r
         WHERE d.arrive_t >= now_t AND d.event_id BETWEEN r.lo AND r.hi
    );
$$;
