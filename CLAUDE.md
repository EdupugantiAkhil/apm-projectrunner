# Subagent policy

When delegating work to a subagent, use Codex instead of Claude Code's built-in
subagents (e.g. `claude`, `general-purpose`, `Explore`, `Plan`). This overrides the
"do not delegate to subagents" rule in `AGENTS.md`, which is written for agents that
lack the `codex-task` wrapper.

## How to run Codex

Use the `codex-task` wrapper:

```
codex-task run --file brief.md [--resume]        # run a brief file
codex-task run --text "Direct prompt" [--resume] # run a direct prompt
codex-task status [job-id]                       # check job status
codex-task watch <job-id>                        # wait for and print the result
codex-task recover <job-id> <brief-file>         # recover a stalled job
codex-task creddits                              # check remaining account usage
codex-swap                                       # switch Codex accounts
```

## Delegation workflow

1. **Split large work into reviewable parts.** Give each part its own brief, run the
   parts sequentially, and commit each part separately after it is verified so it is
   easy to review and revert.
2. **Write a complete brief** for each part: repository path, context files to read,
   exact deliverables, constraints (crates it may and may not touch), verification
   commands, and documentation/bookkeeping expectations. Tell Codex not to commit;
   the reviewer commits after verification.
3. **Start a new agent for each new part** (omit `--resume`). Use `--resume` only for
   a very minor follow-up on the latest thread (e.g. fixing a nit from review).
4. **Launch `codex-task` with the Bash tool's `run_in_background: true`**; a foreground
   tool timeout can kill the task driver.
5. **If a job stays `running`** while its logs and file changes have stopped, recover
   it with `codex-task recover <job-id> <brief-file>`.

## After Codex finishes

- **Re-run verification locally.** The Codex sandbox has no network, cannot create
  sockets, and cannot reach Docker, so its "blocked" test results are expected — run
  the full workspace tests, clippy `-D warnings`, rustdoc, and any Docker-based proof
  scripts yourself before trusting or committing the result.
- **Read the diff, not just the stats.** Check that the change stayed inside the
  brief's scope and review the risky paths line by line (error handling, rollback,
  locking, auth, resource cleanup). Codex's own tests passing is not a review.
- **Give feedback**: send review findings back as a follow-up brief, or fix very
  small issues directly yourself.
- **Commit the reviewed part** before starting the next one.

# Progress and mistake tracking

- Record implementation and verification progress in `PROGRESS.md`.
- Record mistakes, corrections, and lessons in `AGENTMISTAKES.md`.
