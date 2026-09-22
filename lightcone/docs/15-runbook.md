# Runbook

How to build, publish, deploy and roll back. [14-hosting.md](14-hosting.md) is why any of it
is shaped this way; this is the sequence of commands.

Everything runs on **`rocinante`**, a docker context over SSH to a home-lab machine that is
natively `linux/amd64`. Ports 3000–3999 are this project's; 3000 belongs to another project.

| container | port | what |
|---|---|---|
| `lightcone-proxy` | 80, 443 | TLS, and the only thing that should be reached from a browser |
| `lightcone-web` | 3100 | the site |
| `lightcone-cdn` | 3101 | game builds |
| `lightcone-db` | 3102 | the site's PostgreSQL |
| `lightcone-identity` | 3200 | the identity broker |
| `lightcone-admin` | 3300 | the administration console |
| `lightcone-shard` | 8080, 3400 | the game; 3400 is the console's read-only surface |

They share a docker network called `lightcone` and address each other by container name. The
published ports are for debugging; the addresses that matter are:

| | |
|---|---|
| https://lc.zanderlowry.com | the site, the blog, `/play` |
| https://cdn.lc.zanderlowry.com | game builds |

Both resolve to `10.37.1.100` in public DNS and are reachable only from the LAN. The
certificate is real; see [TLS](#tls).

## Secrets

Three, none of which belong in this repository or in a shell history.

| | where it lives | used by |
|---|---|---|
| Namecheap API key | `~/.config/lightcone/proxy.env` on rocinante, mode 600 | the proxy, at certificate renewal |
| `RELEASE_TOKEN` | `~/.config/lightcone/web.env` on rocinante | the site, which checks it; and `tools/release.sh` |
| PostgreSQL password | `proxy.env` | the site |

`release.sh` needs the token in your shell. Take it from the host without printing it:

```bash
export RELEASE_TOKEN="$(ssh zandy@rocinante.local 'grep -m1 ^RELEASE_TOKEN= ~/.config/lightcone/web.env | cut -d= -f2-')"
export LC_SITE=https://dev.lightconefrontier.com LC_CDN=https://cdn.dev.lightconefrontier.com
```

To set one without it reaching your history or the screen:

```bash
read -rs -p 'value: ' v && sed -i "s|^NAME=.*|NAME=$v|" ~/.config/lightcone/proxy.env && unset v
```

Containers read it with `--env-file`. **That flag is resolved by whichever docker CLI you
invoke**, so a `docker --context rocinante run --env-file ~/...` from a laptop looks for the
file on the laptop and fails. Run those commands over `ssh` instead, which also keeps the
secret on the host that needs it.

## Prerequisites

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version "$(cargo metadata --format-version 1 --locked \
  | python3 -c "import json,sys;print(next(p['version'] for p in json.load(sys.stdin)['packages'] if p['name']=='wasm-bindgen'))")"
brew install binaryen brotli          # wasm-opt, brotli
docker context ls | grep rocinante    # must exist
```

`wasm-bindgen-cli` has to match the `wasm-bindgen` crate the client links, exactly. Taking the
version from the lock file is how the two stay together.

---

## Shipping a game build

```bash
export RELEASE_TOKEN=...               # and LC_SITE if the site is not on localhost:3100
tools/build-wasm.sh                    # stages target/web/<build-id>/
tools/publish-build.sh                 # uploads the most recent staged build
tools/release.sh register <build-id>   # tells the site the build exists
tools/release.sh promote <build-id>    # points a channel at it
```

During development, `tools/release.sh ship` does all four. It exists because you are the only
player; do not reach for it once anyone else is.

Four steps because three of them are reversible and one is not. Publishing is the irreversible
one — a build id on the CDN is never overwritten — and promotion is the one that changes what
players get.

**A build carries its own sky.** `build-wasm.sh` packs `CATALOGUE` to
`assets/sky/catalogue.lcsky` inside the build, and the client asks for exactly that path — there
is no URL parameter for it. To ship a different catalogue, change `CATALOGUE` in the script and
commit, so the build id names the catalogue as well as the code.

**Promoting a build is also repointing the shard**, when the catalogue changed: its `--sky`
names one build's chunk, and a shard reading one sky while players download another disagrees
with them about which system every ship is in. See
[the broker and a shard](#the-broker-and-a-shard-on-rocinante).

Builds staged before 2026-09-17 named the chunk `assets/sky/hyg-v42.lcsky`. A shard pointed at
one of those keeps working; the new name only matters when you repoint it at a newer build.

`build-wasm.sh` refuses nothing but takes about four minutes cold. `publish-build.sh` refuses a
`-dirty` build id: a build that exists on one laptop is not something anyone can roll back to.

### Rolling back

```bash
tools/release.sh promote <older-build-id>
```

No deploy, no CDN purge, no restart. Old builds stay on the CDN precisely so this is one
command. `tools/release.sh list` shows what is available.

### Moving builds to a different CDN

Each release row carries its own `cdn_base`, so a build can move without a code change or a
deploy. Re-register it against the new one:

```bash
LC_CDN=https://cdn.lc.zanderlowry.com tools/release.sh register <build-id>
```

`CDN_BASE` on the site is only the default for builds served from the fallback path. Changing
that environment variable does **not** move builds already registered, which is the intended
behavior and reliably surprising the first time.

**Set `LC_CDN` when you register.** It defaults to the development CDN, so a build registered
without it carries `http://rocinante.local:3101` no matter what the site is configured with.
Against an https site a browser then blocks the assets as mixed content, and what `/play`
reports is *"the site is pointing at a build that is not on the CDN"* — naming a URL that is
perfectly reachable by hand, which sends you looking at the CDN instead of at the row.
`release.sh register` now refuses an http CDN for an https site outright.

If a build is actively harmful, `tools/release.sh yank <build-id>` marks it unpromotable
without deleting it, so nobody re-promotes it by muscle memory. The bytes stay on the CDN:
anything already running against them keeps working, and deleting the evidence of a bad build
helps nobody. `unyank` reverses it.

Yanking the build a channel points at does not silently fall through to an older one —
`/play` says there is nothing to play. Promotion is a deliberate act and so is undoing one.

---

## Shipping the shelf

```bash
tools/publish-books.sh --check         # the shelf matches its catalogue; needs no CDN
tools/publish-books.sh                 # uploads what the catalogue names, to library/
```

**On its own schedule, and not with a build.** Books live under `library/` rather than
`game/<build-id>/` because a book is not part of a client: re-uploading forty megabytes of epub
with every wasm build would put a copy of Huckleberry Finn under every build id forever, and a
rollback would change which books exist. `tools/build-wasm.sh` stages named directories rather
than the asset root, which is what keeps that true.

Publishing skips a file already on the CDN rather than replacing it, for the reason a build id
is never reused: the old bytes are what every cache and every reader part-way through that book
already has. A title that needs different bytes gets a new file name and a new row in
`crates/lc-client/assets/books/books.toml`.

The check is worth running on its own. It hashes every file the catalogue names and compares it
against the `sha256` recorded there, so a book edited in place — or dropped into the directory
without being cataloged — is caught before anyone is reading it.

A shard is told where the shelf is, and a client is told by its shard:

```bash
lightcone-server --library crates/lc-client/assets/books/books.toml \
                 --shelf-base https://cdn.lightconefrontier.com/library/
```

`--library` without `--shelf-base` refuses to start: a catalogue nobody can fetch from is a list
of titles that do nothing. Neither is a list of books a shard simply has none to lend, which is
the state every test runs in.

---

## Shipping the site

Content and code roll back together, because both are baked into the image.

```bash
SITE_BUILD=$(git rev-parse --short HEAD)
docker --context rocinante build -f web/Dockerfile \
    --build-arg SITE_BUILD="$SITE_BUILD" \
    -t lightcone-web:"$SITE_BUILD" -t lightcone-web:latest web

docker --context rocinante rm -f lightcone-web
docker --context rocinante run -d --name lightcone-web --restart unless-stopped \
    --read-only --cap-drop ALL --security-opt no-new-privileges \
    -p 3100:3100 \
    -e BASE_URL=https://lightcone.example \
    -e CDN_BASE=https://cdn.lightcone.example \
    -e DATABASE_URL="postgres://lc_site@lightcone-db/lc_site" \
    -e FALLBACK_BUILD_ID=<build-id> \
    -e RELEASE_TOKEN="$(cat ~/.lightcone-release-token)" \
    lightcone-web:"$SITE_BUILD"
```

**`--build-arg SITE_BUILD` is not optional.** Without it `build.rs` falls back to the crate
version, which never changes, and every asset URL freezes — and since assets are served
`immutable`, a stylesheet edit would never reach anyone who had already loaded the old one.

To roll back, run the same command with an older tag. The image holds its own content.

---

## TLS

Running today: real Let's Encrypt certificates for `lc.zanderlowry.com` and
`*.lc.zanderlowry.com`, issued over DNS-01, renewing automatically, on a machine with no
inbound connectivity. The sections below are how it got there and how to rebuild it.

**The site and the CDN need certificates on the same day.** Mixed-content rules forbid an
HTTPS page fetching an HTTP subresource, so moving one without the other breaks `/play`
entirely. They are one change.

There is a second, sharper reason: **WebGPU is only exposed in a secure context.** Over plain
HTTP `navigator.gpu` does not exist, so `/play` cannot run at all, however good the browser.
`localhost` counts as secure; a `.local` hostname over HTTP does not.

### Development: an SSH tunnel

**Superseded** by the Let's Encrypt setup below, which is what is running. Kept because it
needs no DNS, no credentials and no certificates, so it is the fallback whenever the proxy is
being changed or the certificate has expired. Both services become
`localhost`, which is a secure context, and they stay on different ports, so cross-origin
behavior is still exercised rather than accidentally bypassed.

```bash
ssh -N -L 3100:localhost:3100 -L 3101:localhost:3101 zandy@rocinante.local
```

Then open `http://localhost:3100`. **The tunnel holds those local ports**, so a local
development server cannot also use 3100 — it will fail to bind, and every request will go to
the deployed container while looking as though it went to the local one. Run a local server on
a different port, or close the tunnel first:

```bash
lsof -nP -iTCP:3100 -sTCP:LISTEN     # says `ssh` when the tunnel has it
``` Set `CDN_BASE=http://localhost:3101` if you want the assets
through the tunnel too; either works, because an HTTP page may fetch HTTP subresources.

No certificates, nothing to trust, nothing to expire.

### Development for someone else: Caddy's internal CA

When a second person needs to open it and a tunnel is too much to ask, Caddy can issue its own
certificates. The cost is that every client machine has to trust the root.

```caddyfile
{
    local_certs
}

rocinante.local:443 {
    reverse_proxy lightcone-web:3100
}
```

```bash
docker --context rocinante cp lightcone-proxy:/data/caddy/pki/authorities/local/root.crt .
# macOS, on each client:
sudo security add-trusted-cert -d -r trustRoot -k /Library/Keychains/System.keychain root.crt
```

Prefer the tunnel. A root certificate installed on a laptop outlives the reason it was
installed.

### Real certificates on the LAN: Let's Encrypt over DNS-01

**This is the setup to use.** It gives rocinante a publicly-trusted certificate without
exposing it to the internet, which removes the tunnel and makes `/play` work from any device
at home.

The trick is the challenge type. HTTP-01 and TLS-ALPN-01 both require Let's Encrypt to reach
the machine; **DNS-01 requires only the ability to write a TXT record**. Caddy writes
`_acme-challenge`, Let's Encrypt reads it, Caddy deletes it, and the same happens at renewal
without anyone being involved. Nothing inbound, ever.

The A records can then point at **10.37.1.100**. Let's Encrypt does not look at where a name
resolves when the challenge is DNS-01, so a public name may resolve to a private address.

`tools/proxy/` holds the image: Caddy rebuilt with `xcaddy` to include one DNS provider
module, because the stock image has none.

#### 1. Namecheap API access — check this first

Namecheap gates API access behind account activity. The account must have **20+ domains, or
$50+ balance, or $50+ spent in the last two years**, and the domain must be on Namecheap's own
BasicDNS rather than external nameservers.

If that gate does not open, do not fight it. Delegate the subdomain instead: create `NS`
records for `lc.<domain>` pointing at a free DNS host with a usable API (Cloudflare and deSEC
both qualify), and rebuild the proxy image `--with github.com/caddy-dns/cloudflare`. The
registration stays at Namecheap and only that subdomain's DNS moves. Nothing else in this
setup changes.

Then, in the Namecheap dashboard:

- **Profile → Tools → Namecheap API Access** — switch it on, and copy the API key.
- **Whitelisted IPs** — add the address Namecheap will see the requests coming *from*. That is
  the machine's **public egress address**, not its LAN address. Today both rocinante and the
  laptop leave through `108.242.43.159`.

Getting that IP wrong is the single most common failure, and it presents as an authentication
error rather than as anything about addresses.

#### 2. DNS records

| host | type | value |
|---|---|---|
| `lc` | A | `10.37.1.100` |
| `cdn.lc` | A | `10.37.1.100` |

A wildcard `*.lc` A record works too if Namecheap accepts a nested wildcard, and saves adding
a row per service. Explicit records always work, so start there.

#### 3. Run it

Credentials live in `~/.config/lightcone/proxy.env` **on rocinante**:

```
LC_DOMAIN=lc.zanderlowry.com
NAMECHEAP_CLIENT_IP=108.242.43.159
ACME_CA=https://acme-v02.api.letsencrypt.org/directory
ACME_EMAIL=...
NAMECHEAP_USER=...
NAMECHEAP_API_KEY=...
```

```bash
docker --context rocinante build -t lightcone-proxy:latest tools/proxy

# Over ssh, because --env-file is read by the CLI you invoke and that file is on rocinante.
ssh zandy@rocinante.local '
docker volume create lightcone-proxy-data
docker rm -f lightcone-proxy
docker run -d --name lightcone-proxy --restart unless-stopped \
    --network lightcone -p 80:80 -p 443:443 \
    -v lightcone-proxy-data:/data \
    --env-file ~/.config/lightcone/proxy.env \
    lightcone-proxy:latest'
```

Watch the challenge:

```bash
docker --context rocinante logs -f lightcone-proxy 2>&1 | grep -i 'challenge\|obtained\|error'
```

**Get a staging certificate first.** `ACME_CA` is pre-set to Let's Encrypt's staging endpoint
in the template above for that reason: it proves the DNS credentials work without spending a
real issuance, and a wrong `NAMECHEAP_CLIENT_IP` will otherwise burn several in a retry loop.
Once staging says `certificate obtained successfully` for both names, point `ACME_CA` at
production and restart. The production certificate issues in seconds.

**The `/data` volume is not optional.** Caddy keeps issued certificates there, and without it
every container restart asks Let's Encrypt for a new one. The rate limit is 50 certificates
per registered domain per week and it is reachable in an afternoon of restarts. If you do hit
it, the limit is per week and waiting is the only remedy — so add `acme_ca` pointing at
Let's Encrypt's **staging** endpoint while getting the configuration right, and remove it once
a staging certificate is issued successfully.

Then redeploy the site with the HTTPS origins, because feeds, the sitemap and the loader all
build absolute URLs from them:

```bash
-e BASE_URL=https://lc.<domain>
-e CDN_BASE=https://cdn.lc.<domain>
```

#### 4. Point the site and the CDN at the new names

Three things, and the first two are one deploy:

```bash
-e BASE_URL=https://lc.zanderlowry.com        # feeds and the sitemap build absolute URLs
-e CDN_BASE=https://cdn.lc.zanderlowry.com    # the default for the fallback path
```

```bash
-e CDN_ALLOW_ORIGIN=https://lc.zanderlowry.com   # on lightcone-cdn; `*` was a dev convenience
```

And re-register every build, because a release row carries its own `cdn_base` and an
environment variable does not reach it:

```bash
LC_CDN=https://cdn.lc.zanderlowry.com tools/release.sh register <build-id>
```

Miss the last one and `/play` reports "Build not found" while pointing at an `http://` URL —
which is the mixed-content rule doing its job, and reads like the CDN being down.

#### 5. What changes the moment it works

- **`/play` runs without a tunnel**, from any device on the network, because a secure context
  is what WebGPU requires.
- **Brotli starts being used.** Chrome only advertises `br` over a secure connection, so
  everything so far has been gzip — about 9 MB rather than 6.4. This is the first time the
  `.br` files are read at all, which makes it the first time a mistake in them could show.
- **The CDN's `Access-Control-Allow-Origin` should stop being `*`** and become
  `https://lc.<domain>`. The wildcard is a development convenience for serving several local
  origins and should not outlive them.
- The hostname appears in public Certificate Transparency logs. Not a vulnerability, and the
  wildcard means only `lc.<domain>` is published rather than every service under it.

### Production: a real domain, and Caddy in front of both

One Caddy terminates TLS for the site and the CDN, gets certificates from Let's Encrypt on its
own, and renews them without being asked. It needs ports 80 and 443, and DNS pointing at it.

```caddyfile
lightcone.example {
    reverse_proxy lightcone-web:3100
}

cdn.lightcone.example {
    root * /srv/cdn
    import cdn-headers          # the policy from tools/dev-cdn/Caddyfile, unchanged
    file_server {
        precompressed br gzip
    }
}
```

Two things to get right at the same time:

- **`Access-Control-Allow-Origin` becomes the site's origin**, not `*`. The wildcard is a
  development convenience for serving several local origins and should not survive.
- **Brotli starts being used.** Chrome only advertises `br` over a secure connection, so
  everything to date has been served gzip — around 9 MB rather than 6.4. The first HTTPS
  deploy is also the first time the `.br` files are read, which makes it the first time a
  mistake in them would show.

Set `BASE_URL` and `CDN_BASE` to the `https://` origins in the same deploy.

---

## The database

```bash
docker --context rocinante network create lightcone
docker --context rocinante run -d --name lightcone-db --restart unless-stopped \
    --network lightcone \
    -e POSTGRES_USER=lc_site -e POSTGRES_PASSWORD=<password> -e POSTGRES_DB=lc_site \
    -v lightcone-db-data:/var/lib/postgresql/data \
    -p 3102:5432 postgres:17-bookworm
```

Its own database and its own role. The site's credentials must not reach `lc_game`: two
readers of one schema is how a game migration starts breaking a marketing page.

**The site reaches it by container name on the `lightcone` network**, not over the LAN —
`postgres://lc_site@lightcone-db:5432/lc_site`. The published port is a development
convenience for `psql` and nothing else.

Connecting to it from a laptop has a trap worth knowing about. `rocinante.local` is mDNS, and
mDNS answers with a **link-local IPv6 address first** (`fe80::…`), which needs a scope id that
a connection string has nowhere to put. `psql` tries it, fails, and falls back to IPv4; sqlx
does not, and reports `pool timed out while waiting for an open connection` — which reads like
the database being slow or down rather than unreachable at that address. Use the IPv4 address
for a laptop connection, or an SSH tunnel.

Migrations run at startup, under a PostgreSQL advisory lock that sqlx takes for itself, so
this stays correct with more than one replica.

**The site is designed to run without it.** The blog is on disk, `/healthz` touches nothing,
and `/play` falls back to `FALLBACK_BUILD_ID` when the channel lookup fails. Losing the
database costs view counts and release management, not the site — which is worth testing
deliberately now and then:

```bash
docker --context rocinante stop lightcone-db
curl -sS -o /dev/null -w '%{http_code}\n' http://localhost:3100/play    # still 200
docker --context rocinante start lightcone-db
```

---

## The broker and a shard, on rocinante

Three containers beside the site, all on the `lightcone` network.

```bash
# Secrets, generated on the host so they never pass through a laptop's shell history.
ssh zandy@rocinante.local
cd ~/.config/lightcone && umask 077
rand() { head -c 32 /dev/urandom | base64 | tr -d '\n=' | tr '+/' '-_'; }
# identity.env      — DATABASE_URL, LC_IDENTITY_* (see config.rs for the full list)
# site-identity.env — LC_IDENTITY_BASE, LC_IDENTITY_API, LC_IDENTITY_SECRET,
#                     SITE_SESSION_KEY, LC_SHARD, LC_SHARD_URL
# admin.env         — DATABASE_URL (the *broker's*), LC_ADMIN_* (see lc-admin/src/config.rs)
```

`LC_IDENTITY_EXCHANGE_SECRET` on the broker, `LC_IDENTITY_SECRET` on the site and
`LC_ADMIN_IDENTITY_SECRET` on the console are the **same value**. `LC_IDENTITY_SIGNING_SEED`
must be set, or every restart publishes a new key and every ticket minted before it stops
verifying.

Two things about the console are easy to get wrong and fail late rather than at boot:

- `LC_ADMIN_PUBLIC_URL` builds its return URL, and that URL must appear **verbatim** in the
  broker's `LC_IDENTITY_RETURN_TO`. Missing from the allowlist, it fails at the *end* of a
  sign-in rather than at the start. The console logs the return URL it will use on startup so
  the two can be compared without guessing.
- `LC_ADMIN_SESSION_KEY` is its own secret, not the site's, and must be at least 32 characters
  — refused at boot if it is shorter, because a forgeable cookie here is an administrator
  account.

Its own role and database, like the site's. **The console shares them** — it reads and writes
the broker's tables and owns no schema of its own, so it gets the same `DATABASE_URL`:

```sql
CREATE ROLE lc_identity LOGIN PASSWORD '...';
CREATE DATABASE lc_identity OWNER lc_identity;
```

### Deploying the console over an existing stack

Three things in this order, and the order is the whole of it.

**1. The broker first, and with the console's return URL already on its allowlist.** The
console does not migrate; `lc-identity` applies the schema at its own boot. Deploying the
console against a broker that predates `0005_bans.sql` gives a console whose every page is a
500, because `bans` does not exist. Back the database up first — the migrations are forward
only, and `0004` adds a check constraint that a hand-written `permission` outside 0–3 would
fail on:

```bash
ssh zandy@rocinante.local '
  url=$(grep -m1 ^DATABASE_URL= ~/.config/lightcone/identity.env | cut -d= -f2-)
  docker run --rm --network lightcone -e PGURL="$url" postgres:17-bookworm \
    sh -c '"'"'pg_dump "$PGURL"'"'"' > ~/backups/lc_identity-$(date +%Y%m%d-%H%M%S).sql'
```

`--env-file` is read at `docker run` and baked into the container, so an edit to
`identity.env` needs the container **recreated**. A `docker restart` silently keeps the old
values, and the symptom is a sign-in refused as `Bad sign-in request` from a broker whose
configuration file plainly lists the address.

**2. `admin.env`, written on the host.** `LC_ADMIN_IDENTITY_SECRET` is the broker's
`LC_IDENTITY_EXCHANGE_SECRET` and `DATABASE_URL` is the broker's, so both are copied out of
`identity.env` rather than retyped. `LC_ADMIN_IDENTITY_API` is
`http://lightcone-identity:3200` — **not** the public name, for the hairpin reason above.

**3. The proxy last**, once there is something behind the route to answer.

For the Status section on a user page, the shard needs `--admin-bind 0.0.0.0:3400` and the
console needs `LC_ADMIN_SHARD_API=http://lightcone-shard:3400` — the container name, never the
public one, and **no proxy route**: that port is reached from inside the network and from
nowhere else. `LC_ADMIN_SHARD_AUDIENCE` must match the shard's `--audience` and must be on the
broker's `LC_IDENTITY_AUDIENCES`, or no ticket can be minted for it and every card reads "the
shard did not answer". The shard refuses `--admin-bind` without both `--db` and `--jwks`
rather than listening and turning every request away.

Then check the whole path rather than the container: `/users` signed out must be a 303 to the
console's own `/signin`, and that must be a 303 to the broker carrying `return_to` — and the
broker must render a form for it rather than `Bad sign-in request`, which is what an address
missing from the allowlist looks like.

### The first administrator

`0003_permissions.sql` promoted one account by address, once, against data that existed. After
that, levels are handed out from the console — and the console needs somebody to be an
administrator before anyone can reach it, which is the chicken and egg. Break it by hand:

```sql
UPDATE accounts SET permission = 1 WHERE id = (
  SELECT account_id FROM links WHERE email = '...' LIMIT 1);
```

This is also the only way to **remove** an owner. That is deliberate, not an oversight: nobody
outranks a level 1, so the one irreversible administrative act is not reachable from a web
page. See [16-identity.md](16-identity.md).

```bash
docker --context rocinante build -f auth/Dockerfile -t lightcone-identity:<tag> auth
# Same build context, different Dockerfile: the console shares the broker's workspace and
# lockfile, and adds a node stage that compiles its TypeScript.
docker --context rocinante build -f auth/Dockerfile.admin -t lightcone-admin:<tag> auth
docker --context rocinante build -f crates/lc-server/Dockerfile -t lightcone-shard:<tag> .

# Over ssh, because --env-file is read by the CLI you invoke and that file is on rocinante.
ssh zandy@rocinante.local '
  docker rm -f lightcone-identity 2>/dev/null
  docker run -d --name lightcone-identity --restart unless-stopped --network lightcone \
      --env-file ~/.config/lightcone/identity.env lightcone-identity:<tag>
  # After the broker, always: the console does not migrate, so it needs the schema the
  # broker applies at its own boot to be there already.
  docker rm -f lightcone-admin 2>/dev/null
  docker run -d --name lightcone-admin --restart unless-stopped --network lightcone \
      --env-file ~/.config/lightcone/admin.env lightcone-admin:<tag>'

docker --context rocinante run -d --name lightcone-shard --restart unless-stopped \
    --network lightcone --env-file ~/.config/lightcone/shard.env lightcone-shard:<tag> \
    --bind 0.0.0.0:8080 --audience shard-1 \
    --jwks http://lightcone-identity:3200/.well-known/jwks.json \
    --sky http://lightcone-cdn:3101/game/<promoted-build>/assets/sky/catalogue.lcsky
```

### The shard's own database

**A shard without `--db` is a sandcastle.** It runs, and every craft in it — and every
account's claim on one — is gone when it stops. Give it a role and a database like the
others:

```sql
CREATE ROLE lc_store LOGIN PASSWORD '...';
CREATE DATABASE lc_store OWNER lc_store;
```

The URL goes in `~/.config/lightcone/shard.env` as `LC_SHARD_DB`, which the shard reads when no
`--db` is given. The environment and not the command line because it carries a password, and an
argument is visible to anything that can list processes. The schema is applied at boot, so there
is no migration step to remember.

What is kept is a **checkpoint**, not a history: every craft as of an instant, plus the shard's
clock and its identifier counter. Written every twenty real seconds and again on the way out,
`docker stop` included — the shard traps `SIGTERM`. A crash loses at most twenty seconds, and
what it loses is *orders*, not flight: every motive is stamped in absolute coordinate time, so
a checkpoint replayed forward puts a ship exactly where it would have been anyway.

The clock is the part that is easy to leave out and impossible to do without. Every motive
carries an absolute coordinate time, so a shard that came back at zero would read every saved
craft as one whose crossing has not begun, and fly them all again from decades in the past.

A restart resumes the clock where it stopped rather than advancing it by however long the
process was down. The alternative asserts that things happened in the missing time, when
nothing was journalled and nobody was told.

**Point `--sky` at the promoted build's own chunk**, by the CDN's *container* name —
`http://lightcone-cdn:3101/game/<build>/assets/sky/catalogue.lcsky`. Not the public
`https://cdn.…` name: that resolves to the host's own address and hairpins, exactly as the
site's call to the broker does. Same container, same bytes, different route. Both ends place craft into systems by position
against the same shell radius, so two catalogues is two answers to which system a ship is in —
and nothing anywhere reports the disagreement. Reading the same bytes the client downloads is
what makes them agreeing a fact rather than a convention somebody has to keep. A shard started
without it says so loudly and falls back to three hand-written stars, which is right only for a
shard no real client connects to.

The shard reads the broker's keys **at boot**, so the broker has to be up first. It then
verifies locally and never asks again, which is the point — a broker outage does not stop
anyone reconnecting. A shard that restarts while the broker is down will not start, and
`--restart unless-stopped` retries until it can.

### Two addresses for the broker, and why

`LC_IDENTITY_BASE` is where the **browser** is sent: `https://accounts.lc.zanderlowry.com`.
`LC_IDENTITY_API` is where the **site** calls `/exchange` and `/ticket`:
`http://lightcone-identity:3200`.

They must differ in a container deployment and it is not obvious why. The public name resolves
to the host's own LAN address, and a container reaching the host's published port by that
address hairpins through the NAT and hangs. The symptom is a **504 on `/auth/return` with
nothing in any log**, because nothing arrived anywhere. `LC_IDENTITY_API` is unset everywhere
else and falls back to the public name.

### Routes

The proxy gains three. Reload rather than recreate — the container holds the ACME account and
the certificates, and a restart it did not need is a restart that can go wrong:

```bash
docker --context rocinante cp tools/proxy/Caddyfile lightcone-proxy:/etc/caddy/Caddyfile
ssh zandy@rocinante.local \
  'docker exec lightcone-proxy caddy reload --config /etc/caddy/Caddyfile --adapter caddyfile'
```

`accounts.` and `admin.` are host routes and each needs its own A record, pointed at the same
LAN address as everything else. Neither needs a certificate of its own: the block asks for
`*.{$LC_DOMAIN}`, so a new service here is a DNS record and a `handle` and nothing more, which
is what the wildcard was chosen for. Validate before reloading, because a reload of a bad file
is a proxy that stops answering for every name at once:

```bash
docker --context rocinante cp tools/proxy/Caddyfile lightcone-proxy:/tmp/Caddyfile.candidate
ssh zandy@rocinante.local 'docker exec -e LC_DOMAIN=dev.lightconefrontier.com lightcone-proxy \
  caddy validate --config /tmp/Caddyfile.candidate --adapter caddyfile'
```

The shard is a **path** — `/ws` — which
needs none, and a WebSocket upgrade proxies cleanly either way. It must be `wss://`: a page
served over TLS may only open a secure socket, and a browser refuses the other outright.

Testing that upgrade by hand needs `--http1.1`. Without it curl negotiates HTTP/2, where a
WebSocket is Extended CONNECT rather than an `Upgrade:` header, and Caddy answers **502** —
which looks exactly like the shard being unreachable and is not.

## A shard and a client, locally

One process, for tuning game mechanics against the real server:

```bash
cargo run -p lc-client --bin lightcone -- --local
```

`--local` runs the same `lc-server` a shard runs, on a loopback socket, in this process — the
light-delay gate, the intent clamps, the tick, all of it. Not a simulation of a server and not a
second implementation. It is given the stars the client itself loaded, because both ends put
craft into systems by position and two different skies would disagree about which system a ship
is in.

Running with **no** `--server` and no `--local` is the single-process game: local fleet, no
socket, and the client applies its own flight orders. That is what every build before the seam
did, and it still works — but it is not what a deployment does, so a mechanic tuned there is
tuned against something else.

The smallest thing that is actually two processes:

```bash
cargo run -p lc-server --bin lightcone-server -- --bind 127.0.0.1:8080 --open
cargo run -p lc-client --bin lightcone -- --server ws://127.0.0.1:8080 --observe
```

`--open` admits a connection with no valid ticket. It is the same decision the site makes when
no broker is configured, and it is development only for two reasons: it lets anyone in, and an
anonymous player is keyed by their connection, so every reconnection is a new ship.

With a broker, point the shard at its published keys instead. A **file** is as good as a URL
and is how a shard starts when the broker is down — it verifies locally and never asks per
connection:

```bash
curl -s http://127.0.0.1:3210/.well-known/jwks.json > /tmp/shard.jwks
cargo run -p lc-server --bin lightcone-server -- --jwks /tmp/shard.jwks --audience shard-1
```

### The browser build

`/play` hands the client two things on `#boot`: the ticket, and `data-server` — the shard's
address, from `LC_SHARD_URL`. Both come off the page rather than the query string. The ticket
because a credential in a URL is a credential in history, in an access log and in a `Referer`;
the address because which shard a build talks to is part of the handover rather than something
a player types. A `?server=` flag still wins, which is how a build gets pointed at a shard
nobody has deployed yet.

`LC_SHARD_URL` is an address and `LC_SHARD` is an audience name. They are separate on purpose:
one is what the broker and the shard agree a ticket is *for*, the other is where the shard
happens to be, and moving it should not reissue anything.

The sky is not handed over at all. The client loads `sky/catalogue.lcsky` from its build's asset
base, so a build served from anywhere — the CDN, or a staged directory on disk — draws the
catalogue it was built with.

A page served over TLS may only open `wss://`. A browser refuses `ws://` from an `https://`
origin outright, which is the one failure here that looks like the server being down.

A shard given neither refuses to start. Starting without either would mean refusing every
connection, which from the outside is indistinguishable from everyone's credentials being wrong
at once.

The HUD says which of these happened: `LINKED <name>` on a welcome, `REFUSED — <why>` on a
ticket the shard would not take. The words carry it and the color only agrees — see
[18-ui-style.md](18-ui-style.md).

## Checks

```bash
curl -sS localhost:3100/healthz                    # liveness, no dependencies
curl -sS localhost:3100/readyz                     # readiness, pings the pool
curl -sSI localhost:3100/ | grep x-lightcone-build # which site build is up
tools/release.sh list                              # which game build is promoted
```

The header and `tools/release.sh list` answer different questions. The site's build id versions
its own CSS; the game's versions the wasm on the CDN. They move on unrelated schedules and
nothing should ever compare them.

### When something is wrong

| symptom | look at |
|---|---|
| `/play` says "Needs a secure connection" | the page is on plain HTTP. Tunnel, or TLS |
| `/play` says "Build not found" | the promoted build id is not on the CDN. `tools/release.sh list`, then `publish-build.sh` |
| the client starts and the sky is three stars | `assets/sky/catalogue.lcsky` 404'd under the build; the client says so and falls back to a sample |
| thousands of requests for one asset | a wrong asset base. Bevy retries a failed load without bound |
| a stylesheet change does not appear | the asset version did not move, or the tunnel is serving you the deployed container |
| `pool timed out while waiting for an open connection` | mDNS handed out a link-local IPv6. Use the IPv4 address |
| the local server "starts" but serves old code | it failed to bind and something else has the port. `lsof -nP -iTCP:<port> -sTCP:LISTEN` |
| `pull access denied for lightcone-web` | the image tag for that commit was never built. Build and run in one sequence, not two |
| `/play` says "Build not found" over HTTPS | the release row still carries an `http://` `cdn_base`. Re-register it |
| certificate renewal fails silently | the Namecheap allowlist no longer has this network's public IP. It changes when the ISP reassigns |
| the whole 27 MB downloads | no `.br`/`.gz` beside the file, or the connection is not secure so `br` was never asked for |
