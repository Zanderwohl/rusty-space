-- Which message a transmission is, across however many times it was sent.
--
-- A resend is a second pulse of light and a second event: it really happened, at its own
-- coordinate, and both rows stay. What it is not is a second thing somebody said, so a reader
-- collapses rows sharing this into one line.
--
-- Not a sequence number. The two ends do not agree about how many messages exist, because half
-- of them are in flight, so a counter would need reconciling against something that is not
-- there. Unique to the sender is all this has to be.
--
-- Nullable rather than defaulted, because a row written before this column existed has no key
-- and inventing a zero for it would make every one of them the same message.
ALTER TABLE lc_messages ADD COLUMN IF NOT EXISTS idem bigint;

-- A sender's own resends, which is the only lookup this column has.
CREATE INDEX IF NOT EXISTS lc_messages_idem ON lc_messages (sender, idem) WHERE idem IS NOT NULL;
