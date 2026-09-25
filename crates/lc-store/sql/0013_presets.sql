-- Forms an account has kept under a name. See lightcone/docs/29-ship-form.md §Your own presets.
--
-- A checkpoint like `reading`: an answer as of a moment, never replayed and never pruned. Nothing
-- here is knowledge, so nothing is keyed by craft.

CREATE TABLE IF NOT EXISTS presets (
    account text NOT NULL,
    name    text NOT NULL,
    -- `lc_proto::Form` in postcard, the wire's own encoding, so its shape is pinned by the
    -- protocol's goldens. The store does not read it and the server validates it only when a
    -- preset is applied.
    form    bytea NOT NULL,
    PRIMARY KEY (account, name)
);
