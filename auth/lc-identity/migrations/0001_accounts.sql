-- Accounts, and where each one came from.
--
-- Three tables and one partial index, and the index is the interesting part: it is the
-- account-alignment rule from lightcone/docs/16-identity.md expressed as a constraint rather
-- than as a convention someone has to remember in application code.

create table accounts (
    id            uuid primary key,
    display_name  text        not null,
    created_at    timestamptz not null default now()
);

-- One row per way of signing in to an account. `subject` is whatever the provider calls the
-- user: Google's `sub`, a Discord snowflake, or the normalized email for a password account.
create table links (
    provider       text        not null,
    subject        text        not null,
    account_id     uuid        not null references accounts (id) on delete cascade,
    -- Normalized to lowercase on the way in. Null where the provider gave none.
    email          text,
    -- Whether the *provider* verified it. Google's `email_verified`, Discord's `verified`. A
    -- password account's email is false until delivery exists to prove otherwise.
    email_verified boolean     not null default false,
    created_at     timestamptz not null default now(),
    primary key (provider, subject)
);

create index links_by_account on links (account_id);

-- **Only a verified address may align accounts, and only one account may hold one.**
--
-- Partial, so the unverified addresses a password provider hands out do not collide with each
-- other or with anything real. Without the `where`, registering a password account against
-- someone's Google address would be an account takeover; with it, the two simply coexist until
-- one is verified.
create unique index links_one_account_per_verified_email
    on links (email) where email_verified and email is not null;

-- The credential, for providers that have one locally. Today that is `password` and only it.
--
-- Its own table rather than a nullable column on `links`, so a query that wants to know who
-- someone is never has their hash in the result set by accident.
create table secrets (
    provider   text        not null,
    subject    text        not null,
    -- The whole PHC string: algorithm, parameters and salt together, so the cost can be raised
    -- later and an old hash upgraded on the next successful sign-in.
    phc        text        not null,
    updated_at timestamptz not null default now(),
    primary key (provider, subject),
    foreign key (provider, subject) references links (provider, subject) on delete cascade
);

-- A sign-in in flight: handed to a browser as a redirect parameter, spent by the site over a
-- server-to-server call moments later.
--
-- What is stored is the **digest**. A dump of this table contains nothing that can be
-- exchanged, which matters because a code is a bearer credential for an account.
create table signin_codes (
    digest     bytea       primary key,
    account_id uuid        not null references accounts (id) on delete cascade,
    -- Bound to the destination it was issued for, so a code leaked from one site cannot be
    -- redeemed by another that happens to be on the allowlist.
    return_to  text        not null,
    expires_at timestamptz not null
);

create index signin_codes_expiry on signin_codes (expires_at);

-- What a native client keeps instead of a session.
--
-- The desktop build has no website session to lean on, so it signs in through the system
-- browser once and holds this. It trades it for a game ticket on every connection, which is
-- why the ticket can stay sixty seconds long without anyone signing in again.
--
-- The digest, again, not the grant: this is the longest-lived credential in the system and the
-- one most worth not having in a backup.
create table device_grants (
    digest     bytea       primary key,
    account_id uuid        not null references accounts (id) on delete cascade,
    -- What a revocation list shows a person. "Ada's laptop", not a hex string.
    label      text        not null,
    created_at timestamptz not null default now(),
    last_used  timestamptz,
    expires_at timestamptz not null
);

create index device_grants_by_account on device_grants (account_id);
