-- Game builds the site knows about, and which one each channel points at.
--
-- This is the entire coupling between the site and the game: a build id and a base URL. The
-- site never reads the game's database and the game never reads this one.

create table releases (
    build_id     text primary key,
    -- Per release, so a build can move between CDNs without a code change or a redeploy.
    cdn_base     text        not null,
    wasm_bytes   bigint,
    notes        text,
    -- Marks a build unpromotable without deleting it. The bytes stay on the CDN, because
    -- anything already running against them keeps working, and because deleting evidence of
    -- a bad build is how it gets promoted again by muscle memory.
    yanked       boolean     not null default false,
    published_at timestamptz not null default now()
);

create table channels (
    name       text primary key,
    build_id   text        not null references releases (build_id),
    updated_at timestamptz not null default now()
);

-- The channel `/play` serves when the request does not name one.
insert into channels (name, build_id)
select 'stable', build_id from releases limit 0;
