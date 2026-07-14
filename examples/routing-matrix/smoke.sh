#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
fixture_dir="$root/examples/routing-matrix"
deployment="$fixture_dir/deployment.yaml"
artifact_dir="$root/.switchyard/generated/routing-matrix"
runtime_dir="$root/.switchyard/run/routing-matrix"
switchyard="$root/target/debug/switchyard"
router="$root/target/debug/switchyard-router"
export SWITCHYARD_ROUTER_TOKEN="${SWITCHYARD_ROUTER_TOKEN:-routing-matrix-phase4-proof}"
export SWITCHYARD_ROUTER_BIN="$router"
export SWITCHYARD_UID="${SWITCHYARD_UID:-$(id -u)}"
export SWITCHYARD_GID="${SWITCHYARD_GID:-$(id -g)}"

for command in cargo curl docker python3; do
  command -v "$command" >/dev/null || {
    echo "routing-matrix proof: $command is required" >&2
    exit 1
  }
done
docker info >/dev/null
docker compose version >/dev/null

if docker ps --all --quiet --filter label=dev.switchyard.deployment=routing-matrix | grep -q . \
  || docker volume ls --quiet --filter label=dev.switchyard.deployment=routing-matrix | grep -q .; then
  echo "routing-matrix proof: owned fixture resources already exist; clean them with 'switchyard cleanup $deployment --yes'" >&2
  exit 1
fi

cleanup() {
  if [[ -x "$switchyard" ]]; then
    "$switchyard" down "$deployment" >/dev/null 2>&1 || true
    "$switchyard" cleanup "$deployment" --yes >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

cd "$root"
cargo build --locked --workspace --bins
"$switchyard" validate "$deployment"
started_ns="$(python3 -c 'import time; print(time.time_ns())')"
"$switchyard" up "$deployment"
ready_ns="$(python3 -c 'import time; print(time.time_ns())')"

compose=(
  docker compose
  --project-name sy--routing-matrix
  --project-directory "$root"
  --file "$artifact_dir/compose.yaml"
)

published_url() {
  local service="$1"
  local address
  address="$("${compose[@]}" port "$service" 8080 | tail -n 1)"
  printf 'http://%s/identity' "$address"
}

wait_http() {
  local url="$1"
  local deadline=$((SECONDS + 30))
  until curl --noproxy '*' --fail --silent --output /dev/null "$url"; do
    (( SECONDS < deadline )) || {
      echo "routing-matrix proof: timed out waiting for $url" >&2
      return 1
    }
    sleep 0.2
  done
}

ui_identity() {
  local ui="$1"
  curl --noproxy '*' --fail --silent --show-error \
    --resolve "$ui.routing-matrix.localhost:18080:127.0.0.1" \
    "http://$ui.routing-matrix.localhost:18080/identity"
}

browser_identity() {
  local ui="$1"
  curl --noproxy '*' --fail --silent --show-error \
    --header "Origin: http://$ui.routing-matrix.localhost:18080" \
    http://localhost:10081/identity
}

admin_request() {
  local socket="$1"
  local operation="$2"
  local config="${3:-}"
  python3 - "$socket" "$SWITCHYARD_ROUTER_TOKEN" "$operation" "$config" <<'PY'
import json
import socket
import sys

path, token, operation, config_path = sys.argv[1:]
request = {"token": token, "operation": operation}
if config_path:
    with open(config_path, encoding="utf-8") as source:
        request["config"] = json.load(source)
client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
client.settimeout(15)
client.connect(path)
client.sendall(json.dumps(request).encode() + b"\n")
client.shutdown(socket.SHUT_WR)
chunks = []
while True:
    chunk = client.recv(65536)
    if not chunk:
        break
    chunks.append(chunk)
response = json.loads(b"".join(chunks))
print(json.dumps(response, sort_keys=True))
if not response.get("ok", False):
    raise SystemExit(3)
PY
}

make_host_snapshot() {
  local version="$1"
  local ui_one_backend="$2"
  local output="$3"
  python3 - "$runtime_dir/host-router.json" "$version" "$ui_one_backend" "$output" <<'PY'
import json
import sys

source_path, version, backend, output_path = sys.argv[1:]
with open(source_path, encoding="utf-8") as source:
    config = json.load(source)
config["spec"]["snapshot"]["id"] = f"routing-matrix-host-{version}"
config["spec"]["snapshot"]["version"] = int(version)
for route in config["spec"]["browserRoutes"]:
    identity = route["identity"]
    if identity.get("origin") == "http://ui-1.routing-matrix.localhost:18080" \
            or identity.get("value") == "ui-1":
        route["provider"] = backend
with open(output_path, "w", encoding="utf-8") as output:
    json.dump(config, output)
PY
}

ui_1="$(ui_identity ui-1)"
ui_2="$(ui_identity ui-2)"
ui_3="$(ui_identity ui-3)"
backend_1="$(browser_identity ui-1)"
backend_2="$(browser_identity ui-2)"
backend_3="$(browser_identity ui-3)"
python3 - "$ui_1" "$ui_2" "$ui_3" "$backend_1" "$backend_2" "$backend_3" <<'PY'
import json
import sys

ui1, ui2, ui3, b1, b2, b3 = map(json.loads, sys.argv[1:])
assert [ui1["ui"], ui2["ui"], ui3["ui"]] == ["ui-1", "ui-2", "ui-3"]
assert {ui1["backendUrl"], ui2["backendUrl"], ui3["backendUrl"]} == {"http://localhost:10081/identity"}

def expected(group):
    return {
        **{service: {"service": service, "provider": f"services-{group}/{service}"}
           for service in ["catalog", "search", "reports", "scheduler"]},
        "audit": {"service": "audit", "provider": "services-shared/audit"},
    }

assert b1["backend"] == "backend-1" and b1["services"] == expected("feature")
assert b2["backend"] == "backend-2" and b2["services"] == expected("main")
assert b3["backend"] == "backend-1" and b3["services"] == expected("feature")
assert b1["services"] == b3["services"]
PY

startup_order="$("${compose[@]}" ps --quiet routing-matrix--services-feature-catalog--app routing-matrix--backend-1--app--app | xargs docker inspect --format '{{.Name}} {{.State.StartedAt}}')"
python3 - "$started_ns" "$ready_ns" "$startup_order" <<'PY'
import datetime
import sys

started, ready = map(int, sys.argv[1:3])
assert ready - started >= 1_000_000_000, "delayed dependency readiness was not observed"
times = {}
for line in sys.argv[3].splitlines():
    name, value = line.split()
    times[name] = datetime.datetime.fromisoformat(value.replace("Z", "+00:00"))
provider = next(value for name, value in times.items() if "services-feature-catalog" in name)
backend = next(value for name, value in times.items() if "backend-1--app--app" in name)
assert backend > provider
PY

application_ids_before="$("${compose[@]}" ps --quiet routing-matrix--ui-1--app routing-matrix--ui-2--app routing-matrix--ui-3--app routing-matrix--backend-1--app--app routing-matrix--backend-2--app--app)"
make_host_snapshot 2 backend-2 "$runtime_dir/host-switch.json"
echo "host snapshot apply: $(admin_request "$runtime_dir/host.socket" apply "$runtime_dir/host-switch.json")"
switched_ui_1="$(browser_identity ui-1)"
python3 - "$switched_ui_1" <<'PY'
import json, sys
assert json.loads(sys.argv[1])["backend"] == "backend-2"
PY
test "$application_ids_before" = "$("${compose[@]}" ps --quiet routing-matrix--ui-1--app routing-matrix--ui-2--app routing-matrix--ui-3--app routing-matrix--backend-1--app--app routing-matrix--backend-2--app--app)"

"${compose[@]}" stop routing-matrix--backend-1--app--app >/dev/null
make_host_snapshot 3 backend-1 "$runtime_dir/host-unhealthy.json"
if admin_request "$runtime_dir/host.socket" apply "$runtime_dir/host-unhealthy.json" >"$runtime_dir/rejected-apply.json"; then
  echo "routing-matrix proof: unhealthy host snapshot unexpectedly activated" >&2
  exit 1
fi
grep -q 'rolled_back' "$runtime_dir/rejected-apply.json"
test "$(admin_request "$runtime_dir/host.socket" current-version | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["version"])')" = 2
"${compose[@]}" start routing-matrix--backend-1--app--app >/dev/null
wait_http "$(published_url routing-matrix--backend-1--app)"
make_host_snapshot 4 backend-1 "$runtime_dir/host-restored.json"
echo "host snapshot restore: $(admin_request "$runtime_dir/host.socket" apply "$runtime_dir/host-restored.json")"

backend_1_url="$(published_url routing-matrix--backend-1--app)"
"$switchyard" bind "$deployment" backend-1 main-services
main_observation="$(curl --noproxy '*' --fail --silent --show-error "$backend_1_url")"
"$switchyard" bind "$deployment" backend-1 feature-services
feature_observation="$(curl --noproxy '*' --fail --silent --show-error "$backend_1_url")"
python3 - "$main_observation" "$feature_observation" <<'PY'
import json
import sys

def groups(value):
    return {entry["provider"].split("/")[0] for name, entry in value["services"].items() if name != "audit"}
main, feature = map(json.loads, sys.argv[1:])
assert groups(main) == {"services-main"}
assert groups(feature) == {"services-feature"}
assert main["services"]["audit"] == feature["services"]["audit"] == {"service": "audit", "provider": "services-shared/audit"}
PY
test "$application_ids_before" = "$("${compose[@]}" ps --quiet routing-matrix--ui-1--app routing-matrix--ui-2--app routing-matrix--ui-3--app routing-matrix--backend-1--app--app routing-matrix--backend-2--app--app)"
echo "sidecar route snapshot: $(admin_request "$runtime_dir/backend-1.socket" routes)"
echo "sidecar routing decisions: $(admin_request "$runtime_dir/backend-1.socket" events)"

"${compose[@]}" stop routing-matrix--services-main-catalog--app >/dev/null
if "$switchyard" bind "$deployment" backend-1 main-services >"$runtime_dir/rejected-bind.log" 2>&1; then
  echo "routing-matrix proof: unhealthy group unexpectedly activated" >&2
  exit 1
fi
grep -q 'previous snapshot remains active' "$runtime_dir/rejected-bind.log"
python3 - "$(curl --noproxy '*' --fail --silent --show-error "$backend_1_url")" <<'PY'
import json, sys
providers = {value["provider"].split("/")[0] for name, value in json.loads(sys.argv[1])["services"].items() if name != "audit"}
assert providers == {"services-feature"}
PY
"${compose[@]}" start routing-matrix--services-main-catalog--app >/dev/null
catalog_deadline=$((SECONDS + 30))
until "${compose[@]}" exec --no-TTY routing-matrix--services-main-catalog--app \
  /usr/local/bin/routing-fixture probe 127.0.0.1:8080 >/dev/null 2>&1; do
  (( SECONDS < catalog_deadline )) || {
    echo "routing-matrix proof: timed out waiting for restarted main catalog" >&2
    exit 1
  }
  sleep 0.2
done

"${compose[@]}" stop routing-matrix--services-feature-catalog--app >/dev/null
if curl --noproxy '*' --fail --silent --output /dev/null "$backend_1_url"; then
  echo "routing-matrix proof: backend succeeded while its selected provider was stopped" >&2
  exit 1
fi
"${compose[@]}" start routing-matrix--services-feature-catalog--app >/dev/null
wait_http "$backend_1_url"

sidecar_id="$("${compose[@]}" ps --quiet routing-matrix--backend-1--app--router)"
"${compose[@]}" exec --no-TTY routing-matrix--backend-1--app--router \
  sh -c 'kill -KILL 1' >/dev/null 2>&1 || true
wait_http "$backend_1_url"
test "$sidecar_id" = "$("${compose[@]}" ps --quiet routing-matrix--backend-1--app--router)"

backend_id="$("${compose[@]}" ps --quiet routing-matrix--backend-1--app--app)"
"${compose[@]}" exec --no-TTY routing-matrix--backend-1--app--app \
  sh -c 'kill -KILL 1' >/dev/null 2>&1 || true
wait_http "$backend_1_url"
test "$backend_id" = "$("${compose[@]}" ps --quiet routing-matrix--backend-1--app--app)"

host_pid="$(python3 -c 'import json; print(json.load(open(".switchyard/run/routing-matrix/host-gateway.json"))["pid"])')"
kill -KILL "$host_pid"
for _ in {1..50}; do
  [[ ! -e "/proc/$host_pid" ]] && break
  sleep 0.1
done
"$switchyard" up "$deployment"
browser_identity ui-1 >/dev/null

before_restart="$(browser_identity ui-1)"
"${compose[@]}" restart >/dev/null
"$switchyard" down "$deployment"
"$switchyard" up "$deployment"
after_restart="$(browser_identity ui-1)"
python3 - "$before_restart" "$after_restart" <<'PY'
import json, sys
before, after = map(json.loads, sys.argv[1:])
assert after["requestCount"] > before["requestCount"]
assert after["services"] == before["services"]
PY

echo "host routing decisions: $(admin_request "$runtime_dir/host.socket" events)"
echo "routing-matrix proof: custom domains, fixed localhost routes, atomic switching, rollback, crash recovery, delayed readiness, isolation, and persistence verified"
