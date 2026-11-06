# Frozen finished-run records: task 268

[Task 268](https://github.com/taskqorg/agentq/issues/268) adds an AgentQ-owned observation batch for deferred CoreQ persistence under agentq.md M12.4 and M12.13. It unblocks [taskq#2428](https://github.com/taskqorg/taskq/issues/2428).

## Defect and correction

`Run::record` called `Turn::report` again after the caller reached its store. That read the remaining shared log and used `started.elapsed()`, so a later turn added its tokens and calls, and queue delay entered the duration. A CLI `agent.turn` also took a random ID, so a retry from a recovered run could count twice.

A terminal poll now freezes the CLI report and record key once. `Run::snapshot` returns the immutable `Frozen` batch without a store, lock or native call. `Frozen::save` and `restore` carry it over a restart. A CLI `agent.turn` now has an immutable ID. No product capture or second store was added.

## Before and after

```text
cargo test --locked -p agentq-runtime --test agent -- deferred_record
```

Against unchanged source at ede9e465, with only the fixture token switch and the regression added, the command exited with `the first agent.turn stays 2, not 102`, `left: Some(102)`, `right: Some(2)`. After the change, the same check passes.

## Acceptance map

- A1: `snapshot_fixes_every_runtime_at_its_terminal_poll` covers Claude, Codex and Antigravity CLI turns and a Codex queue transcript. A later record, a delay, a late cancel and another `Context` leave tokens, duration, outcome and the batch unchanged.
- A2: `snapshot_outlives_the_budget_lock_a_failed_append_and_a_restart` holds the real CoreQ budget lock of a bounded store. The terminal poll and the snapshot finish; only the append fails, and the run stays finished.
- A3: the same test appends through retry, recovery and a reopened store. It finds one turn and one call with the first payload, the same IDs and the caller's `project`, `task`, `attempt`, `actor` and `entry`. The durable form holds no prompt, result text or log name, and an invalid form is refused.
- A4: `deferred_record_keeps_the_turn_of_its_terminal_poll`.

## Evidence limits

These are fake-client checks through the real runtime and CoreQ store, not native client or platform qualification (R12). TaskQ's adoption and its budget-lock timing stay with taskq#2428. Publication requires green CI on the merge result.
