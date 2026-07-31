# About Switchyard

The goal of this project is to run multiple source-backed instances at once, so you can
work on different branches independently and test them in groups with other already
running instances.

## The idea

Take a project that runs three services:

- 1 React UI
- 1 Node.js backend
- 1 database

Normally you get one of each. If you want to work on the backend in a feature branch
while someone else's UI change is also in flight, you switch branches, restart things,
and lose whatever state you had.

With Switchyard (the APM project manager), you run **multiple source-backed instances**.
Several UIs, several backends, several databases — each instance built from a selected
branch — all alive on the same machine at the same time. Instances in different groups
may use the same branch.

Then you place instances into groups. You are not restricted to matching branch sets; you
pick the combination you want to test.

## Groups

You make a **group** as an ordered list of instances:

```text
group "feature-test"
  ui       → ui-feature-test on branch feature-a
  backend  → backend on branch feature-a
  database → database-new with the new schema

group "regression"
  ui       → ui-regression on branch feature-a
  backend  → backend on main
  database → database-new with the new schema
```

That is the whole act of configuration. Once the group exists, the **auto routing
magically happens** — you do not wire up addresses, edit config files, or change ports.

An instance that makes group-routed outbound calls belongs to **at most one group**.
Receiver-only instances, such as a database, may be shared. To use the same sender in
two groups, create two instances from the same code.

One group may contain several instances listening on the same port. Switchyard warns and
routes to the first listed instance; reorder the list to change the priority.

Use `disabled: [instance-name]` to temporarily exclude a member from one group. The
instance keeps running and retains its list position, but that group does not route to it.

## Routing works as if there was no isolation

The instances are isolated from each other — that is what makes running several copies
possible at all. But the network routing works **as if there was no isolation at all**.

Every UI still calls the backend at the address it was always written to call. Every
backend still calls the database at its usual address. Nothing in the code changes, no
port numbers change, no `.env` file gets edited. Each instance behaves as though it is
the only copy on the machine and its dependencies are exactly where it expects them.

The routing layer is what makes that true. Inside a group, a UI's call to its usual
backend address arrives at that group's backend. The same call from a UI in a different
group arrives at a different backend. The applications never find out.

## Open a group by its address

Each group has one stable custom address, such as
`feature-test.my-project.localhost`. Opening it uses that group's selected instances:

```text
feature-test.my-project.localhost
        ↓
UI feature → Backend feature → Database new
```

Inside the group, every instance still uses its existing dependency host and port. The
group address can optionally be exposed on a LAN or private network.

## Instances on more than one device

A registered device with SSH and Docker can run a container-backed provider. Switchyard
manages it and routes local consumers to its published address without application
changes. Remote consumers, routers, and cross-device sidecars are not yet supported.

## Working this way

You work on each change in its own branch, source worktree, and instance. Build a group,
move a sender, or create another instance from the same code. Application addresses and
ports never change.

Change group membership to change what talks to what. Moved instances keep running while
their routing context is replaced.

---

*This document describes the project as intended. The
[V2 roadmap](../v2-roadmap.md) tracks the work required to align the implementation.*
