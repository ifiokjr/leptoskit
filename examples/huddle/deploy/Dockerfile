# syntax=docker/dockerfile:1
#
# The container for huddle, written by `leptoskit generate deploy`. It's yours
# to edit: running the generator again keeps your changes unless you pass
# --force.
#
# Huddle lives inside the leptoskit repository and builds against its crates,
# so three lines differ from what the generator writes for a project of its
# own: the build context is the repository root, the CLI is installed from
# the same checkout, and the build runs in examples/huddle. Build it from the
# repository root:
#
#   docker build -f examples/huddle/deploy/Dockerfile -t huddle .
#
# The first stage runs `leptoskit build --release`, as you would on your
# machine. The second runs the server as an unprivileged user that writes
# nothing to the image's filesystem, so it works with a read-only root
# filesystem.
#
# The SQLite database is the one thing it writes, on a volume at /data.

# The Rust release that builds the app. `1` is the latest stable; pin one,
# such as 1.96, to build the same way every time.
ARG RUST_VERSION=1
# binaryen's wasm-opt shrinks the release client.
ARG BINARYEN_VERSION=132

FROM rust:${RUST_VERSION}-bookworm AS build
ARG BINARYEN_VERSION
RUN curl --fail --silent --show-error --location \
      "https://github.com/WebAssembly/binaryen/releases/download/version_${BINARYEN_VERSION}/binaryen-version_${BINARYEN_VERSION}-$(uname -m)-linux.tar.gz" \
    | tar -xz -C /opt \
 && ln -s "/opt/binaryen-version_${BINARYEN_VERSION}/bin/wasm-opt" /usr/local/bin/wasm-opt
WORKDIR /src
RUN rustup target add wasm32-unknown-unknown
COPY . .
# The leptoskit CLI from this checkout, so it matches the crates the app
# builds against.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo install --path crates/leptoskit_cli --locked
# The cache mounts keep downloaded crates and compiled dependencies between
# builds. The server is copied out, since the target directory is a cache.
WORKDIR /src/examples/huddle
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    --mount=type=cache,target=/src/examples/huddle/.leptoskit \
    leptoskit build --release --platform web \
 && cp /src/target/release/huddle /usr/local/bin/huddle

FROM debian:bookworm-slim
# curl answers the health check, and `app` is the unprivileged user the
# server runs as.
RUN apt-get update \
 && apt-get install --yes --no-install-recommends curl \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --user-group --uid 10001 --no-create-home app \
 && mkdir /data \
 && chown app:app /data
WORKDIR /app
COPY --from=build /usr/local/bin/huddle /app/server
COPY --from=build /src/examples/huddle/dist /app/dist
# What `leptoskit serve` sets from leptoskit.toml, for a container: listen on
# every interface, and serve the site from /app/dist.
ENV LEPTOS_ENV=PROD \
    LEPTOS_OUTPUT_NAME=huddle_app \
    LEPTOS_SITE_ROOT=/app/dist \
    LEPTOS_SITE_PKG_DIR=pkg \
    LEPTOS_SITE_ADDR=0.0.0.0:3000
# The SQLite database lives on a volume, the one place the server writes.
ENV DATABASE_URL=sqlite:///data/huddle.sqlite3
# The demo keeps every email, sign-in codes included, in the mailbox at
# /_leptoskit/mail, where anyone can read it: that's how strangers sign in to
# a public demo. Don't keep real people's data in it.
ENV HUDDLE_DEMO_MAIL=1
VOLUME /data
# These secrets come from the environment when the container starts, never
# from this file:
#   AUTH_ENCRYPTION_KEYS
USER app
EXPOSE 3000
HEALTHCHECK --interval=10s --timeout=3s --start-period=30s --retries=3 \
  CMD ["curl", "--fail", "--silent", "--output", "/dev/null", "http://127.0.0.1:3000/healthz"]
CMD ["/app/server"]
