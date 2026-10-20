-- Events whose light the shard is still following, each until it is too faint for anything to
-- detect. See lightcone/docs/02-event-store.md, "Light fronts".
--
-- Not partitioned: what leaves is decided per row by `until_t`, which its brightness fixed, and
-- not by when it was emitted. The event is copied whole, so a delivery found decades on still has
-- something to read after its partition has been archived.
CREATE TABLE IF NOT EXISTS lc_fronts (
    event_id  bigint   PRIMARY KEY,
    source_id bigint   NOT NULL,
    t         bigint   NOT NULL,
    gx        bigint   NOT NULL,
    gy        bigint   NOT NULL,
    gz        bigint   NOT NULL,
    kind      smallint NOT NULL,
    payload   jsonb    NOT NULL,
    -- Coordinate microseconds after which its light is fainter than anything can detect.
    until_t   bigint   NOT NULL,
    -- How it spreads and who hears it, which the shard reads and this crate does not.
    spread    jsonb    NOT NULL
);

CREATE INDEX IF NOT EXISTS lc_fronts_until ON lc_fronts (until_t);
