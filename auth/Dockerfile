# Built the same way the site is, on the `rocinante` context. See lightcone/docs/16-identity.md.
#
#   docker --context rocinante build -f auth/Dockerfile -t lightcone-identity:<tag> auth

FROM rust:1-bookworm AS chef
WORKDIR /build
RUN cargo install cargo-chef --locked

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /build/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
RUN cargo build --release --bin lc-identity

FROM gcr.io/distroless/cc-debian12
WORKDIR /app
COPY --from=builder /build/target/release/lc-identity /app/lc-identity
# Migrations ship with the binary and are applied at boot: the schema and the code that assumes
# it are one artifact, which is the only way a rollback rolls back both.
COPY lc-identity/migrations /app/migrations
ENV BIND_ADDR=0.0.0.0:3200
EXPOSE 3200
USER nonroot
ENTRYPOINT ["/app/lc-identity"]
