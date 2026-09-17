-- A message need not be addressed to anybody.
--
-- The public channel broadcasts: something said to nobody in particular, which everyone in
-- range hears and anyone may answer. Until now every row here had an addressee, because every
-- message was a chat with somebody.
--
-- Nullable rather than a sentinel identifier. A zero would be a craft that does not exist, and
-- every query that joins or groups by this column would quietly treat all broadcasts as one
-- conversation with that phantom ship.
ALTER TABLE lc_messages ALTER COLUMN addressee DROP NOT NULL;
