-- What an account may do beyond playing.
--
-- An integer rather than a flag, because it will grow into levels or groups; for now 0 is a
-- player and 1 is an admin, who may issue development actions on a shard. The broker only
-- carries it into the game ticket. What a level allows is each game server's to decide.

alter table accounts add column permission integer not null default 0;

-- The first admin. By the address on any of the account's links, verified or not: at the time
-- of writing it is a password account, whose addresses are never verified, and this runs once
-- against data that exists rather than being a rule about accounts created later.
update accounts set permission = 1
where id in (select account_id from links where email = 'zander@zanderlowry.com');
