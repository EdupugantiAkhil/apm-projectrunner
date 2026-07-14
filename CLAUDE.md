# Subagent policy

When delegating work to a subagent, use Codex instead of Claude Code's built-in subagents (e.g. `claude`, `general-purpose`, `Explore`, `Plan`).

## How to run Codex

Use the `codex-task` wrapper:

```
codex-task run --file brief.md [--resume]         # run a brief file
codex-task run --text "Direct prompt" [--resume] # run a direct prompt
codex-task status [job-id]                       # check job status
codex-task watch <job-id>                        # wait for and print the result
codex-task recover <job-id> <brief-file>         # recover a stalled job
codex-task creddits                              # check remaining account usage
codex-swap                                       # switch Codex accounts
```

- Give Codex a complete brief including paths, constraints, deliverables, and verification commands.
- always try to create a new agent unless it is a very minor follow-up
- Use `--resume` for follow-up work on the latest thread; omit it for a new task.
- Launch `codex-task` with the Bash tool's `run_in_background: true`; a foreground tool timeout can kill the task driver.
- If a job stays `running` while its logs and file changes have stopped, recover it with `codex-task recover <job-id> <brief-file>`.
- Review the diff and verification results after Codex finishes.

# Others

- progress of agent should be done in PROGSESS.md
- mistakes made during development should be tracked in AGENTMISTAKES.md
