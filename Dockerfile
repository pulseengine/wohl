# syntax=docker/dockerfile:1.7
#
# Wohl Hub — multi-stage container build.
#
# Build context: the *parent directory of `wohl/`*, not `wohl/` itself.
# That's because the workspace has path-dependencies like
#   relay-lc = { path = "../relay/crates/relay-lc" }
# Building from inside `wohl/` would put the relay sibling outside the
# context and Docker would refuse to read it. The pattern matches
# .github/workflows/release.yml, which checks out wohl/ and relay/ as
# siblings before invoking cargo.
#
# Typical local invocation (from a directory that contains BOTH wohl/
# and relay/ as subdirs):
#
#   docker build -f wohl/Dockerfile -t wohl-hub:dev .
#
# Or, equivalently, with a configurable layout:
#
#   docker build \
#     --build-arg WOHL_PATH=wohl \
#     --build-arg RELAY_PATH=relay \
#     -f wohl/Dockerfile -t wohl-hub:dev .
#
# CI publishing of an OCI image is tracked as a separate follow-up
# (see "Open questions" in docs/INSTALL.md). Until then, users either
# build locally or download the release tarball.

# ── Stage 1: build ────────────────────────────────────────────────────
# Pin to 1.87 to match `rust-version` in workspace Cargo.toml. The
# release workflow uses dtolnay/rust-toolchain@stable — keep this floor
# matched when bumping MSRV.
FROM rust:1.87-slim-bookworm AS builder

# Argument layout: by default we expect `wohl` and `relay` as
# sibling subdirectories of the build context (matches release.yml's
# checkout layout). Override for non-standard checkouts.
ARG WOHL_PATH=wohl
ARG RELAY_PATH=relay

# Build deps: pkg-config + libc dev headers, in case a transitive
# crate grows a sys-dep. The base image already has gcc + ld.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
        pkg-config \
        ca-certificates \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /build
# Copy both source trees in the same layout cargo expects: wohl and
# relay as siblings under /build, so `path = "../relay/..."` resolves
# from /build/wohl/Cargo.toml.
COPY ${WOHL_PATH}  /build/wohl
COPY ${RELAY_PATH} /build/relay

WORKDIR /build/wohl
# Single-package release build. --locked is intentional: Cargo.lock is
# checked in and the release builds reproducibly against it.
RUN cargo build --release --locked -p wohl-hub \
 && strip target/release/wohl-hub \
 && cp target/release/wohl-hub /wohl-hub

# ── Stage 2: runtime ──────────────────────────────────────────────────
# distroless/cc is the smallest base that still ships glibc + libgcc
# (which the Rust binary links to dynamically). It has no shell, no
# package manager, no util-linux. ~25 MB.
#
# If you need to exec into the container for debugging, swap to
# `gcr.io/distroless/cc-debian12:debug` (has BusyBox) for that build.
FROM gcr.io/distroless/cc-debian12:nonroot

# nonroot tag pins uid=65532. We don't need root in the container —
# wohl-hub reads stdin and writes stdout. Filesystem writes go to
# /var/lib/wohl, which the compose file mounts as a writable volume.
USER 65532:65532

# Bring over the runtime binary and a default config. The compose
# file overrides /etc/wohl/wohl.toml with a host-mounted file in
# real deployments — this one is the fallback so `docker run` works
# out of the box.
COPY --from=builder /wohl-hub             /usr/local/bin/wohl-hub
COPY ${WOHL_PATH}/wohl.toml               /etc/wohl/wohl.toml

# distroless has no HEALTHCHECK helper binary, and wohl-hub doesn't
# expose an HTTP health endpoint yet — adding `HEALTHCHECK` would
# require a probe binary in the image. Skipped on purpose; the compose
# file uses `docker compose ps` + `restart: unless-stopped` to keep
# the container live.

ENTRYPOINT ["/usr/local/bin/wohl-hub", "--config", "/etc/wohl/wohl.toml"]
