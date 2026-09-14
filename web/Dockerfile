# Built on the `rocinante` docker context, which is natively linux/amd64. Building amd64 on
# an Apple Silicon laptop means qemu, which for a Rust release build is the difference between
# minutes and most of an afternoon. See lightcone/docs/14-hosting.md.
#
#   docker --context rocinante build -f web/Dockerfile \
#       --build-arg SITE_BUILD=$(git rev-parse --short HEAD) -t lightcone-web:<tag> web

FROM rust:1-bookworm AS chef
WORKDIR /build
RUN cargo install cargo-chef --locked

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /build/recipe.json recipe.json
# Dependencies are their own layer, so a source-only change rebuilds just this crate.
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
ARG SITE_BUILD=""
ENV SITE_BUILD=$SITE_BUILD
RUN cargo build --release --bin lc-web

# distroless/cc: the binary is dynamically linked against glibc, and this carries that and
# nothing else. No shell, so nothing in the image can be talked into running anything.
FROM gcr.io/distroless/cc-debian12
WORKDIR /app
COPY --from=builder /build/target/release/lc-web /app/lc-web
# The SCSS source ships too; it is a few kilobytes and it is compiled into memory at boot,
# because the root filesystem is read-only and cannot be written beside its source.
COPY static  /app/static
COPY content /app/content
ENV BIND_ADDR=0.0.0.0:3100 SITE_ENV=production
EXPOSE 3100
USER nonroot
ENTRYPOINT ["/app/lc-web"]
