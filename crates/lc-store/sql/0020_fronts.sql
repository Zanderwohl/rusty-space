-- Events whose light the shard is still following. Not partitioned, because rows leave by
-- `until_t`, not emission time. The event is copied whole so a delivery found decades on can be
-- read after its partition is archived. See lightcone/docs/02-event-store.md.
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
    -- Opaque here; the shard's.
    spread    jsonb    NOT NULL
);

CREATE INDEX IF NOT EXISTS lc_fronts_until ON lc_fronts (until_t);
