# MVP acceptance audit

This audit maps the 21 acceptance criteria in `DESIGN.md` section 14 to evidence that
exists in this tree. Test names below are Rust test function names or Vitest test
titles. Shell evidence refers to assertions in the named smoke script, not merely to
the script's final message. `scripts/phase6-proof.sh` runs the workspace and GUI tests
and the JAS smoke; `scripts/phase4-proof.sh` remains the complete routing-matrix proof.

The table deliberately distinguishes complete automation, partial automation, and
manual acceptance. A passing unit or planner test is not described as a live runtime
proof.

| # | Criterion | Existing evidence and remaining manual work |
| --- | --- | --- |
| 1 | Register a monorepo and at least two existing worktrees. | **Partial.** `examples/jas-base/smoke.sh` registers an unmanaged Git repository, creates one managed worktree, and removes both safely. `crates/switchyard-sources/src/lib.rs::inspects_repository_linked_worktree_and_dirty_categories` inspects a repository and linked worktree; `crates/switchyard-daemon/tests/api.rs::source_and_worktree_endpoints_enforce_auth_validation_and_non_destructive_errors` covers the API. No test registers a monorepo with two already-existing linked worktrees. Use manual procedure A below. |
| 2 | Define database, UI, Java, and five-service Python blocks, covering a Dockerfile, containerized legacy script, and Process Compose suite in a runner. | **Automated.** `crates/switchyard-planner/tests/real_codebase_fixtures.rs::legacy_workspace_fixture_expands_through_generic_planner_contracts` checks the expanded JAS topology. `examples/jas-base/smoke.sh` builds the Dockerfile, runs the database and UI containers, runs the Java stand-in through the legacy script, and observes all five Process Compose providers. |
| 3 | Create one database, five UI, two Python, and three Java suites. | **Manual at the exact scale.** The JAS fixture proves one database block instance, two UIs, two Python suites, and two Java suites; the routing matrix proves three UIs. No checked-in fixture or test proves five UIs and three Java suites together. Use manual procedure B. |
| 4 | Preview exactly which containers, images, volumes, and routes will be created. | **Automated planner/API; GUI rendering is component-tested.** `crates/switchyard-planner/tests/planner.rs::compose_and_manifest_are_deterministic_and_owned` and `writes_recovery_artifacts_under_generated_directory` verify deterministic artifacts. `crates/switchyard-daemon/tests/api.rs::deployment_definition_endpoints_validate_and_write_atomically` checks validate-only preview. `packages/web/src/App.test.tsx`, test `builder validates a schema-driven draft and saves it`, exercises the GUI builder preview/save flow. |
| 5 | Start and wait for health-based readiness. | **Automated.** `crates/switchyard-cli/src/runtime.rs::up_builds_then_waits_for_health` checks `docker compose up --detach --wait`; `examples/routing-matrix/smoke.sh` checks a delayed provider starts before its consumer and that `up` does not return before the delay. |
| 6 | Open each UI at a stable hostname. | **Automated for stable hostnames.** `examples/jas-base/smoke.sh` reaches both `ui-*.jas-base.localhost`; `examples/routing-matrix/smoke.sh` reaches all three `ui-*.routing-matrix.localhost`. Direct managed-profile launching is covered by CLI parsing in `crates/switchyard-cli/src/cli.rs::parses_managed_profile_open`, not by either live smoke. |
| 7 | See source path, branch, and commit behind every running instance. | **Partial automation.** `examples/jas-base/smoke.sh` asserts planned source paths in `status`; `crates/switchyard-daemon/tests/api.rs::deployment_list_and_detail_include_applied_manifest_and_reconciliation` asserts commit identity in deployment detail; `crates/switchyard-sources/src/lib.rs::inspects_repository_linked_worktree_and_dirty_categories` checks branch/commit/dirty inspection. The live smoke does not assert branch and commit text for every running instance. Manually run `switchyard status examples/jas-base/deployment.smoke.yaml` while the smoke-derived deployment is up and verify every instance row has path/ref/commit fields. |
| 8 | Select which Java and Python instances each UI uses. | **Automated topology, partial interactive coverage.** `examples/jas-base/smoke.sh` asserts `ui-a -> jas-main + ai-feature` and `ui-b -> jas-feature + ai-main`, then switches a Java consumer's Python group. `packages/web/src/App.test.tsx`, test `renders patch lanes and cables and performs a keyboard-only complete binding switch`, covers GUI group selection. The smoke does not live-switch a UI's direct Java route because `bind` changes complete groups, while the Java selection is an authored direct route. The Routing editor procedure is definition edit, validate, save, and Up. |
| 9 | Define named five-service groups assembled from one or several variants. | **Automated.** `examples/routing-matrix/deployment.yaml` defines `main-services` and `feature-services`, including a shared audit provider; `examples/routing-matrix/smoke.sh` asserts all five selected provider identities. `crates/switchyard-planner/tests/planner.rs::binding_changes_routes_without_changing_resources` covers group binding. |
| 10 | Two consumers call the same `localhost:8001` but reach different groups. | **Automated live.** `examples/routing-matrix/smoke.sh` observes backend-1 and backend-2 using the identical fixed slots with feature and main providers respectively. The fixed contract is in `examples/routing-matrix/contract.yaml`. |
| 11 | Switch a complete consumer group without restarting its application container. | **Automated live.** Both smoke scripts capture application container IDs, invoke `switchyard bind`, assert every group slot changes, and assert the application IDs are unchanged. |
| 12 | Assign and persist custom domains through the native router. | **Automated.** Both smoke scripts route their custom `.localhost` domains through the host gateway. `crates/switchyard-daemon/tests/api.rs::applied_domains_bindings_and_deleted_database_recovery_survive_daemon_restart` verifies the applied custom domains persist in SQLite across daemon reconstruction. |
| 13 | Recover observed deployment and route state through SQLite and Docker labels after control-plane restart. | **Automated, split across layers.** `crates/switchyard-daemon/tests/api.rs::applied_domains_bindings_and_deleted_database_recovery_survive_daemon_restart` covers restart and manifest recovery; `failed_live_binding_versions_and_rollback_history_survive_restart` covers route state/history. `crates/switchyard-state/src/lib.rs::deleted_database_rebuilds_observed_state_without_inventing_applied_state` injects owned Docker-label observations and proves observation recovery without an invented apply. Docker-label collection by a restarted real daemon is an integration boundary, not exercised by the daemon test. |
| 14 | View combined and per-service logs. | **Partial; live manual check required.** `crates/switchyard-cli/src/main.rs::log_target_resolves_all_instance_services_from_manifest` checks per-instance target expansion. `packages/web/src/App.test.tsx`, test `renders live SSE fixtures in the operation drawer`, checks GUI log/event rendering; instance cards have a target-specific Logs button. No Docker smoke asserts combined and targeted log output. Use manual procedure C. |
| 15 | Stop without deleting database state. | **Automated live and unit-level.** `crates/switchyard-cli/src/runtime.rs::down_does_not_delete_volumes` proves `down` omits `--volumes`; `examples/jas-base/smoke.sh` compares database initialization state across down/up; `destructive_cleanup_requires_confirmation` proves cleanup needs confirmation. |
| 16 | Perform all common operations from both CLI and schema-driven GUI. | **Satisfied by the parity matrix below.** Create, inspect, switch, start, stop, cleanup, logs, sources, worktrees, operation cancellation (`switchyard operation cancel` added at exit-gate review), and managed-profile opening (instance-card **Open** button added at exit-gate review) have CLI/API/GUI paths. Vitest covers builder, inspect, switching, routing definition edits, logs/events, and dirty-worktree confirmation, but not every command-bar button end to end. |
| 17 | Replace JAS with an unrelated fixture without core/API/CLI/GUI changes. | **Automated planner-level.** `crates/switchyard-planner/tests/real_codebase_fixtures.rs::unrelated_fixture_bundles_use_the_same_deterministic_planning_path` plans JAS and routing-matrix identically; `production_crate_identifiers_do_not_name_the_legacy_fixture` guards against fixture coupling. The two independent live smoke scripts provide runtime evidence, though they are not run in one proof command. |
| 18 | Apply two overlay sets to one base and run both concurrently without source edits. | **Partial.** `crates/switchyard-planner/tests/real_codebase_fixtures.rs::overlays_create_disjoint_deterministic_variation_plans` and `crates/switchyard-planner/tests/planner.rs::secrets_are_redacted_and_variations_are_disjoint` prove distinct names, Compose projects, artifacts, and hashes. `examples/jas-base/smoke.sh` plans both variations and verifies the source tree is unchanged, but does not start them concurrently; the JAS host router is intentionally a singleton fixed-port resource. Use manual procedure D with a base deployment that has no shared fixed host listener. |
| 19 | Route three unchanged browser UIs calling `localhost:10081` independently by Origin, extension header, or managed profiles. | **Automated live at the router boundary via Origin.** `examples/routing-matrix/smoke.sh` loads three UI identities, then uses `curl` to send their three Origins to the same `http://localhost:10081` and asserts independent backend selection. It does not drive a real browser. Header behavior is additionally covered by `crates/router-core/tests/engine.rs::explicit_header_identity_can_be_compiled` and `header_and_origin_must_select_the_same_provider`; managed proxy authentication is covered by `crates/router-pingora/tests/http_proxy.rs::managed_profile_listener_requires_and_strips_proxy_credentials`. The extension and managed-browser end-to-end flows remain manual; the live Origin path exercises the route behavior required by the criterion. |
| 20 | Reject an ambiguous browser request with an actionable diagnostic. | **Automated.** `crates/router-core/tests/ambiguity.rs::duplicate_route_keys_fail_closed` rejects ambiguous configuration with `AmbiguousRoute`. At request time, `crates/router-pingora/tests/http_proxy.rs::browser_routes_enforce_origin_and_answer_cors_preflight` asserts an unidentified request receives HTTP 400 and `missing_route_identity`. |
| 21 | Duplicate a backend source when two UIs need different downstream groups. | **Automated planner validation and live topology.** `crates/switchyard-planner/tests/planner.rs::backend_group_invariant_requires_duplicate_instances_for_different_groups` emits the duplication guidance. The routing matrix runs backend-1 and backend-2, sourced independently, with different downstream bindings. |

## Manual procedures

These procedures fill only the gaps identified above. Run them from the repository root
on a disposable Git repository with Docker available.

### A. Existing monorepo with two worktrees (criterion 1)

1. In a test monorepo, create two existing linked worktrees with
   `git worktree add ../accept-main HEAD` and
   `git worktree add -b accept-feature ../accept-feature HEAD`.
2. Run `switchyard source register accept-repo /absolute/path/to/monorepo`.
3. Register the already-existing worktree paths independently with
   `switchyard source register accept-main /absolute/path/to/accept-main` and
   `switchyard source register accept-feature /absolute/path/to/accept-feature`.
4. Run `switchyard source list` and open **Sources** in `switchyard gui`.
5. Expected: all three registrations are `unmanaged`; each shows its repository root,
   branch/ref, commit, dirty state, and exact path. Deregister all three. Deregistration
   must remove only Switchyard's records and leave all directories and Git refs intact.

### B. Exact 1/5/2/3 scale (criterion 3)

1. Copy `examples/jas-base/deployment.yaml` outside the repository fixture directory.
2. Keep `db-main`, `ai-main`, and `ai-feature`; add `jas-third` from `jas-service`; add
   `ui-c`, `ui-d`, and `ui-e` from `ui`. Give each new instance a valid source and add
   its direct Java route, complete AI-group binding, `uiRoutes` entry, custom-domain
   destination, provider, host upstream, and browser route. Use distinct UI domains;
   Java instances stay private and may all consume the fixed namespace-local slots.
3. Run `switchyard validate <copy>` and `switchyard plan <copy>`.
4. Expected: validation succeeds; the preview contains one database instance (its two
   database services), five UI instances, two Python/Process Compose suite instances,
   and three Java instances, with no resource-name or listener collision.
5. Run `switchyard up <copy>`, then `switchyard status <copy> --routes`; expect all
   services healthy and all five UI routes present. Finally run `switchyard down <copy>`
   and `switchyard cleanup <copy> --yes`.

### C. Live combined and per-instance logs (criterion 14)

1. Start `examples/jas-base/smoke.sh` interactively up to its first successful identity
   checks, or apply an equivalent disposable copy and leave it running.
2. In terminal one run `switchyard logs <deployment.yaml>`; generate one UI identity
   request and expect the combined stream to show lines from more than one Compose
   service.
3. In terminal two run `switchyard logs <deployment.yaml> jas-main`; generate another
   request and expect only services expanded from `jas-main`.
4. In the GUI select the deployment, use top-level **Logs**, then the **Logs for
   jas-main** instance button. Expected: operation progress appears and log events are
   visible in the Events & logs drawer, with the instance request scoped to `jas-main`.

### D. Concurrent overlay variations (criterion 18)

1. Use a copy of a base deployment with no fixed host listener shared by variations.
   Prepare `overlays/main.yaml` and `overlays/feature.yaml` that select the same base
   instance but set visibly different non-secret values or injected files.
2. Run:

   ```sh
   switchyard up deployment.yaml --with overlays/main.yaml --variation main
   switchyard up deployment.yaml --with overlays/feature.yaml --variation feature
   switchyard status deployment.yaml --variation main
   switchyard status deployment.yaml --variation feature
   ```

3. Expected: both deployments are running concurrently as `<name>--main` and
   `<name>--feature`, with different Compose projects/resources and their respective
   resolved values. `git status --porcelain` for every source worktree remains unchanged.
4. Stop both with matching `down --variation ...` commands and clean each generated
   variation explicitly if persistent volumes were created.

## Criterion 16 CLI/API/GUI parity

“Common” here means deployment creation and definition editing, preview/start/inspect,
routing changes, logs, stop/cleanup, source/worktree handling, operation cancellation,
and browser opening. Command endpoints use `POST /api/v1/commands/<kind>` unless noted.

| Operation | CLI | API | GUI flow | Status |
| --- | --- | --- | --- | --- |
| Create/validate definition | `switchyard validate FILE`; author a file | `POST /api/v1/deployments` (`validateOnly`, then create) | **New deployment** -> schema form -> **Validate draft** -> **Save deployment** | Present; GUI test: `builder validates a schema-driven draft and saves it`. |
| Edit domains/browser routes/profiles | edit YAML, `switchyard validate FILE` | `GET`/`PUT /api/v1/deployments/{name}/definition` with expected hash | Deployment **Routing** -> load -> edit -> validate -> apply definition edit | Present; GUI test: `shows a domain YAML diff and validates before definition PUT`. |
| Preview | `switchyard plan FILE` | `/api/v1/commands/plan`; validate-only definition preview | Command bar **Plan**; builder plan preview | Present; command-bar invocation lacks a dedicated UI test. |
| Start/apply | `switchyard up FILE` | `/api/v1/commands/apply` | Command bar **Up**, or **Run Up after saving** | Present; builder follow-up is tested indirectly, live GUI apply is manual. |
| Inspect status/routes/source | `switchyard status FILE --routes`; `switchyard routes FILE` | `/api/v1/commands/status`, `/commands/routes`, `GET /deployments/{name}`, `GET /deployments/{name}/routes` | deployment rail, inspector, instance cards, patch bay, active-routes table | Present; GUI test: `renders deployment identity, state, routes, domains, and bindings`. |
| Switch complete group | `switchyard bind FILE CONSUMER GROUP [--transition ...]` | `/api/v1/commands/bind` | patch bay -> compatible group -> complete preview -> transition -> **Apply complete change** | Present; keyboard-only GUI test covers it. |
| Combined/per-instance logs | `switchyard logs FILE [INSTANCE]` | `/api/v1/commands/logs` with optional `target` | command bar **Logs** or instance-card **Logs**; drawer shows events | Present, but live output requires manual procedure C. |
| Stop preserving volumes | `switchyard down FILE` | `/api/v1/commands/down` | command bar **Down**, type deployment name | Present; safety unit test exists, no dedicated GUI click test. |
| Destructive cleanup | `switchyard cleanup FILE --yes` | `/api/v1/commands/cleanup` with `confirmed: true` | command bar **Cleanup**, type deployment name | Present; safety unit tests exist, no dedicated GUI click test. |
| Register/list source | `switchyard source register/list` | `GET`/`POST /api/v1/sources` | **Sources** -> **Register unmanaged** and source cards | Present. GUI has no deregister action; deregistration is not needed for the common deployment lifecycle but is a CLI/API-only maintenance action. |
| Create/remove worktree | `switchyard worktree create/remove` | `POST`/`DELETE /api/v1/worktrees` | **Sources** -> **Create worktree**; managed source **Remove** with dirty second step | Present; GUI dirty-removal test exists. |
| Cancel operation | `switchyard operation cancel OPERATION_ID` | `POST /api/v1/operations/{id}/cancel` | **Operations** -> **Cancel** | Present. CLI parsing test: `parses_operation_cancel`; daemon cancellation behavior is covered by the existing API cancellation tests. |
| Open managed browser profile/UI | `switchyard open FILE UI` | `/api/v1/commands/open` | Instance card **Open** (shown for instances with a managed profile) | Present. The GUI button reuses the same `open` command; launching a real browser remains manual procedure D territory. |

Both former criterion-16 gaps were closed during the exit-gate review: instance cards
expose **Open** for managed-profile instances, and `switchyard operation cancel`
cancels an arbitrary operation through the daemon API.
