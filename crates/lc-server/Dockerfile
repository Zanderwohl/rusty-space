# One shard, built the same way the site and the broker are.
#
#   docker --context rocinante build -f crates/lc-server/Dockerfile -t lightcone-shard:<tag> .
#
# The build context is the repository root, because `lc-server` sits in the game workspace and
# shares `lc-proto`, `lc-world` and `lc-spacetime` with the client. That is also why this one
# Dockerfile cannot live beside the crate the way `auth/`'s and `web/`'s do: those are their own
# workspaces, and this is not.

FROM rust:1-bookworm AS chef
WORKDIR /build
RUN cargo install cargo-chef --locked

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /build/recipe.json recipe.json
# Only this binary's dependencies. The workspace also holds a Bevy client, and cooking the
# whole thing would build a renderer nothing in this image runs.
# `-p` as well as `--bin`: the workspace root package is `exotic-matters`, and a bare `--bin`
# is looked for in the default-run package rather than across the workspace.
RUN cargo chef cook --release --recipe-path recipe.json -p lc-server --bin lightcone-server
COPY . .
RUN cargo build --release -p lc-server --bin lightcone-server

# distroless/cc: dynamically linked against glibc and nothing else. No shell, so there is
# nothing in the image that can be talked into running anything.
FROM gcr.io/distroless/cc-debian12
WORKDIR /app
COPY --from=builder /build/target/release/lightcone-server /app/lightcone-server
# The catalogue, which a shard is told with `--library`. Not the books: those are on the CDN,
# and a shard that carried forty megabytes of epub it never reads would be carrying them to
# tell clients about files it does not serve. See lightcone/docs/21-library.md.
COPY --from=builder /build/crates/lc-client/assets/books/books.toml /app/books.toml
EXPOSE 8080
USER nonroot
ENTRYPOINT ["/app/lightcone-server"]
