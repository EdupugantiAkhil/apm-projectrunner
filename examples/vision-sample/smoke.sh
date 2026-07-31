#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
fixture_root="$(mktemp -d "${TMPDIR:-/tmp}/apmpr-vision-sample.XXXXXX")"
project="$fixture_root/project"
origin="$fixture_root/origin"
deployment="$project/deployment.yaml"
apmpr="$root/target/debug/apmpr"
router="$root/target/debug/apmpr-router"
external_name="apmpr-phase3-external-$$"
export APMPR_ROUTER_TOKEN="${APMPR_ROUTER_TOKEN:-phase3-vision-sample-proof}"
export APMPR_ROUTER_BIN="$router"
export APMPR_UID="${APMPR_UID:-$(id -u)}"
export APMPR_GID="${APMPR_GID:-$(id -g)}"

for command in curl docker git python3; do
  command -v "$command" >/dev/null || {
    echo "vision-sample proof: $command is required" >&2
    exit 1
  }
done
if [[ ${APMPR_SKIP_BUILD:-0} != 1 ]]; then
  command -v cargo >/dev/null || {
    echo "vision-sample proof: cargo is required unless APMPR_SKIP_BUILD=1" >&2
    exit 1
  }
fi
docker info >/dev/null
docker compose version >/dev/null

cleanup() {
  local status=$?
  if [[ $status -ne 0 && ${APMPR_KEEP_FAILED_FIXTURE:-0} == 1 ]]; then
    echo "vision-sample proof: preserved failed fixture at $fixture_root" >&2
    return
  fi
  if [[ -x "$apmpr" && -f "$deployment" ]]; then
    (cd "$project" && "$apmpr" down "$deployment" >/dev/null 2>&1) || true
    (cd "$project" && "$apmpr" cleanup "$deployment" --yes >/dev/null 2>&1) || true
  fi
  docker rm --force "$external_name" >/dev/null 2>&1 || true
  rm -rf "$fixture_root"
}
trap cleanup EXIT

mkdir -p "$project" "$origin"
git -C "$origin" init -b main >/dev/null
git -C "$origin" config user.email tests@apmpr.invalid
git -C "$origin" config user.name "APM ProjectRunner Tests"

python3 - "$origin" <<'PY'
from pathlib import Path
import sys

root = Path(sys.argv[1])
(root / "package.json").write_text(
    '{"scripts":{"dev":"node server.js"}}\n', encoding="utf-8"
)
(root / "ui-identity.txt").write_text("ui-1\n", encoding="utf-8")
(root / "backend-identity.txt").write_text("backend-1\n", encoding="utf-8")
(root / "server.js").write_text(r'''
const http = require("http");
const fs = require("fs");
const identity = fs.readFileSync("ui-identity.txt", "utf8").trim();
http.createServer((request, response) => {
  response.writeHead(200, {"content-type": "text/plain"});
  response.end(request.url === "/health" ? "ok" : identity);
}).listen(5173, "0.0.0.0");
'''.lstrip(), encoding="utf-8")
(root / "gradlew").write_text("#!/bin/sh\nexec java Backend.java\n", encoding="utf-8")
(root / "Backend.java").write_text(r'''
import com.sun.net.httpserver.HttpExchange;
import com.sun.net.httpserver.HttpServer;
import java.net.InetSocketAddress;
import java.net.Socket;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;

public class Backend {
    private static boolean connects(int port) {
        try (Socket socket = new Socket()) {
            socket.connect(new InetSocketAddress("127.0.0.1", port), 500);
            return true;
        } catch (Exception ignored) {
            return false;
        }
    }

    private static void respond(HttpExchange exchange) throws java.io.IOException {
        String body = exchange.getRequestURI().getPath().equals("/actuator/health")
            ? "ok"
            : Files.readString(Path.of("backend-identity.txt")).trim()
                + " db=" + connects(5432) + " external=" + connects(9200);
        byte[] bytes = body.getBytes(StandardCharsets.UTF_8);
        exchange.sendResponseHeaders(200, bytes.length);
        exchange.getResponseBody().write(bytes);
        exchange.close();
    }

    public static void main(String[] args) throws Exception {
        HttpServer server = HttpServer.create(new InetSocketAddress("0.0.0.0", 8080), 0);
        server.createContext("/", Backend::respond);
        server.start();
    }
}
'''.lstrip(), encoding="utf-8")
PY
chmod +x "$origin/gradlew"
git -C "$origin" add .
git -C "$origin" commit -m main >/dev/null
git -C "$origin" switch -c feature-a >/dev/null
printf 'ui-2\n' >"$origin/ui-identity.txt"
git -C "$origin" add ui-identity.txt
git -C "$origin" commit -m feature-a >/dev/null
git -C "$origin" switch main >/dev/null
git -C "$origin" switch -c backend-fix >/dev/null
printf 'backend-2\n' >"$origin/backend-identity.txt"
git -C "$origin" add backend-identity.txt
git -C "$origin" commit -m backend-fix >/dev/null
git -C "$origin" switch main >/dev/null

python3 - "$root/docs/vision/sample-config.md" "$deployment" "$origin" "$external_name" <<'PY'
from pathlib import Path
import sys

markdown_path, output_path, origin, external = map(Path, sys.argv[1:])
markdown = markdown_path.read_text(encoding="utf-8")
yaml = markdown.split("```yaml\n", 1)[1].split("\n```", 1)[0]
yaml = yaml.split("\nscripts:", 1)[0]
yaml = yaml.replace("git@github.com:acme/monorepo.git", str(origin))
yaml = yaml.replace("search.staging.internal", external.name)
Path(output_path).write_text(yaml + "\n", encoding="utf-8")
PY

cd "$root"
if [[ ${APMPR_SKIP_BUILD:-0} != 1 ]]; then
  cargo build --locked --workspace --bins
fi
if ! docker image inspect apmpr-router:local >/dev/null 2>&1; then
  docker build --file examples/routing-matrix/Dockerfile --tag apmpr-router:local .
fi

cd "$project"
"$apmpr" validate "$deployment"
"$apmpr" plan "$deployment" >/dev/null
"$apmpr" up "$deployment"

artifact_dir="$project/.apmpr/generated/comparison"
runtime_dir="$project/.apmpr/run/comparison"
compose=(
  docker compose
  --project-name apmpr--comparison
  --project-directory "$project"
  --file "$artifact_dir/compose.yaml"
)

docker run --detach --name "$external_name" python:3.13-alpine \
  python3 -m http.server 9200 >/dev/null
docker network connect apmpr--comparison--private "$external_name"

read -r group_port backend_port < <(python3 - "$runtime_dir/host-router.json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as source:
    config = json.load(source)
group = next(
    listener["bind"]["port"]
    for listener in config["spec"]["listeners"]
    if any(destination.get("kind") == "custom_domain"
           and destination.get("domain") == "feature-test.comparison.localhost"
           for destination in listener["destinations"])
)
backend = next(
    listener["bind"]["port"]
    for listener in config["spec"]["listeners"]
    if any(destination.get("slot") == "browser-8080"
           for destination in listener["destinations"])
)
print(group, backend)
PY
)

group_request() {
  local domain="$1"
  curl --noproxy '*' --fail --silent --show-error \
    --resolve "$domain:$group_port:127.0.0.1" \
    "http://$domain:$group_port/identity"
}

backend_request() {
  local group="$1"
  curl --noproxy '*' --fail --silent --show-error \
    --header "Origin: http://$group.comparison.localhost:$group_port" \
    "http://127.0.0.1:$backend_port/identity"
}

test "$(group_request feature-test.comparison.localhost)" = ui-1
test "$(group_request regression.comparison.localhost)" = ui-2
test "$(backend_request feature-test)" = "backend-1 db=true external=false"
test "$(backend_request regression)" = "backend-2 db=true external=true"

feature_db="comparison--db-feature-test--db--app"
regression_db="comparison--db-regression--db--app"
feature_db_id="$("${compose[@]}" ps --quiet "$feature_db")"
regression_db_id="$("${compose[@]}" ps --quiet "$regression_db")"
test -n "$feature_db_id"
test -n "$regression_db_id"
test "$feature_db_id" != "$regression_db_id"
python3 - "$feature_db_id" "$regression_db_id" <<'PY'
import json
import subprocess
import sys

containers = json.loads(subprocess.check_output(
    ["docker", "inspect", sys.argv[1], sys.argv[2]], text=True
))
workspace_sources = []
data_sources = []
for container in containers:
    mounts = {mount["Destination"]: mount["Source"] for mount in container["Mounts"]}
    workspace_sources.append(mounts["/workspace"])
    data_sources.append(mounts["/var/lib/postgresql/data"])
assert workspace_sources[0] == workspace_sources[1]
assert data_sources[0] != data_sources[1]
PY

canary="comparison--backend-canary--app--app"
test -n "$("${compose[@]}" ps --quiet "$canary")"
python3 - "$runtime_dir/host-router.json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as source:
    config = json.load(source)
assert all(provider["id"] != "backend-canary" for provider in config["spec"]["providers"])
assert all(
    destination.get("domain") != "backend-canary.feature-test.comparison.localhost"
    for listener in config["spec"]["listeners"]
    for destination in listener["destinations"]
)
PY

"${compose[@]}" stop "$feature_db" >/dev/null
test "$(backend_request feature-test)" = "backend-1 db=false external=false"
test "$(backend_request regression)" = "backend-2 db=true external=true"
"${compose[@]}" start "$feature_db" >/dev/null

"$apmpr" down "$deployment"
"$apmpr" cleanup "$deployment" --yes
test -z "$(docker ps --all --quiet --filter label=dev.apmpr.deployment=comparison)"
test -z "$(docker volume ls --quiet --filter label=dev.apmpr.deployment=comparison)"
test ! -e "$runtime_dir/host-gateway.json"

echo "vision-sample proof: source worktrees, two group addresses, distinct backends and databases, external routing, disabled membership, stop, and cleanup verified"
