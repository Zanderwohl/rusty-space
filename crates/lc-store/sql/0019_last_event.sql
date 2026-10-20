-- The highest event identifier each shard has written, read at boot to mint past it.
-- `max(event_id)` over `events` is planned against every partition and runs out of locks.
CREATE TABLE IF NOT EXISTS lc_last_event (
    shard    bigint PRIMARY KEY,
    event_id bigint NOT NULL
);

-- Seeded from the newest partition holding any, a partition at a time: it holds the highest second,
-- and a resuming shard skips that whole second. 14 and 1023 are `lc_store::id`'s sequence bits and
-- largest shard.
DO $$
DECLARE
    part    record;
    highest bigint;
BEGIN
    FOR part IN SELECT child FROM lc_partitions('events') ORDER BY from_t DESC LOOP
        EXECUTE format('SELECT max(event_id) FROM %I', part.child) INTO highest;
        EXIT WHEN highest IS NOT NULL;
    END LOOP;
    IF highest IS NOT NULL THEN
        INSERT INTO lc_last_event (shard, event_id)
        VALUES ((highest >> 14) & 1023, highest)
        ON CONFLICT (shard) DO NOTHING;
    END IF;
END $$;
