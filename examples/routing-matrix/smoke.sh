#!/usr/bin/env bash
set -euo pipefail

fixture_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
compose_file="${SWITCHYARD_COMPOSE_FILE:-${fixture_dir}/compose.yaml}"
run_dir="${SWITCHYARD_RUN_DIR:-${fixture_dir}/.run}"
export SWITCHYARD_RUN_DIR="$run_dir"
export SWITCHYARD_UID="${SWITCHYARD_UID:-$(id -u)}"
export SWITCHYARD_GID="${SWITCHYARD_GID:-$(id -g)}"
compose=(docker compose -f "$compose_file")

compose_down() {
  "${compose[@]}" down --remove-orphans >/dev/null 2>&1 || true
}

cleanup() {
  compose_down
  rm -f "$run_dir/backend-1/router-admin.socket" \
    "$run_dir/backend-1/router-switched.json" \
    "$run_dir/backend-2/router-admin.socket"
  rmdir "$run_dir/backend-1" "$run_dir/backend-2" "$run_dir" 2>/dev/null || true
}
trap cleanup EXIT

command -v curl >/dev/null || { echo "routing-matrix smoke: curl is required" >&2; exit 1; }
command -v python3 >/dev/null || { echo "routing-matrix smoke: python3 is required" >&2; exit 1; }

# Preserve named volumes intentionally: the second start must observe the first start's data.
compose_down
mkdir -p "$run_dir/backend-1" "$run_dir/backend-2"

python3 - "$fixture_dir/router-backend-1.json" "$fixture_dir/router-backend-2.json" \
  "$run_dir/backend-1/router-switched.json" <<'PY'
import json
import sys

initial_path, donor_path, output_path = sys.argv[1:]
with open(initial_path, encoding="utf-8") as source:
    switched = json.load(source)
with open(donor_path, encoding="utf-8") as source:
    main = json.load(source)

switched["spec"]["snapshot"]["id"] = "backend-1-routes-switched"
switched["spec"]["snapshot"]["version"] = 2
switched["spec"]["providers"] = main["spec"]["providers"]
switched["spec"]["groups"] = main["spec"]["groups"]
switched["spec"]["bindings"][0]["group"] = "main-services"
with open(output_path, "w", encoding="utf-8") as output:
    json.dump(switched, output)
PY

"${compose[@]}" up --detach --build --wait --wait-timeout 240

backend_url() {
  local namespace_service="$1"
  local address
  address="$("${compose[@]}" port "$namespace_service" 8080 | tail -n 1)"
  printf 'http://%s/identity' "$address"
}

backend_1_first="$(curl --fail --silent --show-error "$(backend_url backend-1-namespace)")"
backend_2_first="$(curl --fail --silent --show-error "$(backend_url backend-2-namespace)")"
backend_ids_before="$("${compose[@]}" ps --quiet backend-1 backend-2)"

"${compose[@]}" exec --no-TTY backend-1-sidecar \
  /usr/local/bin/routing-fixture admin-apply \
  /run/switchyard/router-admin.socket routing-matrix-fixture \
  /run/switchyard/router-switched.json

backend_1_switched="$(curl --fail --silent --show-error "$(backend_url backend-1-namespace)")"
backend_ids_after="$("${compose[@]}" ps --quiet backend-1 backend-2)"
if [[ "$backend_ids_before" != "$backend_ids_after" ]]; then
  echo "routing-matrix smoke: a backend restarted during route apply" >&2
  exit 1
fi

"${compose[@]}" down --remove-orphans
"${compose[@]}" up --detach --wait --wait-timeout 240

backend_1_second="$(curl --fail --silent --show-error "$(backend_url backend-1-namespace)")"
backend_2_second="$(curl --fail --silent --show-error "$(backend_url backend-2-namespace)")"

python3 - "$backend_1_first" "$backend_2_first" "$backend_1_switched" \
  "$backend_1_second" "$backend_2_second" <<'PY'
import json
import sys

b1_first, b2_first, b1_switched, b1_second, b2_second = map(json.loads, sys.argv[1:])

def expected(group):
    prefix = f"services-{group}"
    return {
        "catalog": {"service": "catalog", "provider": f"{prefix}/catalog"},
        "search": {"service": "search", "provider": f"{prefix}/search"},
        "reports": {"service": "reports", "provider": f"{prefix}/reports"},
        "scheduler": {"service": "scheduler", "provider": f"{prefix}/scheduler"},
        "audit": {"service": "audit", "provider": "services-shared/audit"},
    }

assert b1_first["backend"] == "backend-1"
assert b1_first["services"] == expected("feature")
assert b2_first["backend"] == "backend-2"
assert b2_first["services"] == expected("main")
assert b1_switched["backend"] == "backend-1"
assert b1_switched["services"] == expected("main")
assert b1_second["services"] == b1_first["services"]
assert b2_second["services"] == b2_first["services"]
assert b1_second["requestCount"] > b1_switched["requestCount"]
assert b2_second["requestCount"] > b2_first["requestCount"]
assert b1_first["services"]["audit"] == b2_first["services"]["audit"]

print("routing-matrix smoke: isolated localhost routes, live switching, and persistence verified")
PY
