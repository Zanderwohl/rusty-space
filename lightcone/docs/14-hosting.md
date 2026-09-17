# Hosting and content delivery

The public face of the game: a website, a blog, and the browser build with its assets. Three
things that ship on three schedules and must not be able to break each other.

Auth is out of scope here. Nothing on this page needs it, and everything on it is shaped so that
the auth service slots in at two places, both named below. That service is designed in
[16-identity.md](16-identity.md), which adds a third integration point this page did not
anticipate: `/play` mints the game ticket, because the player's website session is the only
place a browser client can get an identity without running an OAuth dance from the CDN origin.

## Three deploy units

| unit | artifact | lives | cadence | rollback |
|---|---|---|---|---|
| site | Docker image | any container host | whenever a post or a page changes | redeploy the previous tag |
| game build | wasm + assets under one build id | object storage behind a CDN | whenever the client changes | point a channel at an older build id |
| game server | `lc-server`, phase 8 | its own host | its own schedule | its own problem |

They share exactly one string: the site knows a **build id**, and a CDN base URL to prefix it
with. Nothing else crosses. The site cannot read the game database, the game cannot read the
site database, and neither is in the other's dependency tree.

## Decided: the site is a separate cargo workspace

`web/`, with its own `Cargo.toml` declaring `[workspace]` and its own `Cargo.lock`. Packages
inside it keep the `lc-` prefix — `lc-web` — but they are not workspace members of the root.

The site shares no code with the game. Putting it in `crates/` would make every `cargo test
--workspace` on the game compile axum, tower and sqlx, and would pin the site's tokio to
whatever the game's lockfile resolved. Two lockfiles is the mechanism that makes "decoupled"
true rather than aspirational.

The cost is a second CI job and a second `cargo update` to remember. The check that keeps the
split honest is cheap:

```bash
grep -rn 'path = "\.\./\.\./crates' web/*/Cargo.toml     # must be empty
```

If that ever needs to be non-empty — a shared manifest type, say — the answer is a small
`lc-manifest` crate published to neither workspace and vendored to both, or JSON with a
version field. Not a path dependency.

---

# The site

## Stack

| layer | choice | why not the alternative |
|---|---|---|
| runtime | tokio 1, axum 0.8 | — |
| middleware | tower-http: trace, compression, timeout, set-header | — |
| templates | **maud** | askama's template files buy nothing when every fragment is also a function; maud makes "return a partial or a whole page" a composition question, and the compiler checks it |
| styling | SCSS compiled by **grass** at startup, semantic classes | no Node in the build, and no utility-class vocabulary to learn twice |
| interactivity | htmx, pinned and vendored | listed below; it is a very small list |
| markdown | pulldown-cmark + syntect | — |
| database | PostgreSQL via sqlx 0.8, `SQLX_OFFLINE=true` with a committed `.sqlx/` | `lc-store` uses tokio-postgres because it hand-writes range scans; the site writes ordinary queries and wants them checked at compile time |
| feeds | the `rss` crate | hand-rolled XML escaping is a bug farm |

## Routes

| route | what | dynamic? |
|---|---|---|
| `/` | landing: what the game is, one image, one link to `/play` | no |
| `/play` | the loader shell | build id from the DB |
| `/blog` | reverse-chronological feed | no |
| `/blog/:slug` | a post | no |
| `/blog/tag/:tag` | filtered | no |
| `/feed.xml`, `/feed.json` | syndication | no |
| `/about`, `/docs/*` | static pages from markdown, same pipeline as posts | no |
| `/sitemap.xml`, `/robots.txt` | — | no |
| `/healthz` | liveness, touches nothing | no |
| `/readyz` | readiness, pings the pool | yes |
| `/v/:build/*` | site CSS, images, fonts, the htmx build | no |
| `/internal/release` | CI promotes a build (see below) | yes |

Everything but `/play` and `/internal/release` works with JavaScript disabled and with
PostgreSQL stopped.

## The site is documents

**No JavaScript is the design, not a graceful degradation.** Every page is HTML that a browser
renders on arrival; `/play` is the single exception, and what it does is hand over to the game.
Anything that wants real interactivity belongs in the client, which is a Bevy application with
a WebGPU context and no reason to be re-implemented badly in a browser DOM.

So htmx earns its place in four spots: blog pagination, tag filtering, the newsletter form,
and — later — the account pages the auth service will bring. That is the whole list, and each
one is a page that works without it.

Two rules:

- **Vendor the exact build.** Serve it from `/v/<site-build>/vendor/htmx.js`, not from a
  third-party CDN by floating tag. The one script permitted to
  rewrite the DOM should change when you change it and not otherwise.
- **Every `hx-` endpoint also answers a plain request.** The handler returns a fragment when
  `HX-Request` is present and the full page when it is not. In maud that is one `if`, and it
  means no route is reachable only through htmx.

On the version: this plan assumes only the attribute model common to every htmx major. Three
things have moved between majors and must be read out of the release notes before the first
fragment is written — attribute inheritance, the default swap style, and event/`hx-on`
naming. I have not verified htmx 4's specifics from here; check them, then pin.

## Styling: semantic classes, and as few of them as possible

A class names **what a thing is**, never what it looks like. `.post-meta`, `.release-badge`,
`.callout`, `.figure-caption` — not `.mt-4`, not `.text-muted`, and not `.grid-cols-2`. A
utility vocabulary moves the styling into the markup, which means every template change is
also a design change and the stylesheet stops describing anything.

**A class used once is a bug.** Either the name is too specific and wants generalising, or the
thing wanted an element selector and not a class at all. This is checkable rather than
aspirational, because maud writes classes as string literals:

```bash
grep -rho 'class="[^"]*"' web/lc-web/src | tr -d '"' | sed 's/class=//' \
  | tr ' ' '\n' | sort | uniq -c | sort -n | awk '$1 == 1'
```

Anything that prints is a review item. Run it at the end of a phase, the way
`tools/api_surface.py` is run for a crate.

Most pages should add **zero** classes. pulldown-cmark emits plain `<h2>`, `<p>`,
`<blockquote>`, `<table>`, `<pre><code>` with no attributes at all, so element selectors are
already the natural fit for the blog — the entire post body is styled without the renderer
emitting a single class. That is not a coincidence to work around; it is the reason semantic
CSS and a markdown pipeline suit each other.

```
web/static/
  styles/
    application.scss     # the only entry point; @use everything below
    _tokens.scss         # colour, type scale, spacing, as custom properties
    _base.scss           # element selectors: headings, p, a, lists, table, code
    _layout.scss         # .page, .column, .rail, .stack, .cluster
    _components.scss     # .post-card, .post-meta, .tag-list, .callout, .figure
    _play.scss           # the loader — the one page with a sheet of its own
  images/
  fonts/
  vendor/htmx.js
```

Three rules that keep it from rotting:

- **Tokens are custom properties, not SCSS variables.** `--ink`, `--rule`, `--measure`. A
  `@media (prefers-color-scheme: dark)` block then redefines them at `:root` and the rest of
  the sheet is untouched. SCSS variables compile away and cannot do that.
- **Nesting stops at two levels.** Deep nesting manufactures unique compound selectors, which
  is the same mistake as a unique class wearing a different hat.
- **Write selector names out; no `&__` concatenation.** A name you cannot grep for is a name
  nobody will find when they need to change it.

### Compilation

`grass` — a Sass implementation in Rust, so the toolchain stays `cargo` and the container needs
no Node. `~/rust/sandhill/server` does this at startup and it is the right shape:

```rust
let css = grass::from_path("static/styles/application.scss", &grass::Options::default())?;
```

One difference from that example, which writes the result next to the source: compile **into
memory** and serve it from there. The container runs distroless with a read-only root
filesystem, and a server whose first act is to write into its own image is a server that
cannot. The SCSS source ships in the image — it is a few kilobytes — and the compiled sheet
lives in the app state behind an `Arc<str>`.

A failed compile **fails the boot**. A site that came up without a stylesheet is worse than
one that did not come up: the second is an outage and the first is an outage that
returns 200.

In `SITE_ENV=dev`, a `notify` watcher on `static/styles/` recompiles into the same slot on
change, alongside the content watcher. Neither exists in the production binary.

### Versioning

Lifted from sandhill, because it is already right: assets are served under
`/v/<site-build>/…`, with `Cache-Control: public, max-age=31536000, immutable`, and the path
segment is **not** validated against the running build. It exists only to make the URL change
when the image does. The handler strips the segment textually and hands the rest to `ServeDir`
under `static/`, so it can only ever serve files this image ships, and a request carrying an
older build id is correctly answered with the current bytes.

`<site-build>` is the site's git short SHA, baked at compile time. Docker build contexts
routinely lack `.git`, so pass it as `--build-arg SITE_BUILD=$(git rev-parse --short HEAD)` and
have `build.rs` fall back to the crate version when the variable is absent.

**This is not the game's build id.** Two short hex strings, two meanings: `/v/<site-build>/`
versions the site's own CSS and images, and `game/<build-id>/` on the CDN versions the wasm.
They change on unrelated schedules and nothing should ever compare them. Name the variables so
that confusing them requires effort.

## The blog

Markdown on disk, baked into the image. A post is published by a redeploy, which the brief
accepts and which buys atomic rollback of content and code together.

```
web/content/                     # prose: parsed at startup
  posts/2026-03-14-light-delay.md
  pages/about.md
web/static/                      # site resources: served, not parsed
  styles/ images/ fonts/ vendor/
```

`content/` is the site's words and `static/` is its resources. Neither holds anything the game
needs — game assets live on the CDN and reach the browser without passing through this server
at all.

Frontmatter is a TOML block fenced by `+++`, parsed with `toml`:

```toml
+++
title   = "What thirty light-years does to a patch note"
summary = "One paragraph for the feed and the OG card."
tags    = ["design", "relativity"]
draft   = false
+++
```

**The date is not in there.** It is in the filename, which is the only copy — two sources that
can disagree is one too many, and the filename is the one that also sorts a directory listing.
`updated` is front matter, because there is nowhere else for it to live.

**Parsed once at startup, not per request.** Startup walks `content/`, renders each post to
HTML, highlights code with syntect, and builds a `Vec<Post>` sorted by date descending plus a
`HashMap<&str, usize>` from slug. A request is an index lookup and a template render. A
thousand posts is a few megabytes of resident HTML and a startup cost measured in
milliseconds; if that ever stops being true, the fix is to render at build time, not to add a
cache.

Details that are cheap now and annoying later:

- `draft = true` is excluded when `SITE_ENV=production` and listed at `/drafts` otherwise.
- The slug comes from the filename with the date prefix stripped, so renaming a file breaks a
  URL. Startup asserts slugs are unique and fails loudly rather than serving whichever won.
- **Posts write `/static/…` for an image and the renderer rewrites it** to the versioned URL.
  A markdown file cannot know the build id, and it should not have to: the authored path is
  stable and the immutable one is derived.
- A dev-only file watcher re-runs the parse on change. Behind `cfg(debug_assertions)`; the
  production binary does not watch anything.
- Post images go on the CDN, not in the image. A 3 MB screenshot in the container is 3 MB in
  every deploy forever.
- The feed carries full content, not summaries, and a stable `guid` per post.

## The site database

Its own database and its own role on the cluster — `lc_site`, owned by `lc_site_rw`, with no
grant on `lc_game`. Same cluster is fine until it isn't; separate credentials are what makes
splitting them later a config change.

```sql
releases (
  build_id     text primary key,     -- git short sha, or semver+sha
  cdn_base     text not null,        -- lets a build move CDNs without a code change
  manifest_key text not null,
  wasm_bytes   bigint,
  published_at timestamptz not null,
  notes        text,
  yanked       bool not null default false
)

channels (
  name       text primary key,       -- 'stable' | 'beta' | 'nightly'
  build_id   text not null references releases(build_id),
  updated_at timestamptz not null
)

subscribers (email citext primary key, confirmed_at timestamptz, token uuid)
post_views  (slug text, day date, count bigint, primary key (slug, day))
```

That is the whole schema and it is allowed to stay this small. No game data crosses into it,
ever. If the site needs to show server population, it calls an HTTP endpoint on `lc-server`;
it does not open a connection to `lc_game`. Two readers of one schema is how a game database
migration starts breaking a marketing page.

**The site boots and serves with Postgres down.** The pool is lazy, `/healthz` does not touch
it, and `/play` falls back to `FALLBACK_BUILD_ID` from the environment when the channel lookup
fails. Losing the database costs view counts and signups, not the site.

Migrations are `sqlx::migrate!` embedded in the binary, run at startup. sqlx takes a Postgres
advisory lock, so this is still correct when the host scales to more than one replica.

---

# The game build

## The wasm pipeline

```
cargo build -p lc-client --bin lightcone_web \
      --target wasm32-unknown-unknown --profile wasm-release
wasm-bindgen --target web --out-dir stage/ target/.../lightcone_web.wasm
wasm-opt -Oz --enable-bulk-memory --enable-nontrapping-float-to-int \
      stage/lightcone_web_bg.wasm -o stage/lightcone_web_bg.wasm
```

```toml
[profile.wasm-release]
inherits      = "release"
opt-level     = "s"
lto           = "fat"
codegen-units = 1
panic         = "abort"
strip         = true
```

`CLAUDE.md` says do not build release. That rule is about the desktop dev loop, where LTO buys
minutes of link time for a speedup nobody feels. It does not apply here: this is the shipped
artifact, download size *is* the product, and the build runs in CI. Say so in the CI job's name
so a future session does not helpfully delete it.

## What was not wasm-ready — measured, not predicted

This section originally listed four blockers. Building it found that **one of them did not
exist, one was missing, and the compile story was far easier than expected**: with the
`getrandom` backend named, `lc-client` compiled for `wasm32-unknown-unknown` unmodified.

| where | problem | what it took |
|---|---|---|
| `getrandom` 0.3 | refuses `wasm32-unknown-unknown` unless a backend is named — **and needs both** the crate feature and a `--cfg` rustflag; either alone fails | `.cargo/config.toml` plus a wasm-only dependency. This was not predicted and was the only thing stopping the build |
| `lc_world::sky::hyg` — `csv::Reader::from_path` | `std::fs`, and a 34 MB CSV | the sky chunk, below. `csv` is now absent from the browser build's dependency tree entirely, which is a stronger claim than a grep for `std::fs` |
| `bin/lightcone.rs` — `current_exe`, `env::args` | compile fine on wasm and do the wrong thing quietly | `bin/lightcone_web.rs`, with the flag parsing factored into `lc_client::entry` so both binaries share one vocabulary |
| `AssetPlugin { file_path }` | points at a local directory | on wasm it is an HTTP prefix, so a CDN URL is the whole integration. Also `meta_check: AssetMetaCheck::Never` — Bevy probes for a `.meta` beside every asset, which over a CDN is a round trip and a cached 404 per file |
| ~~shader roots are split~~ | **wrong.** The client instantiates exactly three `em-render` materials — relativistic starfield, population, body surface — and its own asset root already holds all three shaders. The `body_point` and `encounter_marker` shaders belong to the app, which the client does not use | no merge step. `tools/check-shaders.sh` derives the needed set from the `em_render::` modules the client imports and asserts each exists, so this stays true rather than happening to be true |

`starfield.wgsl` does exist in both roots and they differ — 3 kB against 16 kB. That is not a
collision: `em-render` ships material definitions and each host supplies its own shader at the
path the material names, which is the arrangement [06-crate-layout.md](06-crate-layout.md)
chose. Two hosts, two shaders, one path, and neither build ever sees the other's.

## The sky chunk

120 000 rows of CSV is 34 MB. The same catalogue packed is **39 bytes a star**, and the client
only ever uses the nearest 6 000 of them, so the shipped chunk holds 8 000 — a little above
`SKY_LIMIT`, so raising that does not silently shorten the sky.

**What is stored is what cannot be recomputed.** Radius, temperature, mu, mass and metallicity
all follow from colour index, luminosity and velocity, so they are absent and derived on load.
`StarRecord::assemble` is that derivation and both importers go through it; the chunk cannot
disagree with the code that made it, because there is only one.

The equivalence test runs the real 107 000-star catalogue down both routes and compares star
for star. **It found four bugs**, none of which would have shown up as anything but a slightly
different sky:

| bug | why it was invisible |
|---|---|
| singleton groups were pruned before derivation, so a star whose partner was later rejected kept a group it was alone in | grouping is not yet consumed by anything |
| eleven stars sit at exactly `B-V = -0.4`, the low end of `BV_VALID`, and the nearest `f32` is *below* it — so they assembled from the CSV and failed from their own chunk | eleven stars out of a hundred thousand |
| zero as the "no group" sentinel collided with HYG's Sun, whose `comp_primary` really is 0 | the Sun acquired a phantom companion |
| a rename shadowed the star's own key with the group's inside a struct literal | every identity in a decoded chunk was wrong, and every star still looked like a star |

Colour index is stored as thousandths rather than `f32`: exact for every value HYG publishes,
two bytes instead of four, and it is what fixes the boundary. Grouping carries a presence bit
in the component byte, because no key value is free to mean "absent".

## Decided: single-threaded, therefore no cross-origin isolation

Threads in wasm need `SharedArrayBuffer`, which needs COOP/COEP on the document, which needs
`Cross-Origin-Resource-Policy` on **every** subresource including all CDN objects, and which
breaks embedding the page anywhere. [07-rendering.md](07-rendering.md) already chose
single-threaded Bevy; the hosting consequence is that the CDN needs only ordinary CORS and the
page needs no special headers.

If profiling later says threads are worth it, the migration is: COOP/COEP on `/play` only,
CORP on every CDN object, re-check every third-party embed. Not free, not foreclosed.

## Asset layout

Everything under a build id, and **nothing on the CDN is ever mutated**. No overwrite means no
purge, no stale-edge class of bug, and rollback that is a pointer change rather than a
propagation wait.

```
cdn.<domain>/game/<build-id>/manifest.json
cdn.<domain>/game/<build-id>/lightcone_web.js
cdn.<domain>/game/<build-id>/lightcone_web_bg.wasm
cdn.<domain>/game/<build-id>/assets/shaders/*.wgsl
cdn.<domain>/game/<build-id>/assets/sky/catalogue.lcsky
```

The sky is at the same path in every build and is not a parameter: the client always asks for
`sky/catalogue.lcsky`, and which catalogue that is is decided by the build that staged it. A
build directory is immutable, so the name needs no content hash to be cacheable.

```json
{
  "build_id": "2026.09.1+a10eac4",
  "engine": "bevy-0.17",
  "requires": ["webgpu"],
  "entry": "lightcone_web.js",
  "wasm":  "lightcone_web_bg.wasm",
  "asset_base": "assets/",
  "bytes": { "wasm": 24118272, "wasm_br": 7340032 },
  "protocol": 3
}
```

`protocol` is the one field the game server also cares about: a client whose protocol number
the server does not accept is told to reload, and that is the only version negotiation between
the two. The site does not read it and does not need to.

## Headers

| object | `Content-Type` | `Cache-Control` | note |
|---|---|---|---|
| `*_bg.wasm` | `application/wasm` | `public, max-age=31536000, immutable` | wrong type silently disables streaming compilation |
| `*.js` | `text/javascript` | same | |
| `*.wgsl` | `text/plain` | same | |
| `*.lcsky` | `application/octet-stream` | same | |
| `manifest.json` | `application/json` | same — it is per-build and immutable too | |

Upload the wasm **pre-compressed**: brotli quality 11, `Content-Encoding: br`, keeping
`Content-Type: application/wasm`. Streaming compilation still works, because the browser
decompresses transparently before handing bytes to the compiler. Object storage will not do
this for you the way a full CDN origin might; it is an upload flag, and forgetting it triples
the download.

CORS on the bucket: `Access-Control-Allow-Origin` for the site's origin. The wasm fetch and
every Bevy asset fetch are cross-origin by construction.

## What a play actually costs

**Measured**, on the build at W3.

| object | raw | brotli |
|---|---|---|
| `lightcone_web_bg.wasm` | 26.7 MB | **6.12 MB** |
| js glue | 0.17 MB | 0.02 MB |
| shaders (3 × wgsl) | 0.03 MB | 0.01 MB |
| sky chunk (8 000 stars) | 0.31 MB | 0.25 MB |
| **first visit** | 27.2 MB | **6.4 MB** |
| **repeat visit, same build** | | **0** |

The estimate was 8–11 MB brotli and the measurement is 6.4, so the sizing above holds with
room. Two of the numbers moved a long way from the guess and both are worth keeping:

- The **sky** was estimated at 1.5 MB compressed and is 0.25. Two thirds of that came from
  packing only the stars the client uses; the rest from storing inputs rather than results.
- The **wasm** is still 94% of the download, so it is the only figure worth optimising. `bevy`
  is on default features here; trimming those is the obvious next lever and was deliberately
  not pulled in W3, where the goal was a build that runs.

`wasm-opt -Oz` takes 26.7 MB off 32.1 and costs 15 seconds. `--profile wasm-release` is the
exception to `CLAUDE.md`'s "do not build release": there the rule protects a dev loop, and
here the artifact *is* the product.

The client has no textures, no skybox and no fonts — the starfield is generated from the
catalogue and body appearance is derived, per [07-rendering.md](07-rendering.md). So the
download is the binary, and the lever that matters is `opt-level = "s"` plus `wasm-opt -Oz`,
not asset compression.

At 10 MB and 10 000 plays a month that is 100 GB of egress. Storage is nothing: a few hundred
megabytes per build, fifty builds retained, under 20 GB.

| provider | 100 GB egress | note |
|---|---|---|
| Cloudflare R2 + CDN | $0 | **recommended**; zero egress, S3 API, custom domain on the bucket |
| Bunny.net | ~$1 | simplest to set up; fine alternative |
| S3 + CloudFront | ~$9 | the egress line grows linearly with the thing you want to grow |

Egress is the entire cost story of hosting a 10 MB game, so pick on egress. R2's S3-compatible
API means the publish script is `aws s3 cp` either way and switching later is a config change.

## The development CDN

There is no cloud bucket yet and there will not be for a while, so delivery is rehearsed
locally: a **Caddy container** on `rocinante:3101` serving a volume that builds are published
into. `tools/dev-cdn/Caddyfile` is the real deliverable — it is the header policy, and it is
what should be handed to R2 or Bunny when there is one.

**Not a small web application.** The obvious move is a FastAPI app with a couple of routes
matching the CDN's URL shape, and it is the wrong one: `/game/<build-id>/…` is a directory
layout, so there is no request to route, and everything that actually breaks is a header —
`application/wasm`, pre-compressed content served with the *original* content type, CORS,
immutability. A hand-written server gets those wrong in the direction that hides the bug until
production. A static server with an explicit header policy cannot.

Fault injection — latency, partial responses, a 500 on the wasm — is the one thing a small app
would do better, and it is worth adding when the loader's failure paths need exercising rather
than now.

`tools/publish-build.sh` is the seam. Today it is `docker cp` into a volume; the day there is a
bucket it becomes `aws s3 cp --recursive` with the headers the Caddyfile already names, and
nothing else moves. It refuses to publish over an existing build id, and refuses a `-dirty`
one outright — a build that exists on one laptop is not something anyone can roll back to.

### Two things that only show up over HTTP

Both were found by running a real browser against it, and both change how the dev setup has to
be used.

**Chrome only advertises `Accept-Encoding: br` on a secure origin.** Against the plain-HTTP dev
CDN a browser asks for `gzip, deflate` and nothing else — so with only `.br` files present it
took the whole 26.6 MB uncompressed, in six requests that were all `200 OK`. Nothing looked
wrong. Builds now carry `.gz` as well as `.br`, which brings the dev download to 9.4 MB and,
more importantly, means the compression path is exercised somewhere other than production.

**WebGPU needs a secure context, and so the page cannot be served over plain HTTP.** Loading
the loader from `http://rocinante.local:3101/` shows the WebGPU refusal, correctly: `navigator.gpu`
does not exist there. It works from `http://127.0.0.1:3200/` because localhost counts as
secure. So the arrangement that works today is **shell on localhost, build on the CDN**, which
is also the cross-origin case worth testing.

This lands on W4: the site on `rocinante:3100` over plain HTTP **cannot run the game**, whatever
`/play` does. An SSH tunnel to localhost is the cheap fix for development; a real domain with
real TLS is the answer past that. Mixed content rules out the other pairing — an HTTPS page may
not fetch an HTTP build — so the day the site gets TLS, the CDN needs it the same day.

---

# The seam

## Promotion is one row

CI, on a tag:

1. build wasm, `wasm-opt`, stage the merged asset tree
2. pack the sky chunk to `assets/sky/catalogue.lcsky`
3. write `manifest.json`
4. `aws s3 cp --recursive` into `game/<build-id>/` with the headers above
5. `POST /internal/release` on the site: `{build_id, cdn_base, manifest_key, bytes}` — inserts
   into `releases`, does **not** touch `channels`

Promoting to players is a separate, deliberate act: update `channels` where `name='stable'`.
Rollback is the same statement with an older id. Neither requires a site deploy, a CDN purge,
or a game server restart.

`/play?build=<id>` and `?channel=beta` override the lookup for testing. Both are validated
against the `releases` table — a build id is an index into rows you published, never a URL and
never a path fragment. Letting a query string name an arbitrary origin for the script you are
about to execute is the vulnerability this paragraph exists to prevent.

## The loader

`/play` is a small HTML shell with an inline, nonced bootstrap. In order:

1. **Capability check before anything large.** `if (!navigator.gpu)` — show the WebGPU message
   and stop. Checking after a 10 MB download is a download wasted on someone who cannot play.
2. Fetch `manifest.json`.
3. Fetch the wasm through a `TransformStream` that counts bytes, wrap it back into a
   `Response` with the original headers, and hand that to `instantiateStreaming`. This is how
   you get a real progress bar *and* streaming compilation; picking one is the common mistake.
4. `init()`, canvas sized, focus handled.

Every failure has a written message: no WebGPU, adapter request failed, manifest 404, wasm
fetch failed, panic during init. Keep the browser-support wording in one place and derive the
page text from it — it changes faster than the document does.

---

# The container

Multi-stage, cargo-chef for dependency caching, distroless runtime.

```dockerfile
FROM rust:1-bookworm AS chef      # cargo-chef prepare / cook
FROM chef AS builder              # SQLX_OFFLINE=true SITE_BUILD=$SITE_BUILD cargo build --release
FROM gcr.io/distroless/cc-debian12
COPY --from=builder /app/target/release/lc-web /lc-web
COPY web/content /content
COPY web/static  /static          # SCSS source included; compiled at boot
USER nonroot
EXPOSE 8080
ENTRYPOINT ["/lc-web"]
```

Under 100 MB. `content/` and `static/` are baked in, which is what makes a content rollback,
a style rollback and a code rollback the same operation. The site is one image plus
environment and does not depend on the CDN to render a single page — the CDN serves the game,
not the website.

## Building it: the `rocinante` context

Images are built on **`rocinante`**, a docker context over SSH to an idle home-lab machine
that is natively `linux/amd64`.

```bash
docker --context rocinante build -t lightcone-web:<tag> -f web/Dockerfile .
docker --context rocinante run -d --name lightcone-web -p 3100:3100 ... lightcone-web:<tag>
```

Three reasons it beats CI for this, and they are the same reasons sandhill already builds
there: it is free where GitHub Actions minutes are not, it is faster, and it runs when asked
rather than when a queue gets to it. It is also the right architecture — building `amd64` on
an Apple Silicon laptop means qemu emulation, which for a Rust release build is the difference
between minutes and most of an afternoon.

CI's job is therefore `cargo test` and the dependency-invariant greps. **CI does not build or
push images.** If that ever changes, it is because a deploy needs to happen when the machine
is off, and the answer then is a registry, not emulation.

Ports 3000–3999 on that host are the range for this project; 3000 is sandhill's, and the site
takes **3100**.

## Configuration

Environment only; no config file, no secrets in the image.

| var | | |
|---|---|---|
| `BIND_ADDR` | `0.0.0.0:8080` | |
| `DATABASE_URL` | | optional — absent means degraded mode, and that is a supported state |
| `CDN_BASE` | `https://cdn.<domain>` | default when a release row does not carry its own. **Not yet implemented** — `/play` is what reads it, and that is W4; a field nothing reads is worse than a missing one |
| `FALLBACK_BUILD_ID` | | what `/play` serves when the DB is unreachable |
| `SITE_ENV` | `production` \| `staging` \| `dev` | gates drafts and the file watcher |
| `RELEASE_TOKEN` | | shared secret for `/internal/release` |
| `SITE_BUILD` | build arg, not runtime | the `/v/<id>/` segment; falls back to the crate version |
| `RUST_LOG` | | |

## Observability

`TraceLayer` plus `tracing-subscriber` with JSON output, a request id on every span,
`/healthz` (no dependencies) and `/readyz` (pool ping) split so a container orchestrator does
not restart the site because Postgres blinked. Graceful shutdown on SIGTERM via axum's
`with_graceful_shutdown`. `tower_governor` on the two form endpoints. Metrics can wait until
something needs measuring.

## Security, with auth deferred

- **CSP.** `script-src 'self' 'nonce-…' 'wasm-unsafe-eval'` — strict CSP blocks
  `WebAssembly.instantiate` without `'wasm-unsafe-eval'`, which is the first thing that will
  go wrong on `/play`. `connect-src` and `worker-src` must list the CDN origin.
  `frame-ancestors 'none'`, `object-src 'none'`, nosniff, HSTS.
- **`/internal/release`** is the one privileged endpoint. Today: a constant-time comparison
  against `RELEASE_TOKEN`, plus an IP allowlist if the host makes that easy. **This is the
  first thing that moves behind the auth service**, along with the eventual account pages.
  Those two are the whole integration surface, which is the point of deferring it.
- No user input is stored except an email address, and that wants double opt-in before the
  first send.

---

# Buildout

Same shape as [12-buildout.md](12-buildout.md): what must exist, what it delivers, how you
know. W1 and W2 depend on nothing and can start today. W3 needs phase 6, which is done.

## W1 — Skeleton and deploy — **done**

**Deliver:** `web/lc-web`, axum, maud, the static routes, the SCSS pipeline with its token and
base sheets, `/v/<build>/` asset serving, health checks, the Dockerfile, and one deploy to a
real host.

**Done when:** `docker run -e BIND_ADDR=... <image>` serves the landing page with no other
configuration; the image is under 100 MB; SIGTERM drains in-flight requests; a deliberately
broken `.scss` fails the boot rather than serving an unstyled page; the separate-workspace
grep is in CI.

**Do not:** add a database, htmx, or a build pipeline. Write `_base.scss` and `_tokens.scss`
and stop — components arrive when a second page wants one.

**What it measured.** Image 30.5 MB. Cold amd64 release build on `rocinante`, 46 s. Compiled
stylesheet 4.4 kB, 1.4 kB brotli. Seven classes site-wide — `stack`, `lede`, `cta`, `page`,
`columns`, `wordmark`, `fine-print` — and both pages' prose is styled entirely by element
selectors, so the markdown pipeline in W2 walks into it unchanged. The container runs
`--read-only --cap-drop ALL`, which the in-memory stylesheet is what makes possible.

One bug worth keeping, because it looks like a grid problem and is not. As a bare rule,
`.stack + .stack` also fires between grid siblings, so the second column of a `.columns` sat a
whole section gap below the first. Section spacing belongs to the prose column, not to the
primitive: `.page > .stack + .stack`. Layout primitives should say how children are arranged
and nothing about where the primitive itself sits.

`--check-styles` exists for the same reason the boot fails on a bad sheet: compiling at boot
turns a CSS syntax error into a failed deploy, and CI compiling it first turns it back into a
failed build.

## W2 — The blog

**Deliver:** the markdown pipeline, post and tag pages, Atom and JSON feeds, sitemap, OG cards,
the dev file watcher.

**Done when:** adding a file to `content/posts/` and redeploying publishes it; the feed
validates; a post with three code blocks renders highlighted; duplicate slugs fail at startup
rather than at request time; drafts are invisible in production and listed in staging; **the
post body is styled entirely by element selectors**, and every entry the single-use-class scan
prints has been looked at and is either shell furniture or a name with an obvious second
caller.

That last clause was originally "the scan prints nothing", which was written before there was
any markup and was wrong. A class that appears once in a template but on every page — the
wordmark, the prose column — is exactly the case the scan is meant to raise and a human is
meant to dismiss. A check that cannot be satisfied gets disabled.

**Do not:** build an admin UI. The editor is a text editor and the CMS is git. No pagination
and no htmx until a page is long enough to need them.

**What it measured.** Eleven classes site-wide, four of them single-use and all four reviewed.
The renderer emits none of them: a post body is `_base.scss` and nothing else, which is what
the section above predicted and the reason to have predicted it.

Syntax highlighting is **classes, not inline styles** — `ClassedHTMLGenerator` with a `syn-`
prefix — so code changes colour with the rest of the page. An inline `style="color:…"` would
have been the same in both themes and no rule could have overridden it.

Three bugs worth keeping, because none of them look like what they are:

| symptom | cause |
|---|---|
| tags always broke onto their own line, whatever the CSS said | a `ul` inside a `p`. Invalid HTML, so the parser closes the paragraph before the list. The fix is a `div`; no stylesheet could have fixed it |
| the gap between a page's sections vanished on the blog index | `.post-list { margin: 0 }` ties `.page > * + *` on specificity and wins on source order. `_base.scss` already zeroes list margins, so the reset was redundant as well as harmful |
| the figure was invisible in dark mode, then vanished entirely | an SVG loaded through `img` is its own document: `currentColor` resolves against *its* root, not the page, so it needs its own `prefers-color-scheme` block. Adding one with an angle bracket in the comment then broke it outright — SVG is XML, and a broken SVG shows as alt text with nothing in the console |

**Caching had to become conditional.** Immutable assets plus a build id that only changes on
commit means an edited stylesheet is invisible until the next commit — the versioning scheme
working exactly as designed, and useless to edit against. Outside production the header is
`no-store`.

## W3 — The browser build — **done**

**Before:** phase 6.

**Deliver:** the wasm profile, `bin/lightcone_web.rs`, the four blockers above fixed, the sky
chunk format and its offline packer, the merged asset staging step, and a CI job that produces
the artifact.

**Done when:** the build loads from a plain local static server and plays; the merged shader
tree is complete, verified by a run that touches every material rather than by inspection;
`grep -rn 'std::fs' ` over the client's own tree is empty; the measured sizes replace the
estimates in this document.

**Do not:** involve the CDN or the site yet. This phase is over when a directory of files
plays in a browser.

**What it measured.** `tools/build-wasm.sh` stages `target/web/<build-id>/` — wasm, glue,
assets, manifest, and a loader page — in about four minutes cold. It runs in a browser on
WebGPU with 7 973 stars, and flying to Proxima shows coordinate time and proper time diverging
(4.60 years out, 1.23 aboard) with the starfield blue-shifted, which is the whole premise
working through a fetch instead of a file.

The loader checks `navigator.gpu` **and** asks for an adapter before touching the wasm, and
both refusals were provoked deliberately: the message appears and no 26 MB download starts. It
gets a byte-accurate progress bar *and* streaming compilation by teeing the response body
through a counting `TransformStream` and rebuilding a `Response` around it, carrying the
original headers — drop those and the content type goes with them.

Bevy's winit loop reports its exit by throwing, so the loader has to not treat that as a
failure. Anything else from `init` is shown to the reader.

**`AssetMetaCheck::Never` matters more than it looks.** The default probes for a `.meta`
sidecar beside every asset; on a filesystem that is a wasted stat, and over a CDN it is an
extra round trip and a negatively-cached 404 per file. Three of them, on every first visit.

## W4 — Delivery — **half done**

**Deliver:** the bucket, the CDN, the publish script, `releases` and `channels`, the
`/internal/release` endpoint, and `/play` with its loader.

**Done when:** promotion is one row and rollback is one row, neither touching a deploy; a
second build coexists with the first and `?build=` reaches it; **the site serves `/play`
correctly with Postgres stopped**; the wasm arrives brotli-compressed as
`application/wasm` and DevTools shows streaming compilation; an unpublished build id in
`?build=` is rejected.

**Do not:** add accounts, saves, or matchmaking. `/play` launches a client; what it connects
to is phase 8's problem.

**Done so far.** The CDN, the publish script, and `/play` — which reads `CDN_BASE` and
`FALLBACK_BUILD_ID` and hands over to the client. That is deliberately the *degraded* path
built first: it is what the site falls back to when the database is unreachable, so the
database adds a lookup in front of it rather than replacing it.

**Still to do:** `releases` and `channels`, `/internal/release`, and promotion as a row.

With no build configured `/play` says so, rather than rendering a loader with nothing to load.

**`/play` needs a secure context**, so over plain HTTP it cannot run at all — see the
development CDN above. Today it is reached through an SSH tunnel, which makes both the site
and the CDN `localhost` and keeps them different origins, so CORS is still exercised:

```bash
ssh -N -L 3100:localhost:3100 -L 3101:localhost:3101 zandy@rocinante.local
```

The loader checks `isSecureContext` **separately from** `navigator.gpu`. They fail together,
because WebGPU is not exposed outside a secure context, and reporting the second sends someone
to download a browser they already have.

**Two bugs, both about something not changing when it should have.**

`build.rs` declared only `rerun-if-env-changed`, so it ran once and the baked site build id
never moved again — frozen at W1's commit through three phases of stylesheet changes. Assets
are served `immutable`, so in production a CSS edit would never have reached anyone who had
already loaded the old one; the versioned URL exists to prevent exactly that and had quietly
stopped doing it. It hid because a development server sends `no-store`, and surfaced only as
`/play` laying itself out with CSS from twelve commits ago. It now watches `.git/HEAD` and the
ref that HEAD names, following the worktree's `gitdir:` pointer.

`#boot { display: grid }` outranks the user-agent sheet's `[hidden] { display: none }` by a
whole class of specificity, so setting `hidden` on the loading overlay did nothing and it sat
on top of a running game. The overlay is now **removed** rather than hidden — no specificity
argument to lose, and it leaves the accessibility tree at the same time. `[hidden]` is also
now `!important` in the base sheet, because that trap is general.

## W5 — Hardening

**Deliver:** CSP with nonces, rate limits, structured logs, real 404 and 500 pages, robots,
canonical URLs, and the failure-message table for the loader.

**Done when:** the CSP is strict and `/play` still runs; every loader failure mode has been
provoked deliberately and shows its message.

---

# Deferred, deliberately

| item | why |
|---|---|
| accounts, saves, profiles | waiting on the auth service; two integration points are named above |
| comments | moderation is a staffing decision, not a feature |
| a CMS | git is the CMS until someone who does not use git needs to post |
| server-side rendering of game state | the site does not read the game database, and this is why |
| i18n | cheap to add to maud later, expensive to do speculatively now |
| multi-region | one container and a global CDN covers a great deal of traffic |

# Open questions

1. **Same origin or subdomain for `/play`?** Same-origin is simpler for CSP and for any future
   API call; a subdomain isolates the game page if COOP/COEP is ever needed. Cheap to decide
   now, expensive to change after links exist.
2. **Blog on the apex or a subdomain?** Apex, unless the site and the blog will ever deploy
   separately. They will not, under this plan.
3. **How many builds to retain?** Storage is nearly free and old build ids are the rollback
   path; fifty is a guess, not a measurement.
4. **htmx 4's inheritance and swap defaults** — read the release notes before the first
   fragment, per the note above.
5. ~~**Does the sky chunk belong to `lc-world` or a new `lc-pack`?**~~ Settled in W3:
   `lc_world::sky::chunk` for the format and `lc-world`'s `skypack` binary for the packer. A
   separate crate would have split `StarRecord` from the importer that produces it, and that
   shared derivation is the point.
6. **How much can `bevy`'s default features be trimmed?** The wasm is 94% of the download and
   nothing has been trimmed yet. Worth a measurement before it is worth an opinion.
7. **How does development get a secure context?** An SSH tunnel works today and needs nothing.
   Caddy's internal CA would work everywhere on the network at the cost of installing a root
   certificate on each machine. A real domain settles it properly. Decide when someone other
   than the author needs to open it.
8. **Bevy retries a failed asset load without bound.** A wrong asset base produced over forty
   thousand requests for one shader before anyone looked. In production that is a client
   hammering whichever origin it was aimed at, so the base must come from a manifest and never
   from anything a person types. Whether it also wants a cap is a question for `lc-client`.
