# Installing `wohl-hub`

The Wohl Hub is the verified home-supervision orchestrator: it wires the
six monitor crates (`wohl-leak`, `wohl-temp`, `wohl-air`, `wohl-door`,
`wohl-power`, `wohl-alert`) and the Relay engines into a single
long-running daemon. This guide covers three deployment shapes:

- **Release tarball + systemd** — recommended for a Raspberry Pi or
  MiniPC field hub. This is the primary supported path.
- **Building from source** — useful for development, AArch64 cross
  builds outside the matrix, or when you need to track `main`.
- **Docker / Compose** — useful for local trials, CI smoke tests, or
  running the hub on a NAS / homelab box.

> Wohl's tagline is "Verified. Always on." — every option here is
> shaped so a freshly installed hub stays up across crashes, power
> blips, and OS upgrades without operator babysitting.

---

## Prerequisites

| Target | OS / arch | Notes |
|---|---|---|
| Raspberry Pi 4 / 5 | Raspberry Pi OS Bookworm (64-bit) — `aarch64-unknown-linux-gnu` | Primary modeled HubNode (see `spar/wohl_nodes.aadl`). |
| MiniPC / NUC | Debian 12 Bookworm or equivalent — `x86_64-unknown-linux-gnu` | Secondary HubNode variant. |
| Dev workstation | macOS 13+ on Apple Silicon or Intel | Tarballs published for both, suitable for local trials but not production. |

For source builds you additionally need:

- Rust 1.85.0 or newer (`rustup install 1.85.0`)
- `git` and a checkout of the `pulseengine/relay` repository **as a
  sibling of the `wohl/` checkout** (the workspace uses `path = "../relay/..."`
  dependencies — see `Cargo.toml`).
- `pkg-config`, `gcc`, `libssl-dev` (Debian-style base systems include
  these in `build-essential`; Pi OS Bookworm has them in the default
  `bookworm` repo).

For Docker:

- Docker Engine 24+ with the Compose plugin, or Podman 4+ with
  `podman-compose`.

---

## Quick install from a release tarball

This is the fastest path on a fresh Pi or MiniPC. v0.1.0+ release
binaries are published at
<https://github.com/pulseengine/wohl/releases> with cosign-signed
SHA256SUMS and SLSA build provenance.

```bash
# 1. Pick the right archive for your host
VERSION=v0.1.0
case "$(uname -m)" in
  aarch64|arm64)  TARGET=aarch64-unknown-linux-gnu ;;
  x86_64|amd64)   TARGET=x86_64-unknown-linux-gnu  ;;
  *) echo "unsupported arch: $(uname -m)"; exit 1 ;;
esac

# 2. Download + verify (see "Verifying integrity" below for the full one-liner)
cd /tmp
gh release download "$VERSION" \
    --repo pulseengine/wohl \
    --pattern "wohl-hub-${VERSION}-${TARGET}.tar.gz" \
    --pattern "SHA256SUMS.txt"
sha256sum --check --ignore-missing SHA256SUMS.txt

# 3. Install the binary
mkdir -p wohl-staging && tar -xzf "wohl-hub-${VERSION}-${TARGET}.tar.gz" -C wohl-staging
sudo install -m 0755 wohl-staging/wohl-hub /usr/local/bin/wohl-hub

# 4. Create the runtime user + state directories
sudo useradd --system --home-dir /var/lib/wohl --shell /usr/sbin/nologin wohl || true
sudo install -d -o wohl -g wohl /var/lib/wohl /var/log/wohl
sudo install -d -m 0755 /etc/wohl

# 5. Drop in a config (use the in-repo example as a starting point)
sudo curl -fsSL \
    "https://raw.githubusercontent.com/pulseengine/wohl/${VERSION}/wohl.toml" \
    -o /etc/wohl/wohl.toml

# 6. Install + enable the systemd unit
sudo curl -fsSL \
    "https://raw.githubusercontent.com/pulseengine/wohl/${VERSION}/deploy/systemd/wohl-hub.service" \
    -o /etc/systemd/system/wohl-hub.service
sudo systemctl daemon-reload
sudo systemctl enable --now wohl-hub

# 7. Tail the log to confirm boot
journalctl -u wohl-hub -f
```

You should see:

```
[wohl-hub] loaded config from /etc/wohl/wohl.toml
[wohl-hub] ready — reading sensor events from stdin
```

The hub is now running. By default it reads JSON-line sensor events on
stdin and prints JSON alerts on stdout — both go to the journal. To
switch to the CCSDS binary wire format used by the Wohl sensor nodes,
append `--ccsds` to `ExecStart=` in the unit (or set
`Environment=WOHL_INPUT=ccsds`) and `systemctl daemon-reload &&
systemctl restart wohl-hub`.

---

## Building from source

```bash
# Sibling layout: relay/ and wohl/ next to each other.
mkdir -p ~/src/pulseengine && cd ~/src/pulseengine
git clone https://github.com/pulseengine/relay.git
git clone https://github.com/pulseengine/wohl.git

cd wohl
# Pin to the workspace MSRV
rustup install 1.85.0
cargo +1.85.0 build --release -p wohl-hub

# Install
sudo install -m 0755 target/release/wohl-hub /usr/local/bin/wohl-hub
```

Continue with steps 4–7 from the tarball install above.

### Sibling-path caveat

The workspace `Cargo.toml` references the Relay engines via
`path = "../relay/..."`. If you put `relay/` somewhere other than
`../relay` relative to `wohl/`, you need to either symlink it or edit
the workspace `[workspace.dependencies]` block. The release workflow
(`.github/workflows/release.yml`) pins a specific Relay commit via
`RELAY_REF`; you can pin the same SHA locally to reproduce a release
build bit-for-bit (modulo timestamps).

---

## Configuration

The hub loads its config from (in order):

1. The path passed via `--config /path/to/wohl.toml`
2. The `WOHL_CONFIG` environment variable
3. `./wohl.toml` (cwd) — kept for `cargo run` workflows
4. `./crates/wohl-hub/wohl.toml` — same, for in-repo development

If an explicit `--config` or `$WOHL_CONFIG` path is set and the file is
missing or invalid, the hub exits with status 2 (this is deliberate —
we want systemd to restart and surface the error rather than silently
boot with defaults).

The example `wohl.toml` at the repo root is the reference and is the
file installed by the quick-install script above. Its sections:

| Section | Purpose |
|---|---|
| `[scheduler]` | Tick rate for the housekeeping scheduler (default 1 Hz). |
| `[[zones]]` | One entry per monitored zone (kitchen / bathroom / basement / …). Each zone enables a subset of monitors via `sensors = ["temp", "water", "air", "power"]` and sets thresholds (`temp_freeze`, `co2_critical`, `power_max_watts`, …). |
| `[[contacts]]` | One entry per door/window contact, with `max_open_sec` and `night_start` / `night_end` for the after-hours alert. |
| `[alerts]` | Dispatcher tuning: `rate_limit_per_minute`, `dedup_cooldown_sec`. |

Edit `/etc/wohl/wohl.toml`, then `sudo systemctl restart wohl-hub` to
pick up the change. The hub does not yet watch the file or hot-reload.

---

## Docker deployment

The Compose stack at `deploy/docker/docker-compose.yml` runs a single
`wohl-hub` container backed by a distroless image, a host-mounted
config, and a named volume for persistent state.

```bash
# Sibling layout — the Dockerfile build context climbs to the parent
# directory so it can see both wohl/ and relay/.
cd ~/src/pulseengine
git clone https://github.com/pulseengine/relay.git
git clone https://github.com/pulseengine/wohl.git

cd wohl/deploy/docker
mkdir -p config
cp ../../wohl.toml config/wohl.toml   # edit as needed

docker compose up -d --build
docker compose logs -f wohl-hub
```

CI does not yet publish an OCI image — see "Open questions" below.
Until then the Compose file builds locally; expect ~3 min the first
time on a Pi 4, ~30 s on a MiniPC.

To switch a Compose deployment to CCSDS input, edit the `services:
wohl-hub:` block to add `command: ["--config", "/etc/wohl/wohl.toml", "--ccsds"]`
(which overrides the image ENTRYPOINT's args).

---

## Verifying integrity

Every release archive is covered by two independent attestations:

- **cosign-signed SHA256SUMS** — keyless OIDC signature bound to the
  exact `release.yml` workflow run that produced the assets.
- **SLSA build provenance** — in-toto statement recorded in the Rekor
  transparency log, generated by `actions/attest-build-provenance@v2`.

Verify both:

```bash
# 1. The checksum file itself was produced by our release workflow
cosign verify-blob \
  --certificate-identity-regexp \
    'https://github.com/pulseengine/wohl/.github/workflows/release.yml@.*' \
  --certificate-oidc-issuer \
    'https://token.actions.githubusercontent.com' \
  --bundle SHA256SUMS.txt.cosign.bundle \
  SHA256SUMS.txt

# 2. The binary archive's build provenance
gh attestation verify "wohl-hub-${VERSION}-${TARGET}.tar.gz" \
  --repo pulseengine/wohl

# 3. The archive matches the signed checksum
sha256sum --check --ignore-missing SHA256SUMS.txt
```

All three must succeed before you put the binary into `/usr/local/bin`
on a production hub.

---

## Troubleshooting

| Symptom | Likely cause | Fix |
|---|---|---|
| `[wohl-hub] fatal: cannot read /etc/wohl/wohl.toml: No such file` and the unit restarts in a loop | Step 5 of the install skipped | Create the file, or run with `--config /path/to/your.toml`. |
| `failed to load manifest for dependency 'relay-lc'` during source build | The `relay/` sibling is missing | `git clone https://github.com/pulseengine/relay.git ../relay`. |
| Unit enters `failed` state after 5 fast restarts | Crash loop hit `StartLimitBurst=5` within 60 s | `journalctl -u wohl-hub -p err` to see the underlying crash; fix the config, then `systemctl reset-failed wohl-hub && systemctl start wohl-hub`. |
| `Permission denied` writing to `/var/lib/wohl` | Directory not owned by the `wohl` user | `sudo chown -R wohl:wohl /var/lib/wohl`. |
| Docker build fails with `failed to compute cache key: "/relay" not found` | Build context doesn't include the relay sibling | Build from the parent directory: `docker build -f wohl/Dockerfile .` with `relay/` and `wohl/` both present. |
| Compose container restarts on every CCSDS packet | Sensor stream is not aligned to 14-byte CCSDS frames | Confirm the upstream sender is emitting `relay_ccsds::sensor_wire::encode_packet` output, not JSON. |

The hub currently exposes no listening ports, so port-conflict
diagnoses don't apply. When network input lands, this section will
grow a row for `bind: address already in use`.

---

## Updating

```bash
# 1. Download the new tarball + checksums and verify (see above)
NEW_VERSION=v0.2.0
gh release download "$NEW_VERSION" \
    --repo pulseengine/wohl \
    --pattern "wohl-hub-${NEW_VERSION}-${TARGET}.tar.gz"

# 2. Stage the new binary
mkdir -p /tmp/wohl-${NEW_VERSION} && tar -xzf "wohl-hub-${NEW_VERSION}-${TARGET}.tar.gz" \
    -C /tmp/wohl-${NEW_VERSION}

# 3. Keep the old one for fast rollback
sudo mv /usr/local/bin/wohl-hub /usr/local/bin/wohl-hub.previous
sudo install -m 0755 /tmp/wohl-${NEW_VERSION}/wohl-hub /usr/local/bin/wohl-hub

# 4. Restart the unit and watch the log
sudo systemctl restart wohl-hub
journalctl -u wohl-hub -f
```

### Rolling back

```bash
sudo systemctl stop wohl-hub
sudo mv /usr/local/bin/wohl-hub /usr/local/bin/wohl-hub.broken
sudo mv /usr/local/bin/wohl-hub.previous /usr/local/bin/wohl-hub
sudo systemctl start wohl-hub
```

The config schema is intentionally additive — new release tarballs
should accept your existing `wohl.toml` unchanged. If a breaking change
ever lands it will be called out in the release notes with a migration
recipe.

For Docker deployments, swap the image tag in `docker-compose.yml` and
`docker compose up -d` — Compose recreates the container with the new
image and the named state volume is preserved.

---

## Open questions

- **sd_notify watchdog** — wiring `sd_notify(READY=1)` and a periodic
  `WATCHDOG=1` would let the unit move to `Type=notify` and let systemd
  kill+restart on a tick-loss. Tracked for 0.3 once the scheduler
  thread modeled in `spar/wohl_system.aadl` lands.
- **Published OCI image** — the release workflow signs and uploads
  tarballs today; adding a ghcr.io push (with cosign signature and
  SLSA attestation matching the tarball provenance) is a separate PR.
- **State-directory schema** — `/var/lib/wohl` is reserved but
  currently unused. Once `wohl-alert` persists dedup state or the
  health table snapshots between restarts, this guide will grow a
  layout section.
- **Network input modes** — when sensor nodes start delivering CCSDS
  over UDP/TCP (vs. piping into stdin from a separate process),
  `RestrictAddressFamilies=` and the firewall recipe will need an
  update.
