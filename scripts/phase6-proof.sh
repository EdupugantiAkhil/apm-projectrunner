#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

for command in cargo node npm docker; do
  command -v "$command" >/dev/null || {
    if [[ "$command" == npm || "$command" == node ]]; then
      echo "phase 6 proof: Node.js and npm are required for the MVP GUI; install the pinned project-compatible Node toolchain and rerun" >&2
    else
      echo "phase 6 proof: $command is required" >&2
    fi
    exit 1
  }
done

docker info >/dev/null 2>&1 || {
  echo "phase 6 proof: Docker Engine is unavailable to the current user" >&2
  exit 1
}
docker compose version >/dev/null 2>&1 || {
  echo "phase 6 proof: Docker Compose v2 is required" >&2
  exit 1
}

./scripts/check.sh

(
  cd packages/web
  npm ci
  npm run build
  npm test
)

./examples/jas-base/smoke.sh

echo "phase 6 proof: workspace formatting, tests, clippy -D warnings, rustdoc -D warnings, GUI clean install/build/tests, and the live JAS MVP fixture passed"
echo "phase 6 proof: examples/routing-matrix/smoke.sh remains the standalone live routing-proof command"
