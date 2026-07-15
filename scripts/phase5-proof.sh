#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

# Covers daemon restart, durable route history, acknowledgement-gated failures,
# and missing-database manifest recovery without requiring Docker.
cargo test -p switchyard-daemon --all-features --test api

if ! docker info >/dev/null 2>&1 || ! docker compose version >/dev/null 2>&1; then
  echo "phase 5 Docker gate skipped: Docker Engine or Compose is unavailable" >&2
  exit 0
fi

# The routing matrix remains the owned-resource, live-router, crash-recovery,
# label-recovery, and cleanup proof for the persistent control plane.
./examples/routing-matrix/smoke.sh
