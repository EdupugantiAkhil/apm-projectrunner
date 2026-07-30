# About Switchyard

The goal of this project is to let you run multiple instances of different parts of a
project, so you can independently work on each of them in a different branch and test
them by connecting them with other already running instances.

## The idea

Take a project with three parts:

- 1 React UI
- 1 Node.js backend
- 1 database

Normally you get one of each. If you want to work on the backend in a feature branch
while someone else's UI change is also in flight, you switch branches, restart things,
and lose whatever state you had.

With Switchyard (the APM project manager), you run **multiple instances of each of these
segments**. Several UIs, several backends, several databases — each instance built from a
different branch — all alive on the same machine at the same time.

Then you connect each segment independently. Any UI can be pointed at any backend, and
any backend at any database. You are not restricted to matching sets; you pick the
combination you want to test.

## Groups

You make a **group** with one entry of each segment:

```text
group "feature-test"
  ui       → ui on branch feature-a
  backend  → backend on branch feature-a
  database → database with the new schema

group "regression"
  ui       → ui on branch feature-a
  backend  → backend on main
  database → database with the new schema
```

That is the whole act of configuration. Once the group exists, the **auto routing
magically happens** — you do not wire up addresses, edit config files, or change ports.

Two groups can share an instance. In the example above, both groups use the same UI
instance and the same database; only the backend differs. That is exactly the comparison
you want when you are trying to find out whether a bug is in the UI or the backend.

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
`feature-test.my-project.localhost`. Opening it uses that group's UI, backend, database,
and other selected segments:

```text
feature-test.my-project.localhost
        ↓
UI feature → Backend feature → Database new
```

Inside the group, every segment still uses its existing dependency host and port. The
group address can optionally be exposed on a LAN or private network.

## Instances on more than one device

A registered device with SSH and Docker can run a container-backed provider. Switchyard
manages it and routes local consumers to its published address without application
changes. Remote consumers, routers, and cross-device sidecars are not yet supported.

## Working this way

You work on each part in its own branch, in its own instance, without coordinating with
anyone else's work in progress. When you want to test, you make a group that combines
your instance with whatever already-running instances you want to test against — no need
to start a fresh copy of everything, and no need to rebuild a whole environment for each
combination.

Change the group to change what talks to what. The instances keep running.

---

*This document describes the project as intended. Where the current implementation
differs from this description, see [DEVIATION.md](DEVIATION.md).*
