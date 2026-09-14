# Runbook

How to build, publish, deploy and roll back. [14-hosting.md](14-hosting.md) is why any of it
is shaped this way; this is the sequence of commands.

Everything runs on **`rocinante`**, a docker context over SSH to a home-lab machine that is
natively `linux/amd64`. Ports 3000–3999 are this project's; 3000 belongs to another project.

| | port | what |
|---|---|---|
| `lightcone-web` | 3100 | the site |
| `lightcone-cdn` | 3101 | game builds |
| `lightcone-db` | 3102 | the site's PostgreSQL |

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

`build-wasm.sh` refuses nothing but takes about four minutes cold. `publish-build.sh` refuses a
`-dirty` build id: a build that exists on one laptop is not something anyone can roll back to.

### Rolling back

```bash
tools/release.sh promote <older-build-id>
```

No deploy, no CDN purge, no restart. Old builds stay on the CDN precisely so this is one
command. `tools/release.sh list` shows what is available.

If a build is actively harmful, `tools/release.sh yank <build-id>` marks it unpromotable
without deleting it, so nobody re-promotes it by muscle memory. The bytes stay on the CDN:
anything already running against them keeps working, and deleting the evidence of a bad build
helps nobody. `unyank` reverses it.

Yanking the build a channel points at does not silently fall through to an older one —
`/play` says there is nothing to play. Promotion is a deliberate act and so is undoing one.

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

**The site and the CDN need certificates on the same day.** Mixed-content rules forbid an
HTTPS page fetching an HTTP subresource, so moving one without the other breaks `/play`
entirely. They are one change.

There is a second, sharper reason: **WebGPU is only exposed in a secure context.** Over plain
HTTP `navigator.gpu` does not exist, so `/play` cannot run at all, however good the browser.
`localhost` counts as secure; a `.local` hostname over HTTP does not.

### Development: an SSH tunnel

The cheapest correct answer, and what to use until there is a domain. Both services become
`localhost`, which is a secure context, and they stay on different ports, so cross-origin
behaviour is still exercised rather than accidentally bypassed.

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

```bash
docker --context rocinante build -t lightcone-proxy:latest tools/proxy
docker --context rocinante volume create lightcone-proxy-data

docker --context rocinante run -d --name lightcone-proxy --restart unless-stopped \
    --network lightcone -p 80:80 -p 443:443 \
    -v lightcone-proxy-data:/data \
    -e LC_DOMAIN=lc.<domain> \
    -e ACME_EMAIL=<you@example.com> \
    -e NAMECHEAP_USER=<namecheap username> \
    -e NAMECHEAP_API_KEY=<api key> \
    -e NAMECHEAP_CLIENT_IP=108.242.43.159 \
    lightcone-proxy:latest
```

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

#### 4. What changes the moment it works

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
| the client starts and the sky is three stars | the sky chunk 404'd; the client says so and falls back to a sample |
| thousands of requests for one asset | a wrong asset base. Bevy retries a failed load without bound |
| a stylesheet change does not appear | the asset version did not move, or the tunnel is serving you the deployed container |
| `pool timed out while waiting for an open connection` | mDNS handed out a link-local IPv6. Use the IPv4 address |
| the local server "starts" but serves old code | it failed to bind and something else has the port. `lsof -nP -iTCP:<port> -sTCP:LISTEN` |
| the whole 27 MB downloads | no `.br`/`.gz` beside the file, or the connection is not secure so `br` was never asked for |
