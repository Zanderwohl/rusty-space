-- Craft, and the shard clock they are stamped against.
--
-- The rest of this schema holds *events*, from which a worldline can in principle be
-- reconstructed. This table is not that: it is a checkpoint, written so a shard that restarts
-- does not have to replay the history of the world to find out where anything is. The events
-- remain the record of what happened; this is the answer as of a moment.

CREATE TABLE IF NOT EXISTS ships (
    ship_id  bigint PRIMARY KEY,
    -- The account it belongs to, so signing in finds it again. Null for a craft with no pilot:
    -- a probe, a relay, or an anonymous connection's ship, which has no account to come back to.
    account  text UNIQUE,
    -- Coordinate microseconds `state` was taken at.
    --
    -- Load-bearing, not provenance. Every motive is stamped with absolute coordinate times, so
    -- a station or a crossing restored at any later time is still correct -- but a ballistic arc
    -- is re-solved from the position and velocity beside it, and reading a state taken an hour
    -- ago as though it were the present solves the wrong orbit. Restoring reads at this time and
    -- then advances to now.
    saved_t  bigint NOT NULL,
    -- The craft, as bytes. This crate has no opinion about what a motive is; the server does.
    --
    -- **Bytes and not JSON, and the reason is exactness.** `serde_json` does not round-trip
    -- every f64: -1.8149592025296526e-22 comes back as -1.8149592025296529e-22, one place out.
    -- The whole model rests on two machines folding the same numbers and agreeing to the bit,
    -- so a format that quietly perturbs a coordinate every restart is the wrong one however
    -- readable it is. Postcard writes an f64 as its eight bytes.
    state    bytea NOT NULL,
    -- Which encoding wrote `state`, and a contract rather than a note: a positional format
    -- cannot notice that it is reading an older shape, so a row whose format is not the current
    -- one is refused instead of misread.
    format   integer NOT NULL
);

-- One row per shard.
--
-- The world's clock has to outlive the process. Every motive carries an absolute coordinate
-- time, so a shard that restarted at zero would read every saved ship as one whose crossing has
-- not begun -- and then fly them all again, from the start, decades of world time in the past.
CREATE TABLE IF NOT EXISTS shard_state (
    shard_id  bigint PRIMARY KEY,
    now_t     bigint NOT NULL,
    -- The next identifier to hand out, so a restart does not reissue one.
    next_ship bigint NOT NULL
);
