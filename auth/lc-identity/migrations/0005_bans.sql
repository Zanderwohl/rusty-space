-- Bans, and the record of who did what to whom.
--
-- A ban is a **row, not a column**, because they are served concurrently: three weeks for one
-- thing and two months for another are two facts about an account, both true, and collapsing
-- them into one `banned_until` loses the reason the longer one was issued. What a person is
-- told is the furthest expiry among the ones in force; what an administrator reads is the
-- list.
create table bans (
    id          uuid        primary key,
    account_id  uuid        not null references accounts (id) on delete cascade,
    -- One of a closed set, stored as its name. `lc_identity::bans::Reason` is the set; a name
    -- rather than an integer so a dump is readable and so retiring a variant does not
    -- renumber the rest.
    reason      text        not null,
    -- **Never shown to the banned account.** The case notes: what was seen, which report it
    -- came from, what was decided. Everything the public reason deliberately does not say.
    notes       text        not null default '',
    issued_by   uuid        references accounts (id) on delete set null,
    issued_at   timestamptz not null default now(),
    -- Null is no end. A finite ban is the normal case and the column is nullable rather than
    -- carrying a sentinel far future, so "is this permanent" is a null check and not a
    -- comparison against a date that will one day arrive.
    expires_at  timestamptz,
    -- Lifting keeps the row. A ban that was issued and withdrawn is a thing that happened, and
    -- deleting it leaves the next administrator reading a clean account.
    lifted_at   timestamptz,
    lifted_by   uuid        references accounts (id) on delete set null,
    lift_notes  text        not null default '',
    constraint bans_lifted_together check ((lifted_at is null) = (lifted_by is null))
);

-- Partial, because the only question asked on the sign-in path is "what is in force for this
-- account", and the lifted rows are history that path never reads.
create index bans_in_force on bans (account_id, expires_at) where lifted_at is null;
create index bans_by_account on bans (account_id, issued_at desc);

-- Who changed what. Promotions, demotions, bans and lifts.
--
-- Separate from `bans` although a ban writes to both: a ban is a state the sign-in path reads
-- every time, and this is a log nothing but a user page reads. Mixing them would put an
-- append-only table in front of the hottest query here.
create table admin_actions (
    id         bigserial   primary key,
    -- Null where the account has since been deleted. The line stays: the subject is gone and
    -- the fact that somebody acted is not.
    actor_id   uuid        references accounts (id) on delete set null,
    subject_id uuid        references accounts (id) on delete set null,
    -- `lc_identity::actions::Action`, by name, on the same reasoning as `bans.reason`.
    action     text        not null,
    -- A sentence for a person to read: "level 3 to level 2", "banned for 2 months". Rendered
    -- when the action is taken rather than reconstructed later, so a rename of a level or a
    -- reason does not rewrite what the log says happened.
    detail     text        not null default '',
    at         timestamptz not null default now()
);

create index admin_actions_by_subject on admin_actions (subject_id, at desc);
create index admin_actions_by_actor on admin_actions (actor_id, at desc);
