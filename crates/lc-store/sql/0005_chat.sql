-- Conversations: what was said, what landed, and who holds whose key.
--
-- The events table already holds every transmission, so it is worth saying why this is not a
-- view over it. Two reasons, and the second is the load-bearing one.
--
-- An event partition is *detached* once its light cone has swept past every observer, because
-- nothing can still be learning of it. A conversation is the opposite: it is read long after
-- everything in it has arrived, by the two ships that are party to it, whenever either of them
-- signs in. Retention would take the transcript away exactly when it became history.
--
-- And a light cone is not a conversation. "Every event that reached this ship" is a question
-- about geometry; "everything this ship has said and been told" is a question about who was
-- talking, and the second one has to be answerable without walking the first.
--
-- See lightcone/docs/05-observation.md for what a transmission is, and 02-event-store.md for
-- why the events beside these are the ones that get archived.

-- One transmission, whole. The body here is never redacted; redaction happens on the way out
-- to a receiver, which is the server's job and not this table's.
CREATE TABLE IF NOT EXISTS lc_messages (
    event_id  bigint PRIMARY KEY,
    sender    bigint NOT NULL,
    -- Who it was addressed to. Never null: a chat is with somebody, even when it is shouted in
    -- every direction and half a system hears it.
    addressee bigint NOT NULL,
    sealed    boolean NOT NULL,
    -- True for a key offer, which is a message with nothing in it and belongs in the same
    -- transcript because that is where a player looks for it.
    is_key    boolean NOT NULL DEFAULT false,
    body      text   NOT NULL,
    -- Event ids of the addressee's messages the sender had received when this went out.
    acks      bigint[] NOT NULL DEFAULT '{}',
    sent_t    bigint NOT NULL
);

-- A ship's own half of every conversation it started.
CREATE INDEX IF NOT EXISTS lc_messages_sender ON lc_messages (sender, sent_t);

-- Where a transmission actually landed, and when.
--
-- Not the same set as the addressees: an open message is read by everyone in earshot, and a
-- sealed one is *heard* by them too. Both are rows here, because what a ship has a record of is
-- what reached it, not what was meant for it.
CREATE TABLE IF NOT EXISTS lc_message_receipts (
    event_id bigint NOT NULL REFERENCES lc_messages ON DELETE CASCADE,
    observer bigint NOT NULL,
    arrive_t bigint NOT NULL,
    PRIMARY KEY (observer, event_id)
);

CREATE INDEX IF NOT EXISTS lc_receipts_arrival ON lc_message_receipts (observer, arrive_t);

-- Who holds whose public key.
--
-- A game mechanic wearing cryptography's clothes: there are no keys, only the fact of having
-- been told one. A row appears when a key offer's **light arrives**, never when it is sent, so
-- across four light-years the sender waits four years to be able to be answered in private.
CREATE TABLE IF NOT EXISTS lc_keyring (
    holder   bigint NOT NULL,
    subject  bigint NOT NULL,
    -- Coordinate microseconds the offer landed. Kept because "since when" is a fair question
    -- about a key and the answer is not derivable from anything else here.
    learnt_t bigint NOT NULL,
    PRIMARY KEY (holder, subject)
);
