-- `lc_samples` as one table rather than partitioned by learned time.
--
-- What it holds is bounded by the room aboard each craft and leaves by key when a log is
-- consumed, never by age, so a partition per span gave retention nothing to take and cost a
-- table per span. See lightcone/docs/24-standing-instruments.md.
CREATE TABLE lc_samples_whole (
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

INSERT INTO lc_samples_whole
SELECT ship_id, subject, witness, band, observed_s, learned_t, deficit, sigma FROM lc_samples;

DROP TABLE lc_samples;
ALTER TABLE lc_samples_whole RENAME TO lc_samples;
ALTER INDEX lc_samples_whole_pkey RENAME TO lc_samples_pkey;
