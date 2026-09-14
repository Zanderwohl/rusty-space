# The development CDN: Caddy plus a config file. Built on the `rocinante` docker context.
#
# Content is NOT baked in. Builds are published into a volume at /srv/cdn, so the container
# accumulates them the way a bucket does, and rebuilding the image never loses a build.
FROM caddy:2-alpine
COPY Caddyfile /etc/caddy/Caddyfile
EXPOSE 3101
