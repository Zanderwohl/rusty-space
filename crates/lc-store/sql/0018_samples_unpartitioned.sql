-- `lc_samples` as one table: its rows leave by key when consumed, never by age, so time partitions
-- gave retention nothing to take. `lc_store::migrate` has already moved and dropped each partition,
-- a transaction each, so this step locks none of them.
INSERT INTO lc_samples_whole
SELECT ship_id, subject, witness, band, observed_s, learned_t, deficit, sigma FROM lc_samples
ON CONFLICT DO NOTHING;

DROP TABLE lc_samples;
ALTER TABLE lc_samples_whole RENAME TO lc_samples;
ALTER INDEX lc_samples_whole_pkey RENAME TO lc_samples_pkey;
