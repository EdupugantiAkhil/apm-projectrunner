---
name: switchyard-project
description: Operate, inspect, and safely modify a Switchyard local development topology. Use when working with deployment.yaml, overlays, registered sources or devices, instances, service-group pairings, lifecycle commands, generated plans, or Switchyard runtime diagnostics in this project.
---

# Switchyard Project

Treat `deployment.yaml` as authored desired state and `.switchyard/` as generated or
runtime state. Do not hand-edit files below `.switchyard/`.

## Workflow

1. Read `deployment.yaml` and any referenced overlays before changing topology.
2. Use `switchyard validate deployment.yaml` after authored edits.
3. Use `switchyard plan deployment.yaml` to preview generated resources and routes.
4. Use `switchyard tui .` for interactive source, device, instance, pairing, script, and
   lifecycle management.
5. Use `switchyard up deployment.yaml`, `switchyard status deployment.yaml`, and
   `switchyard down deployment.yaml` for explicit shell workflows.

Preserve unrelated YAML and comments. Add sources under `spec.sources` before referring
to them from an instance. Change consumer pairings as complete compatible service groups;
do not invent partial bindings. Prefer `down` for normal stopping because it preserves
volumes. Run `cleanup --yes` only when the user explicitly intends destructive cleanup.

When a command fails, report its concrete output and inspect status or generated plans
before changing desired state. Never copy passwords, private keys, or tokens into the
deployment definition.
