# Hosting and content delivery

The public face of the game: a website, a blog, and the browser build with its assets. Three
things that ship on three schedules and must not be able to break each other.

Auth is out of scope. Nothing here needs it yet, and everything here is shaped so that when an
auth service exists it slots in at two places, both named below.

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

## What is not wasm-ready today

Four concrete blockers, all in the client, none deep:

| where | problem | fix |
|---|---|---|
| `lc_world::sky::hyg` — `csv::Reader::from_path` | `std::fs`, and a 32 MB CSV | a binary sky chunk fetched over HTTP; keep the CSV importer behind the existing `hyg` feature for native tooling |
| `bin/lightcone.rs` — `current_exe`, `env::args` | neither exists in a browser | a sibling `bin/lightcone_web.rs`; the flags it parses come from the query string |
| `AssetPlugin { file_path }` | points at a local directory | on wasm, set it to the CDN prefix for the build — Bevy's default reader treats it as an HTTP base |
| shader roots are split | `lc-client/assets/shaders/` has three; `em-render` names `body_point.wgsl` and `encounter_marker.wgsl`, which live in the *app's* `assets/` | a staging step that merges the roots into one tree per build. This one fails as a 404 at runtime in the browser, not at build time, which is the worst way to find out |

The sky chunk is the interesting one. 120 000 rows of CSV is 32 MB; the fields the client
actually reads, packed as fixed-width binary, are a few megabytes and parse without a CSV
reader. It is a new `lc-world` module and a small offline tool, and it deletes both the
largest download and the only `std::fs` call in the client's tree.

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
cdn.<domain>/game/<build-id>/assets/sky/hyg-<content-hash>.lcsky
```

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

First estimates. W3 replaces them with measurements.

| object | raw | brotli |
|---|---|---|
| `lightcone_web_bg.wasm` | 20–30 MB | 6–9 MB |
| js glue | ~100 KB | ~30 KB |
| shaders | ~30 KB | ~8 KB |
| sky chunk | 2–4 MB | ~1.5 MB |
| **first visit** | | **8–11 MB** |
| **repeat visit, same build** | | **0** |

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

---

# The seam

## Promotion is one row

CI, on a tag:

1. build wasm, `wasm-opt`, stage the merged asset tree
2. hash and name the sky chunk
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
| `CDN_BASE` | `https://cdn.<domain>` | default when a release row does not carry its own |
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

## W3 — The browser build

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

## W4 — Delivery

**Deliver:** the bucket, the CDN, the publish script, `releases` and `channels`, the
`/internal/release` endpoint, and `/play` with its loader.

**Done when:** promotion is one row and rollback is one row, neither touching a deploy; a
second build coexists with the first and `?build=` reaches it; **the site serves `/play`
correctly with Postgres stopped**; the wasm arrives brotli-compressed as
`application/wasm` and DevTools shows streaming compilation; an unpublished build id in
`?build=` is rejected.

**Do not:** add accounts, saves, or matchmaking. `/play` launches a client; what it connects
to is phase 8's problem.

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
5. **Does the sky chunk belong to `lc-world` or a new `lc-pack`?** It is a format plus a
   packer, and phase 4's provider interface is the natural home for the reader. Decide when
   writing it, not now.
