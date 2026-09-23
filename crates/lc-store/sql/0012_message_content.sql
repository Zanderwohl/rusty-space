-- What a message is, stated rather than read off an empty body. An acknowledgment and a key
-- offer were both stored as '', and every reader had to tell them from text by remembering to.
ALTER TABLE lc_messages ADD COLUMN content text;
UPDATE lc_messages
   SET content = CASE WHEN is_key THEN 'key' WHEN body = '' THEN 'ack' ELSE 'text' END;
ALTER TABLE lc_messages ALTER COLUMN body DROP NOT NULL;
UPDATE lc_messages SET body = NULL WHERE content <> 'text';
ALTER TABLE lc_messages
    ALTER COLUMN content SET NOT NULL,
    ADD CONSTRAINT lc_messages_content
        CHECK (content IN ('text', 'ack', 'key') AND (content = 'text') = (body IS NOT NULL)),
    DROP COLUMN is_key;
