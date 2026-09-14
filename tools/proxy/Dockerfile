# TLS termination for the site and the CDN.
#
# Caddy's stock image has no DNS provider plugins, and DNS-01 is the only ACME challenge that
# works for a machine with no inbound connectivity — which is the whole point here. So the
# binary is rebuilt with one module compiled in.
#
# Pin the plugin. A DNS module that changes under you takes certificate renewal with it, and
# renewal failures are quiet until the certificate expires.
FROM caddy:2-builder-alpine AS builder
RUN xcaddy build \
    --with github.com/caddy-dns/namecheap

FROM caddy:2-alpine
COPY --from=builder /usr/bin/caddy /usr/bin/caddy
COPY Caddyfile /etc/caddy/Caddyfile
EXPOSE 80 443
