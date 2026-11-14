#!/bin/sh
# Consumer guard for seam gale-wohl/air-sensor-read (pulseengine/gale#511).
#
# wohl is consume-only on this seam: it binds to the SEN66 driver's output,
# never to i2c-master itself (gale#511 decision, 2026-11-13). This guard pins
# the bus package version the driver is built against, so a silent move of
# gust:bus on gale's default branch goes red here instead of surfacing later
# as a binding mismatch.
#
# Exit 0: pin matches. Exit 1: pin moved (seam violation — update the block
# in pulseengine/gale#511 and this guard together). Exit 2: gale unreadable
# (unknown, never green) — same exit semantics as seam-check.
set -u

REPO="pulseengine/gale"
BRANCH="main"
WIT_PATH="wit/bus/gust-bus.wit"
WANT="package gust:bus@0.1.0;"

url="https://raw.githubusercontent.com/${REPO}/${BRANCH}/${WIT_PATH}"
body=$(curl -fsS --max-time 30 "$url") || {
    echo "UNKNOWN: could not read ${url} (network or GitHub error)" >&2
    exit 2
}

if printf '%s\n' "$body" | grep -qxF "$WANT"; then
    echo "OK: ${REPO}@${BRANCH} ${WIT_PATH} carries '${WANT}'"
    exit 0
fi

got=$(printf '%s\n' "$body" | grep -m1 '^package ' || echo '(no package line)')
echo "SEAM VIOLATION: expected '${WANT}' in ${WIT_PATH} on ${REPO}@${BRANCH}; found: ${got}" >&2
echo "The provider moved the interface. Reconcile the seam block in pulseengine/gale#511 and this guard in the same change." >&2
exit 1
