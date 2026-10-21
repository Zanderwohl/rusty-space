-- Reports in the transcript, and what each sender called itself. See
-- lightcone/docs/23-factions.md §Transcripts.
--
-- `stars`: how many stars a 'report' row carried, so a sign-in can say so again.
-- `claim`: the name the sender claimed when it sent the row, so a sign-in can say where it
-- changed.
ALTER TABLE lc_messages
    ADD COLUMN stars integer,
    ADD COLUMN claim text,
    DROP CONSTRAINT lc_messages_content,
    ADD CONSTRAINT lc_messages_content
        CHECK (content IN ('text', 'ack', 'key', 'faction', 'report')
               AND (content = 'text') = (body IS NOT NULL)
               AND (content = 'faction') = (faction IS NOT NULL)
               AND (content = 'report') = (stars IS NOT NULL));
