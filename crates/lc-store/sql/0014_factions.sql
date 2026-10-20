-- Factions. See lightcone/docs/23-factions.md: a faction is a key and its chat, with no roster.

-- One row per key ever minted. The name is what the founder called it, and travels with the key.
CREATE TABLE IF NOT EXISTS lc_factions (
    faction   bigint PRIMARY KEY,
    founder   bigint NOT NULL,
    name      text   NOT NULL,
    -- The keys this one replaces, nearest first: its generation is one more than their count.
    lineage   bigint[] NOT NULL DEFAULT '{}',
    founded_t bigint NOT NULL
);

-- Who holds which faction key, from when its light landed. A schedule like lc_keyring: written
-- when the key is sent, read only once the clock has reached learned_t.
CREATE TABLE IF NOT EXISTS lc_faction_keys (
    holder    bigint NOT NULL,
    faction   bigint NOT NULL REFERENCES lc_factions,
    learned_t bigint NOT NULL,
    PRIMARY KEY (holder, faction)
);

-- `under`: the faction key a message is sealed to, which puts it in that faction's chat.
-- `faction`: the key a 'faction' message gives.
ALTER TABLE lc_messages
    ADD COLUMN under bigint,
    ADD COLUMN faction bigint,
    DROP CONSTRAINT lc_messages_content,
    ADD CONSTRAINT lc_messages_content
        CHECK (content IN ('text', 'ack', 'key', 'faction')
               AND (content = 'text') = (body IS NOT NULL)
               AND (content = 'faction') = (faction IS NOT NULL));
