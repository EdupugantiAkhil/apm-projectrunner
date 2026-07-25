# Subagent policy

When delegating work to a subagent, use sonnet (its mapped to gpt-5.6-sol)

## Delegation workflow

1. **Split large work into reviewable parts.** Give each part its own brief, run the
   parts sequentially, and commit each part separately after it is verified so it is
   easy to review and revert.
2. **Write a complete brief** for each part: repository path, context files to read,
   exact deliverables, constraints (crates it may and may not touch), verification
   commands, and documentation/bookkeeping expectations. Tell Codex not to commit;
   the reviewer commits after verification.
3. **Start a new agent for each new part** . Use resume only for
   a very minor follow-up on the latest thread (e.g. fixing a nit from review).

## After Codex finishes

- **Give feedback**: send review findings back as a follow-up brief, or fix very
  small issues directly yourself.
- **Commit the reviewed part** before starting the next one.

# Progress and mistake tracking

- Record implementation and verification progress in `PROGRESS.md`.
- Record mistakes, corrections, and lessons in `AGENTMISTAKES.md`.
