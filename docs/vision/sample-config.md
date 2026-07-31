# A sample configuration

This is one complete deployment, annotated. It is the shape [ABOUT.md](ABOUT.md) and
[user_flow.md](user_flow.md) describe, in one file, so you can see how the pieces fit before
reading the steps that introduce them one at a time.

The scenario is the one from step 9: a UI and a backend, each with a main branch and a feature
branch, sharing one database. Two named combinations — `feature-test` and `regression` — differ
only in which checkouts they use, and you switch between them by opening a different address.

```yaml
apiVersion: switchyard.dev/v1alpha2
kind: Deployment
metadata:
  name: comparison

spec:
  # ── Repositories: where the code comes from ─────────────────────────────────
  # Named once. Every worktree below is backed by this one clone, which `up` makes
  # if it is not there. No path — you never work in the clone itself, so where it
  # sits is Switchyard's business. Use `clone: ~/work/monorepo` instead of `url:`
  # to point at a clone you already have, which is then read and never modified.
  repositories:
    monorepo:
      url: git@github.com:acme/monorepo.git

  # ── Sources: which checkout each instance runs ──────────────────────────────
  # Every source is a worktree: a repository, a ref, and where it lives. If the
  # path is not there, `up` creates it. Add a fifth branch to compare by adding one
  # line. The clone always lives elsewhere, so a source directory is only ever a
  # worktree — nothing has to work out which kind of thing it is looking at.
  sources:
    ui-main:          { repository: monorepo, ref: main,        path: ./sources/ui-main }
    ui-feature:       { repository: monorepo, ref: feature-a,   path: ./sources/ui-feature }
    backend-main:     { repository: monorepo, ref: main,        path: ./sources/backend-main }
    backend-feature:  { repository: monorepo, ref: backend-fix, path: ./sources/backend-feature }
    infra:            { repository: monorepo, ref: main,        path: ./sources/infra }

  # ── Startup profiles: how each kind of part starts ──────────────────────────
  # Written once per kind of part, not once per branch. A profile says how to start
  # the thing and which port it listens on. It does not say what talks to what —
  # that is the group's job, below.
  blocks:
    react-ui:
      services:
        app:
          execution:
            type: container
            image: node:22
            command: ["npm", "run", "dev", "--", "--host", "0.0.0.0", "--port", "5173"]
          publish: [5173]
          probe: { type: http, path: /health, port: 5173 }

    java-backend:
      # Parameters are the knobs you vary per instance.
      parameters:
        LOG_LEVEL: { default: info }
      services:
        app:
          execution:
            type: container
            image: eclipse-temurin:21
            command: ["./gradlew", "bootRun"]
          publish: [8080]
          probe: { type: http, path: /actuator/health, port: 8080 }

    postgres:
      services:
        db:
          execution:
            type: container
            image: postgres:16
            environment: { POSTGRES_PASSWORD: local }
          volumes: [{ name: pgdata, target: /var/lib/postgresql/data }]
          probe: { type: tcp, port: 5432 }

  # ── Instances: one checkout, run through one profile ────────────────────────
  # `ui-1` and `ui-2` are the same program from different branches. Nothing about
  # the profile changes; only the source does.
  instances:
    - { name: ui-1, block: react-ui, source: ui-main,    address: ui-1.comparison.localhost }
    - { name: ui-2, block: react-ui, source: ui-feature, address: ui-2.comparison.localhost }

    - { name: backend-1, block: java-backend, source: backend-main }
    - { name: backend-2, block: java-backend, source: backend-feature,
        parameters: { LOG_LEVEL: debug } }
    - { name: backend-canary, block: java-backend, source: backend-feature }

    - { name: db-new, block: postgres, source: infra }

    # An external instance: something already running that Switchyard routes to but
    # never starts. `external:` is the host, `ports:` the list — 9200 inside the
    # group reaches 9200 there. Ranges work too: ports: ["8000-8010"].
    - { name: staging-es, external: search.staging.internal, ports: [9200, 9300] }

  # ── Groups: named combinations ──────────────────────────────────────────────
  # A group is a list of members and an address, and that is the whole act of
  # configuration. There is no second section saying what connects to what:
  # membership is the connection. Every member shares one localhost, so the UI's
  # call to 127.0.0.1:8080 reaches backend-1 in `feature-test` and backend-2 in
  # `regression`, because that is who listens on 8080 in each group.
  #
  # Move `ui-1` to the other list and the same running UI talks to backend-2
  # instead — no restart, no rebuild, no edited source.
  #
  # `db-new` is in both groups and can receive traffic in both. If a shared member
  # originates a loopback call itself, Switchyard rejects that call as ambiguous;
  # duplicate a sender when it needs its own outbound group context.
  groups:
    feature-test:
      address: feature-test.comparison.localhost
      instances: [ui-1, backend-1, backend-canary, db-new]
      # Keeps running and keeps its priority position, but this group ignores it.
      disabled: [backend-canary]

    regression:
      address: regression.comparison.localhost
      instances: [ui-2, backend-2, db-new, staging-es]

# ── Run actions: project-level scripts ────────────────────────────────────────
# DEFERRED TO V3 — shown for completeness; the current build uses a different
# format (see the differences section below).
#
# A flat name-to-command map, exactly the `package.json` scripts model. The runner
# puts Switchyard's own binary directory on PATH and exports $SWITCHYARD_PROJECT
# and $SWITCHYARD_BUNDLE, so the commands stay short. Like `npm run`, the
# convenience is the environment rather than the schema — every value is an
# ordinary shell command in an ordinary shell.
#
# The open question that deferred it: a script runs in your shell, so it has no
# way to reach the deployment. Published ports are ephemeral and the group's
# shared localhost only exists inside sidecars, so `smoke.sh --target
# feature-test` below cannot actually hit anything. Two ideas, neither settled:
# export the group addresses as environment variables, or add `switchyard exec
# backend-1 -- <cmd>` to run inside a member's namespace.
scripts:
  dev-up: switchyard up $SWITCHYARD_BUNDLE --with overlays/dev.yaml --set LOG_LEVEL=debug
  smoke:  ./scripts/smoke.sh --target feature-test
  status: switchyard status $SWITCHYARD_BUNDLE
```

## Reading it

**Four sections do the real work.** Sources say which checkouts exist, blocks say how a kind of
part starts, instances pair the two, and groups name the combinations. Everything else is detail.

**A source is a repository, a ref, and a path — and missing paths get created.** If
`./sources/ui-feature` is not there, `up` runs `git worktree add` for you; if the clone itself is
not there, it clones first. So the file is enough on its own: hand someone this `deployment.yaml`
and they get the whole tree, rather than a list of directories they have to assemble by hand.

**Every source is a worktree, and the clone always lives somewhere else.** Two populations of
directory that never overlap: clones under `.switchyard/clones/` (or your own path, if you point
at one you already have), worktrees wherever you author them. Nothing ever has to work out which
kind a directory is, and the question of what Switchyard may modify is settled once at the
repository level rather than per source.

**Only sources carry a path, and that asymmetry is deliberate.** A source directory is something
you use: you open it in an editor, run commands in it, point tooling at it. It has to be yours to
choose and written down where you can see it. The clone is bookkeeping you never work in, so where
it sits is Switchyard's business. Declaring the repository once is what makes "all backed by one
clone" true in the file rather than only in prose.

**Nothing declares what talks to what.** There is no wiring section and no connections section,
because a group *is* the connection — membership is the whole statement, said once. Every member
of a group shares one localhost, so a call to `127.0.0.1:8080` reaches
whichever member of that group listens on 8080. This works because your code already calls the
port the service actually listens on — the address the UI expects and the port the backend binds
are the same number — so there is nothing left for you to say. Sidecars observe active listener
ports at runtime. If two members of one group listen on the same port,
you get a warning and the first listed wins, the same rule as
[address collisions](user_flow.md#address-collisions-and-who-wins).

**Disabled members keep their place.** A name in a group's `disabled:` list stays running but
is ignored by that group. Remove the name to restore it at the same `instances:` position and
therefore the same routing priority.

**`scripts:` and `blocks:` are not the same kind of thing.** Only `scripts:` follows the
`npm run` model — a flat name-to-command map, because a run action genuinely is one line of shell
and nothing more. A startup profile cannot collapse to that shape: it carries an execution, ports,
volumes, a probe, parameters, and possibly several services. The two do meet inside a block's
`command:` — `["npm", "run", "dev"]` above is an npm script. The profile says which script and in
what container; npm says what the script does.

**The same profile serves both branches.** `react-ui` is written once. `ui-1` and `ui-2` differ
only in `source:`. This is what makes adding a third branch to compare a two-line change rather
than a copy of the whole block.

**Not every member is something Switchyard starts.** `staging-es` is an *external instance*: a
host and a list of ports, no block and no source. A member of `regression` reaching
`search.staging.internal:9200` gets there through the group like any other dependency, but
nothing is launched or stopped — Switchyard only routes. The same form covers a Postgres
installed natively on your machine (`external: 127.0.0.1, ports: [5432]`), a service on a
teammate's box, or a shared staging cluster. `ports:` takes single ports and inclusive ranges
(`["8000-8010"]`) in one list, and maps port-for-port: 9200 inside the group is 9200 there.
This is the one place a port is always written by hand, because an external has no `publish:`
or image metadata to learn from.

**Addresses sit on the thing they name.** `ui-1.comparison.localhost` is on the instance;
`feature-test.comparison.localhost` is on the group. Open the instance address to look at one UI;
open the group address to get the whole combination. Both are in
[step 9](user_flow.md#step-9--addresses-open-a-group-or-an-instance-by-name).

**The comparison is one click.** `feature-test` and `regression` differ only in which checkouts
they name. Open both in a browser and you are looking at two builds of your product side by side,
with one database underneath.

## What you do not write

- **Any statement of what connects to what.** No slot list, no dependency declarations, no
  capability names, no bindings or routes section. Group membership is the entire statement,
  and there is no second place that could contradict it.
- **Any address rewriting in your application.** No `--api-url` flag, no `.env` per branch, no
  per-instance config file. If your app already works when run by hand, it works here.
- **Unique ports.** `backend-1` and `backend-2` both listen on 8080 and never collide. Each
  instance has its own loopback, so 8080 means something different inside each one.
- **A front door.** No member of a group is designated the entry point. The group address reaches
  the group and routing decides which member answers.

## Where today's schema differs

The sample above is the intended shape. Six things about the current build differ, all tracked
in [docs/v2-roadmap.md](../v2-roadmap.md):

1. **There is no `repositories:` section, and nothing is ever created.** Today a source is
   `{ type: worktree, path, repository, ref }`, with the repository path repeated on every source
   that shares it. Those git fields are descriptive rather than operative: they validate, but
   nothing in the planner clones or creates a worktree, so every directory must already exist
   before `up` — validation fails outright if one does not. Part 2c names the repository once and
   makes `up` create what is missing.
2. **Routing is declared, not discovered.** Today each profile must carry `provides:` (what it
   offers, and on which port) and `consumes:` (each dependency, with the address the application
   already calls). Omit them and the deployment validates but produces **no router sidecars at
   all** — every instance runs isolated. Part 2b removes both fields and makes group membership
   sufficient, as the sample shows. Routing is port-for-port; remapping is not a retained
   capability/slot escape hatch.
3. **Connections are authored twice more.** A `bindings:` map names a group per consumer, and a
   `routes:` map can name a provider per slot directly. Both restate what group membership
   already says, and both can contradict it. Part 2d deletes them. A member shared across groups
   can receive in all of them; an outbound loopback call from that shared member is rejected as
   ambiguous.
4. **There are no external instances.** Every instance must have a block and a source, so
   anything already running outside Switchyard — a natively installed database, a shared staging
   service — cannot be a group member at all. Part 2e adds the `{ name, external, ports }` form.
5. **`scripts:` is not the current shape, and is deferred.** Run actions live in
   `.switchyard/run-scripts.yaml` as seven-field records. They are deliberately outside the V2
   roadmap until a shell action has a settled way to reach the deployment it is about.
6. **Addresses still require a hand-authored `hostRouter:` block.** Declaring `address:` on a
   group or instance is accepted, but validation then insists on a `hostRouter:` section
   alongside it — listeners, providers, routes, and an explicit-header `browserRoutes` entry per
   addressed UI — plus a `hostUpstreams:` map from each router provider back to a published port.
   That is roughly sixty lines the sample omits. Deriving it from the addresses is Part 3.

Bare instance names in `instances:` do work today, and resolve to the single member service.
Writing `ui-1/app` is also accepted, and is what you need when one instance runs several services
and you mean a particular one; the repository's own examples use that longer form throughout.

For a deployment that validates against the build as it stands today, see
`examples/routing-matrix/deployment.yaml`.
