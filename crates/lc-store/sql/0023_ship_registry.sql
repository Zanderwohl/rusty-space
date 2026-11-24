-- Every ship ever launched, and who launched it.
--
-- `ships` is a checkpoint: a wreck's row there loses its account at the collapse and is deleted
-- once its light has faded. This table keeps the record an administrator reads after both: which
-- account flew the ship, what it was called, and when it ended. Nothing restores from it.
CREATE TABLE IF NOT EXISTS ship_registry (
    ship_id     bigint      PRIMARY KEY,
    -- The account that launched it. Null for a craft that never had a pilot.
    account     text,
    name        text,
    launched_at timestamptz NOT NULL DEFAULT now(),
    -- Coordinate microseconds of its collapse. Null while it flies.
    ended_t     bigint,
    ended_at    timestamptz
);

CREATE INDEX IF NOT EXISTS ship_registry_by_account ON ship_registry (account, ship_id DESC);

-- The ships already saved. Their names are inside `state`, which only the server reads; the
-- shard writes them on its next boot.
INSERT INTO ship_registry (ship_id, account)
SELECT ship_id, account FROM ships WHERE account IS NOT NULL
ON CONFLICT DO NOTHING;
