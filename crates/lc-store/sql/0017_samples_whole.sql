-- What `lc_samples` becomes in 0018.
CREATE TABLE IF NOT EXISTS lc_samples_whole (
    ship_id    bigint           NOT NULL,
    subject    bytea            NOT NULL,
    -- The witness's id as its bit pattern: the charting office is u64::MAX, which is -1 here.
    witness    bigint           NOT NULL,
    band       smallint         NOT NULL,
    -- Coordinate seconds, exactly as stamped: a sweep finishes a field at an instant that is not
    -- a whole microsecond, and a key that rounded it would not be the key.
    observed_s double precision NOT NULL,
    learned_t  bigint           NOT NULL,
    deficit    double precision NOT NULL,
    sigma      double precision NOT NULL,
    PRIMARY KEY (ship_id, subject, witness, band, observed_s, learned_t)
);
