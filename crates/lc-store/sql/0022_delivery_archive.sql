-- Deliveries are archived rather than dropped, so a client back after longer than the retention
-- window is still told what landed on it while it was away (`lc-catchup`). The direction it came
-- from is kept with each, because a reader with no worldline cannot work it out again. Null on a
-- row written before this.
ALTER TABLE deliveries ADD COLUMN IF NOT EXISTS dx real;
ALTER TABLE deliveries ADD COLUMN IF NOT EXISTS dy real;
ALTER TABLE deliveries ADD COLUMN IF NOT EXISTS dz real;

-- Made here rather than at the first detach, so each has the index a catch-up reads by: `LIKE`
-- copies no indexes, and an archive scanned whole per resume is the archive's whole size per resume.
CREATE TABLE IF NOT EXISTS deliveries_archive (LIKE deliveries);
ALTER TABLE deliveries_archive ADD COLUMN IF NOT EXISTS dx real;
ALTER TABLE deliveries_archive ADD COLUMN IF NOT EXISTS dy real;
ALTER TABLE deliveries_archive ADD COLUMN IF NOT EXISTS dz real;
CREATE INDEX IF NOT EXISTS deliveries_archive_landing ON deliveries_archive (observer_id, arrive_t, event_id);

CREATE TABLE IF NOT EXISTS events_archive (LIKE events);
CREATE INDEX IF NOT EXISTS events_archive_id ON events_archive (event_id);
