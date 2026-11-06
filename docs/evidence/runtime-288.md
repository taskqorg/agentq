# History envelope validity

[Task #288](https://github.com/taskqorg/agentq/issues/288) requires: "Typed history distinguishes a supported Claude native record envelope with an unrecognized type from an object missing the required native record discriminator; consumers never inspect raw JSON to decide validity."

Applied rules: R9/R12/R13, agentq §12 M12.3/M12.9, P2/P3 and specq Testing's development/candidate policy. The history wire contract was extended before code. No existing rule or guarding assertion was removed or weakened.

## Failure and retained protection

On baseline `d60307ba08e38a6977cee15049ab07f1e5cc4ef5` (runtime 0.13.0), this native-free public history regression exited 101:

```text
cargo test --locked -p agentq-runtime --test history migration_history_validity_distinguishes_ignored_metadata_from_invalid_records
```

The assertion `A2: ignored metadata and invalid records need different consumer decisions` failed with `left: Null`, `right: Null` for `file-history-snapshot` and `{}`. The regression checks both Claude transports and both paging directions, missing/empty/non-string discriminators, recognized and future types, and explicit unknown validity on other transports. It reads the neutral envelope, never parses the raw record to decide validity.

Existing history regressions retain exact multiline input, physical duplicate IDs, source/session identity, partial and 16 MiB record bounds, checkpoint restart, rewrite/replacement refusal, lifecycle labels, failed assistant observations and saved permission observations. Existing fake-client runtime checks retain exec/resume/restart and protected app-server behavior. These checks prove adapter/process behavior, not native client or TaskQ qualification (R12).

After the fix, `cargo test --locked -p agentq-runtime --test history` passed all 22 tests, including the same reproduction. The full gate ran once after the pre-push rebase on implementation candidate `30d83b3a6d0d8b0fe57decaaf45683a9f269d10d`. The final amendment adds these observations only; executable source, contract, lockfile and tests are identical to that candidate.

| Check on Windows x86_64 | Observed outcome |
|---|---|
| `cargo test --locked` | PASS: 121 AgentQ unit tests; compression/control/hook/release integration targets; 62 runtime unit tests (one live check ignored); all 42 fake-client scenarios, including exec/resume/restart and protected app-server behavior; all 22 history tests; qualification-example check. |
| `AGENTQ_SELFTEST_OFFLINE=1 cargo run --locked -- selftest` | PASS: `selftest: ok 0.0.0-dev windows-x86_64`. |
| `python3 <specq>/engineering/scripts/contract_lint.py agentq.md` | PASS: zero errors, workflow pin `7e9adf57287553176d8b3be61c2b22565543dc51`; script SHA-256 `77badc19c6a67f550fab1bdd1dffe20aff6f996443362ddc4703aa70bdd8d2df` matched the existing provenance check. |
| `python tests/runtime_publication.py` | PASS: both immutable-tag and single-CoreQ-source checks. The example consumer also built in the full gate. |
| `git diff --check` | PASS. |

After the rebase on `bef051a1b07dd777f919d812f084d9d9432885d2`, with the changelog and version text added, the same gate passed on macOS ARM64: `cargo test --locked` (42 fake-client scenarios, 22 history tests), `cargo run --locked -- selftest`, the pinned contract lint and `tests/runtime_publication.py`.

Release run [42261730122](https://github.com/taskqorg/agentq/actions/runs/42261730122) failed on Intel macOS at `runtime/tests/support/protected.rs:347` (`Running`, expected `Unknown`). This change does not touch that code. The same assertion and cause are tracked in [#274](https://github.com/taskqorg/agentq/issues/274) and fixed by [PR #281](https://github.com/taskqorg/agentq/pull/281), so that failure stays possible here until #281 merges.

No paid native turns, working-profile configuration changes, consumer adoption or host deployment were performed. Cross-platform qualification and merge-result CI remain separate observations for review (R12; TaskQ §7).

## Consumer contract and publication

Runtime 0.13.1 adds `validity: {status, reason}` to every history event. Claude records with a non-empty string discriminator have status `valid`, even when their meaning is unsupported; missing or malformed discriminators have status `invalid`. Other transports report `unknown` with a reason. This field proves envelope validity only. Raw bytes, content projections, event IDs, cursor checkpoints and execution APIs retain their existing behavior.

A consumer can refuse `invalid` or `unknown` envelopes, skip a valid unsupported metadata record, and continue applying its exact-input and duplicate-input checks to valid input records using the existing blocks and IDs. No native discriminator inspection is needed.

The version bump uses the existing exact-source `Test` → `Tag runtime` workflow (§12 runtime publication). Only the merged source's successful Linux, macOS, Windows and contract checks permit creation and readback of `runtime-v0.13.1`. The worker opens the PR and hands it to review without waiting for CI (TaskQ §7); it does not merge or create an unchecked tag. A4 remains unproved until that tag is read back. Publication and consumer pinning are separate from TaskQ's real-worker qualification.
