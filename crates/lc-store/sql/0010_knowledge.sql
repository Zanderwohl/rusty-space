-- What each craft knows, and the photometric log its files leave out.
--
-- A craft's knowledge lives on the shard, not in its client, so that instruments keep running
-- and reports keep landing while nobody is signed in. This is where it outlives the process.
-- See lightcone/docs/24-standing-instruments.md.

-- One row per craft per subject: a star, a planet, a population, another craft.
--
-- The file is bounded — bearings are a reservoir of sixteen per witness — so it is rewritten
-- whole when it changes. Bytes and not columns because it is read and written whole and its
-- shape will keep changing for a while; `format` is what lets an old one be refused rather than
-- misread, exactly as `ships.format` does.
CREATE TABLE IF NOT EXISTS lc_knowledge (
    ship_id  bigint  NOT NULL,
    -- The subject, as the server encodes it. A key, never read by this crate.
    subject  bytea   NOT NULL,
    format   integer NOT NULL,
    file     bytea   NOT NULL,
    -- Coordinate microseconds it was last written at.
    saved_t  bigint  NOT NULL,
    PRIMARY KEY (ship_id, subject)
);

-- Photometric samples, which a file leaves out.
--
-- Unbounded while a star is watched, so appended rather than rewritten, and partitioned by when
-- the craft **learned** each sample rather than when it was measured: a relayed series can be
-- years old on arrival, and a partition keyed on measurement time would need to exist for a past
-- nobody made one for. Learned time is always about now, which is where partitions are kept ready.
-- Kept until processing consumes it into a conclusion (phase 11d).
CREATE TABLE IF NOT EXISTS lc_samples (
    ship_id    bigint           NOT NULL,
    subject    bytea            NOT NULL,
    -- The witness's id as its bit pattern: the charting office is u64::MAX, which is -1 here.
    witness    bigint           NOT NULL,
    band       smallint         NOT NULL,
    -- Coordinate seconds, exactly as the sample was stamped: a sweep finishes a field at an
    -- instant that is not a whole microsecond, and a key that rounded it would not be the key.
    observed_s double precision NOT NULL,
    -- Coordinate microseconds, for the partitions.
    learned_t   bigint           NOT NULL,
    deficit    double precision NOT NULL,
    sigma      double precision NOT NULL,
    PRIMARY KEY (ship_id, subject, witness, band, observed_s, learned_t)
) PARTITION BY RANGE (learned_t);
