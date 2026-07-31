#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
fixture_dir="$root/examples/jas-base"
base_deployment="$fixture_dir/deployment.yaml"
deployment="$fixture_dir/deployment.smoke.yaml"
artifact_dir="$root/.apmpr/generated/jas-base"
apmpr="$root/target/debug/apmpr"
router="$root/target/debug/apmpr-router"
source_name="jas-base-smoke-source-$$"
worktree_name="jas-base-smoke-worktree-$$"
main_source="$root/.apmpr/jas-base-source-main-$$"
worktree="$root/.apmpr/worktrees/$worktree_name"
export APMPR_ROUTER_TOKEN="${APMPR_ROUTER_TOKEN:-jas-base-phase6-proof}"
export APMPR_ROUTER_BIN="$router"
export APMPR_UID="${APMPR_UID:-$(id -u)}"
export APMPR_GID="${APMPR_GID:-$(id -g)}"

for command in cargo curl docker git python3; do
  command -v "$command" >/dev/null || {
    echo "jas-base proof: $command is required" >&2
    exit 1
  }
done
docker info >/dev/null
docker compose version >/dev/null

if docker ps --all --quiet --filter label=dev.apmpr.deployment=jas-base | grep -q . \
  || docker volume ls --quiet --filter label=dev.apmpr.deployment=jas-base | grep -q . \
  || docker network ls --quiet --filter label=dev.apmpr.deployment=jas-base | grep -q .; then
  echo "jas-base proof: owned fixture resources already exist; clean them with 'apmpr cleanup $base_deployment --yes'" >&2
  exit 1
fi

initial_status="$(git -C "$root" status --porcelain=v1 --untracked-files=all)"
registered=false
created_worktree=false

cleanup() {
  if [[ -x "$apmpr" && -f "$deployment" ]]; then
    "$apmpr" down "$deployment" >/dev/null 2>&1 || true
    "$apmpr" cleanup "$deployment" --yes >/dev/null 2>&1 || true
  fi
  if [[ "$created_worktree" == true ]]; then
    "$apmpr" worktree remove "$worktree_name" >/dev/null 2>&1 || true
  fi
  if [[ "$registered" == true ]]; then
    "$apmpr" source deregister "$source_name" >/dev/null 2>&1 || true
  fi
  rm -f "$deployment"
  rm -rf "$main_source"
}
trap cleanup EXIT

cd "$root"
cargo build --locked --workspace --bins

mkdir -p "$main_source"
cp "$fixture_dir/sources/main/start-jas-service.sh" "$main_source/"
cp "$fixture_dir/sources/main/process-compose.yaml" "$main_source/"
git -C "$main_source" init --quiet
git -C "$main_source" config user.name "APM ProjectRunner fixture"
git -C "$main_source" config user.email "fixture@apmpr.invalid"
git -C "$main_source" add .
git -C "$main_source" commit --quiet -m "fixture source"

"$apmpr" source register "$source_name" "$main_source"
registered=true
"$apmpr" worktree create "$source_name" HEAD --name "$worktree_name"
created_worktree=true

python3 - "$base_deployment" "$deployment" "$main_source" "$worktree" <<'PY'
import pathlib
import sys

source, destination, main, feature = map(pathlib.Path, sys.argv[1:])
text = source.read_text(encoding="utf-8")
text = text.replace("sources/main", str(main))
text = text.replace("sources/feature", str(feature))
destination.write_text(text, encoding="utf-8")
PY

"$apmpr" validate "$deployment"

# Variation planning proves two overlay variations resolve without colliding. The
# collision guard also inspects other deployments' generated artifacts, so a workspace
# that previously generated an unrelated deployment claiming 127.0.0.1:10081 (for
# example routing-matrix) legitimately rejects the variation. Skip the demonstration
# in that case; the offline planner variation test still covers disjointness.
plan_variation() {
  local overlay="$1" variation="$2" output
  if output="$("$apmpr" plan "$deployment" --with "$overlay" --variation "$variation" 2>&1)"; then
    return 0
  fi
  if grep -q "ListenerConflict" <<<"$output"; then
    echo "jas-base proof: skipping variation '$variation' demo; another generated deployment claims its host listener" >&2
    return 0
  fi
  echo "$output" >&2
  return 1
}
plan_variation "$fixture_dir/overlays/main.yaml" main
plan_variation "$fixture_dir/overlays/feature.yaml" feature
"$apmpr" up "$deployment"

compose=(
  docker compose
  --project-name apmpr--jas-base
  --project-directory "$root"
  --file "$artifact_dir/compose.yaml"
)

published_identity() {
  local service="$1"
  local port="$2"
  local address
  address="$("${compose[@]}" port "$service" "$port" | tail -n 1)"
  curl --noproxy '*' --fail --silent --show-error "http://$address/identity"
}

ui_identity() {
  local ui="$1"
  curl --noproxy '*' --fail --silent --show-error \
    --resolve "$ui.jas-base.localhost:18081:127.0.0.1" \
    "http://$ui.jas-base.localhost:18081/identity"
}

ui_a="$(ui_identity ui-a)"
ui_b="$(ui_identity ui-b)"
python3 - "$ui_a" "$ui_b" "$main_source" "$worktree" <<'PY'
import json
import sys

ui_a, ui_b = map(json.loads, sys.argv[1:3])
main, feature = sys.argv[3:]

def instances(values):
    return {value["instance"] for value in values.values()}

assert ui_a["instance"] == "ui-a"
assert ui_a["selectedProviders"]["java"]["instance"] == "jas-main"
assert instances(ui_a["selectedProviders"]["python"]) == {"ai-feature"}
assert instances(ui_a["selectedProviders"]["java"]["selectedProviders"]["python"]) == {"ai-feature"}
assert {value["instance"] for value in ui_a["selectedProviders"]["java"]["selectedProviders"]["database"].values()} == {"db-main"}
assert ui_a["selectedProviders"]["java"]["source"] == main
assert {value["source"] for value in ui_a["selectedProviders"]["python"].values()} == {feature}

assert ui_b["instance"] == "ui-b"
assert ui_b["selectedProviders"]["java"]["instance"] == "jas-feature"
assert instances(ui_b["selectedProviders"]["python"]) == {"ai-main"}
assert instances(ui_b["selectedProviders"]["java"]["selectedProviders"]["python"]) == {"ai-main"}
assert ui_b["selectedProviders"]["java"]["source"] == feature
assert {value["source"] for value in ui_b["selectedProviders"]["python"].values()} == {main}
PY

status="$($apmpr status "$deployment" --routes)"
grep -F "jas-feature path=$worktree" <<<"$status" >/dev/null
grep -F "ai-feature path=$worktree" <<<"$status" >/dev/null
grep -F "jas-main path=$main_source" <<<"$status" >/dev/null

jas_service=jas-base--jas-main--service--app
jas_id_before="$("${compose[@]}" ps --quiet "$jas_service")"
"$apmpr" bind "$deployment" jas-main ai-main
switched="$(published_identity jas-base--jas-main--service 10081)"
python3 - "$switched" <<'PY'
import json
import sys

value = json.loads(sys.argv[1])
assert {provider["instance"] for provider in value["selectedProviders"]["python"].values()} == {"ai-main"}
PY
test "$jas_id_before" = "$("${compose[@]}" ps --quiet "$jas_service")"

before_restart="$(published_identity jas-base--jas-main--service 10081)"
"$apmpr" down "$deployment"
"$apmpr" up "$deployment"
after_restart="$(published_identity jas-base--jas-main--service 10081)"
python3 - "$before_restart" "$after_restart" <<'PY'
import json
import sys

before, after = map(json.loads, sys.argv[1:])
for store in ["kv", "document"]:
    previous = before["selectedProviders"]["database"][store]
    current = after["selectedProviders"]["database"][store]
    assert previous["initialized"] and current["initialized"], (store, previous, current)
    assert current["initializationCount"] == previous["initializationCount"] + 1, (
        store,
        previous,
        current,
    )
PY

"$apmpr" down "$deployment"
"$apmpr" cleanup "$deployment" --yes
test -z "$(docker ps --all --quiet --filter label=dev.apmpr.deployment=jas-base)"
test -z "$(docker volume ls --quiet --filter label=dev.apmpr.deployment=jas-base)"
test -z "$(docker network ls --quiet --filter label=dev.apmpr.deployment=jas-base)"

"$apmpr" worktree remove "$worktree_name"
created_worktree=false
"$apmpr" source deregister "$source_name"
registered=false
rm -f "$deployment"
rm -rf "$main_source"
test "$initial_status" = "$(git -C "$root" status --porcelain=v1 --untracked-files=all)"

echo "jas-base proof: typed legacy topology, worktree sources, live group switching, task initialization, persistence, and ownership cleanup verified"
