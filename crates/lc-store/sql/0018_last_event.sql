-- The highest event identifier each shard has written, kept beside the events rather than found
-- among them.
--
-- A shard reads it once at boot to mint past it. `max(event_id)` over `events` is planned against
-- every partition, and with thousands of them it ran out of lock space before it returned.
CREATE TABLE IF NOT EXISTS lc_last_event (
    shard    bigint PRIMARY KEY,
    event_id bigint NOT NULL
);

-- What is there already, under the shard that wrote the highest. 14 and 1023 are
-- `lc_store::id`'s sequence bits and largest shard.
INSERT INTO lc_last_event (shard, event_id)
SELECT (m >> 14) & 1023, m FROM (SELECT max(event_id) AS m FROM events) highest
 WHERE m IS NOT NULL
ON CONFLICT (shard) DO NOTHING;
