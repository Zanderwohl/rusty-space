-- `lc_samples` as one table rather than partitioned by learned time.
--
-- What it holds is bounded by the room aboard each craft and leaves by key when a log is
-- consumed, never by age, so a partition per span gave retention nothing to take and cost a
-- table per span. See lightcone/docs/24-standing-instruments.md.
--
-- `lc_store::migrate` has already moved each partition's rows into `lc_samples_whole` and dropped
-- it, a transaction each: doing it here, in this step's one transaction, would hold a lock on
-- every partition at once, and a store with thousands has not got that many.
INSERT INTO lc_samples_whole
SELECT ship_id, subject, witness, band, observed_s, learned_t, deficit, sigma FROM lc_samples
ON CONFLICT DO NOTHING;

DROP TABLE lc_samples;
ALTER TABLE lc_samples_whole RENAME TO lc_samples;
ALTER INDEX lc_samples_whole_pkey RENAME TO lc_samples_pkey;
