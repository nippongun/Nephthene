@AGENTS.md

## Reporting
- Intermediate messages: terse or none. No narration of steps.
- Only the final result is reported, labelled (e.g. **Result**), and kept short.

## GPU testing
- GPU checks (Metal timing, goldens, look) run locally on the owner's machine, not in the cloud.
- Each change that needs one leaves a handover in `docs/handover.md`: the commands to run and what to look for.
- The owner marks an item done or deletes it once approved; otherwise sends it back with feedback, and the item stays open.
