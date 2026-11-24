-- An account need not have a name. A player names each ship they launch, on the game's shard;
-- the account itself is who signs in. Names already held are kept, and a Google sign-in still
-- records the one Google gives.
alter table accounts alter column display_name drop not null;
