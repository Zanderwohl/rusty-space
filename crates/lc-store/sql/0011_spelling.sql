-- American spelling, which the rest of the schema and code use. The steps that named these
-- columns have shipped and are left exactly as they ran.
ALTER TABLE lc_keyring RENAME COLUMN learned_t TO learned_t;
ALTER TABLE lc_samples RENAME COLUMN learned_t TO learned_t;
