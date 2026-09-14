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
tools/build-wasm.sh                   # stages target/web/<build-id>/
tools/publish-build.sh                # uploads the most recent staged build
tools/release.sh register <build-id>  # tells the site the build exists
tools/release.sh promote <build-id>   # points a channel at it
```

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
without deleting it, so nobody re-promotes it by muscle memory.

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

Then open `http://localhost:3100`. Set `CDN_BASE=http://localhost:3101` if you want the assets
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
docker --context rocinante run -d --name lightcone-db --restart unless-stopped \
    -e POSTGRES_USER=lc_site -e POSTGRES_PASSWORD=<password> -e POSTGRES_DB=lc_site \
    -v lightcone-db-data:/var/lib/postgresql/data \
    -p 3102:5432 postgres:17-bookworm
```

Its own database and its own role. The site's credentials must not reach `lc_game`: two
readers of one schema is how a game migration starts breaking a marketing page.

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
| a stylesheet change does not appear | the site build id did not move. Check `x-lightcone-build` against `git rev-parse --short HEAD` |
| the whole 27 MB downloads | no `.br`/`.gz` beside the file, or the connection is not secure so `br` was never asked for |
