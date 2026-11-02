# Backward history paging and 16 MiB records: task 204

Source: [task 204](https://github.com/taskqorg/agentq/issues/204).
The owner goal is: "AgentQ conversation records support bounded backward paging and records up to 16 MiB, matching the taskq Workspace D4 limit, so taskq Conversations can render full history."

## Applied rules and scope

AgentQ R13 / §12 (M12.1 and M12.3) owns the read-only history API.
P3 is applied by editing the linked RUNTIME history contract before implementation.
The task's A2 changes the prior 1 MiB record limit to 16 MiB; the existing oversized
record regression moves to the new limit and retains refusal/cursor-preservation protection.
SpecQ Testing's development policy is applied with failure-before / pass-after checks.
No native turn, client configuration, completion parser, grant, optimizer, dependency,
consumer repository or working installation changes.

## Acceptance evidence

- A1: `History::poll_before` uses the existing checkpoint as an exclusive backward
  boundary, starts a fresh cursor at EOF, returns at most 128 ascending records / 16 MiB,
  and saves the preceding boundary for recovery. Forward/backward IDs and adapter output
  match on all seven retained transport fixtures. Tests cover 0, 1, 128, 129, 256 and
  300 records, append during paging, EOF, recovery, invalid boundaries, prefix rewrites,
  truncation and file replacement.
- A2: source records of 1 MiB + 1 byte and exactly 16 MiB read back byte-for-byte;
  16 MiB + 1 byte fails with `history record exceeds 16 MiB`, without advancing.
  The source-record limit includes the JSONL newline. Both directions keep a maximum-size
  row intact between adjacent rows, and expose partial-tail state without losing records.
- A3: the existing history integration suite adds three regression checks, updates the
  oversized corruption case, and reuses the seven-format fixture loop for direction parity.
  Tests generate large records in temporary files rather than storing large fixtures.

Before implementation, `cargo test --locked -p agentq-runtime --test history history_accepts_large_records_without_loss`
failed with `history record exceeds 1 MiB` on the first 1 MiB + 1 byte record.
The backward regression selection failed to compile with E0599 because `poll_before`
did not exist. This establishes the missing API, not a prior runnable backward behavior.
After implementation, `cargo test --locked -p agentq-runtime --test history` passed
all 18 tests, including the new boundaries and retained source-integrity guards.

## Candidate checks and limits

The single pre-push rebase reported the branch up to date with `origin/main`.
The implementation and tests remained unchanged through the candidate checks;
only this evidence record was completed afterward.

| Check | Observed result |
|---|---|
| `cargo test --locked -p agentq-runtime --test history` | PASS, 18/18; includes the failure-before / pass-after regressions above. |
| `cargo test --locked` | sandbox-blocked, not PASS: 117 passed; the unchanged legacy fixture at `agentq/legacy.rs:473` failed writing a temporary file with `Operation not permitted`; 33.46 s including build. |
| `cargo test --locked -p agentq-runtime` | sandbox-blocked, not PASS: 15 unit checks and 16 process scenarios passed; the unchanged external-consumer check could not write a dependency `.d` file under `target/runtime-consumer`; 8.65 s. This remainder was selected because the root failure stopped the canonical suite before runtime tests. |
| `cargo run --locked -- selftest` | PASS, offline with isolated legacy/npm homes; `selftest: ok 0.0.0-dev macos-aarch64`; 0.71 s. |
| Workflow-pinned `contract_lint.py agentq.md` | PASS, 0 errors and two existing headroom warnings; 0.04 s. Script read from SpecQ `7e9adf57287553176d8b3be61c2b22565543dc51`, as pinned in the workflow. |
| Changed-document local links and `git diff --check` | PASS. |

No permissions, sandbox policy, fixture assertions or consumer dependency paths
were changed to bypass the host write failures. The two host gaps remain open.

Native version/OS qualification remains separate from adapter tests (§12/P2).
TaskQ Conversations adoption and rendering are outside this repository change.
Merge-result CI is enforced by TaskQ review/publication (TaskQ §7/R12), not established
by local tests. No deployment or stable-platform claim follows from this work.
