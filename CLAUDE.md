# Claude Code in this repo

Read `AGENTS.md` first; its rules apply. This file adds how a session works.
@AGENTS.md

## Use the graph (owner's standing opt-in)
- Implementation runs through the `graph` workflow (`.claude/workflows/graph.js`, skill `graph`): the owner has opted in to
  multi-agent orchestration for this repository, for every task, without asking again.
- The main session is the **planner**: it researches, designs, decides, and writes the task graph
  (`{goal, nodes: [{id, deps, files, task, check}], final_check}`). It does not write the implementation itself. Exceptions:
  a fix of a few lines after a refutation, and docs rows (`docs/bugs.md`, `docs/decisions.md`).
- **Executors** (`.claude/agents/executor.md`, Haiku) do one node each; a **fresh verifier** (`.claude/agents/verifier.md`) checks
  every node and the whole change (rule 5). The planner commits only after the final verdict.
- Write each node so an executor needs no other context: the files, the exact change (names, types, defaults, formulas), and a
  check that runs in minutes. Design-heavy code (new maths, shader passes) is split into small, fully specified nodes, not
  done by the planner instead.

## Branches and PRs
- One new branch per task, off the latest `main` (`git fetch origin main`). Never reuse a branch whose PR is merged; never force-push
  over one. `main` is protected: changes land through PRs.
- Before starting, check open branches and PRs for work in flight on the same files; do not collide with another session.
- Push the branch when the final verdict passes; open the PR when the owner asks (they usually do).
- Every PR body carries the owner's visual check list, so they can check the rest by eye on their laptop: this PR's items and
  every open `docs/handover.md` row (both sessions'). The owner does not run tests; the cloud verifier does, and the PR says in one
  line that they passed. Each item gives the commands that render shots (PNGs to open side by side) or open the live window, what
  the eye should see, the same scene with the feature off (the red arm as a picture), and the look call. Do not render the list's
  shots in the cloud: the owner renders them locally. Keep the list current when the PR or the handover changes.

## Reporting
- Intermediate messages: terse or none. No narration of steps.
- Only the final result is reported, labelled (e.g. **Result**), and kept short.

## GPU testing
- GPU checks (Metal timing, goldens, look) run locally on the owner's machine, not in the cloud.
- Each change that needs one leaves a handover in `docs/handover.md`: the commands to run and what to look for.
- The owner marks an item done or deletes it once approved; otherwise sends it back with feedback, and the item stays open.

