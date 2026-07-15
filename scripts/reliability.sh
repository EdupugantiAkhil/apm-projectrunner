#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

export SWITCHYARD_SOAK_SECONDS="${SWITCHYARD_SOAK_SECONDS:-30}"
export SWITCHYARD_RELOAD_STORM_SECONDS="${SWITCHYARD_RELOAD_STORM_SECONDS:-30}"
export SWITCHYARD_CONCURRENCY="${SWITCHYARD_CONCURRENCY:-16}"

echo "Reliability suite"
echo "  SWITCHYARD_SOAK_SECONDS=$SWITCHYARD_SOAK_SECONDS"
echo "  SWITCHYARD_RELOAD_STORM_SECONDS=$SWITCHYARD_RELOAD_STORM_SECONDS"
echo "  SWITCHYARD_CONCURRENCY=$SWITCHYARD_CONCURRENCY"

echo "Building reliability test binaries..."
cargo test -p router-core --test engine --no-run
cargo test -p router-tcp --test tcp_proxy --no-run
cargo test -p router-pingora --test http_proxy --no-run
cargo test -p switchyard-daemon --test api --no-run

run_test() {
  local label="$1"
  shift
  local start end elapsed
  start="$(date +%s)"
  echo
  echo "==> $label"
  "$@"
  end="$(date +%s)"
  elapsed="$((end - start))"
  echo "<== $label completed in ${elapsed}s"
}

run_test "router-core reload storm" \
  cargo test -p router-core --test engine reload_storm_preserves_group_atomicity_and_version_order -- --ignored --exact

run_test "router-tcp reload storm and leak check" \
  cargo test -p router-tcp --test tcp_proxy reload_storm_under_concurrent_clients_has_no_partial_tcp_responses_or_leaks -- --ignored --exact

run_test "router-pingora HTTP reload storm and leak check" \
  cargo test -p router-pingora --test http_proxy reload_storm_under_concurrent_http_clients_returns_complete_provider_responses -- --ignored --exact

run_test "router-pingora HTTP soak and health flap check" \
  cargo test -p router-pingora --test http_proxy long_running_http_soak_correlates_health_flaps_and_has_no_resource_leak -- --ignored --exact

run_test "switchyard-daemon high concurrency API" \
  cargo test -p switchyard-daemon --test api high_concurrency_api_respects_global_limit_deployment_locks_and_sqlite_consistency -- --ignored --exact
