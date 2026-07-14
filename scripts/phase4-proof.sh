#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

./scripts/check.sh test
./examples/routing-matrix/smoke.sh
