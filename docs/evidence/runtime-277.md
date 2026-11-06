# Typed history for TaskQ event migration

Historical evidence at the cited revisions; optimization behavior below is retired. Current decision: [optimization retirement](../spec/OPTIMIZATION-RETIRED.md).

[Task #277](https://github.com/taskqorg/agentq/issues/277) supplies the typed observations required by TaskQ #2448. Applied rules: R9/R12/R13, agentq §12 M12.3/M12.9/M12.10, P2/P3 and specq Testing's development/candidate policy. The linked runtime contract was updated before implementation. No existing guarding assertion or authority rule was weakened.

## Defect and regression protection

On the task baseline `4b16e1c39e2a23c05e3e2010dbd69815abfcd8b3`, the new native-free public-API checks ran with:

```text
cargo test --locked -p agentq-runtime --test history migration_
```

The command exited 101: `migration_assistant_failure_keeps_auth_and_detail` found identical successful and failed assistant LogRecords; `migration_lifecycle_and_saved_permission_are_projected` found null instead of `turn started`. `migration_exact_input_and_identity_survive_restart` already passed and remains a compatibility guard.

After the initial correction, all three passed. Extending the assistant regression then exposed a second loss: an assistant authentication error followed by a failed `result` classified as `(failed, client-error, None)`, instead of `(failed, auth-rejected, Some(detail))`. The focused `migration_assistant_failure` check exited 101 before its correction. The classifier now retains that result's most recent assistant cause within the current turn. It does not inherit a cause across another assistant record, result, system init or turn-duration boundary. A later successful result remains successful.

`cargo test --locked -p agentq-runtime --test history` then passed all 21 tests. The three migration regressions protect:

- **A1:** exact multiline user text and native item identity for Claude and Codex; stable source hashes/IDs on reread; distinct physical duplicates; partial writes; checkpoint recovery; no extra record on restart; consumed-prefix rewrite refusal without cursor movement. Existing replacement, bounds and backward-page guards remain.
- **A2:** successful versus failed assistant observations; `authentication_failed`, generic and absent error codes; exact detail; final failed-result cause; successful-result recovery; turn boundaries; error-shaped user input; and no synthetic live completion from an API failure.
- **A3:** lifecycle labels through both `History` and `LogRecord`, covering rollout/exec/app-server, compact boundaries and session start. Claude quota, Codex token counts and account-limit updates have no lifecycle label.
- **A4:** observed saved permission modes on Claude native and stream history and LogRecords, including changed, unsupported, empty, malformed and absent values. The caller checks its accepted baseline; this projection grants no access and supplies no inferred fallback.

## Consumer API and release

Runtime 0.13.0 adds `LogRecord.assistant_failure`, `.lifecycle` and `.permission_mode`, and the same top-level fields on `History` events. `AssistantFailure` carries the native code, neutral error classification and local error detail. A consumer uses these projections without reading `raw` or native blocks. Exhaustive LogRecord constructors/patterns need updating; live Event variants and checkpoint data stay unchanged.

The runtime version bump uses the existing exact-source `Test` → `Tag runtime` workflow, which creates and reads back `runtime-v0.13.0` only after the merged source passes Linux, macOS, Windows and contract checks (§12 runtime publication). A proposed version is not a published tag. PR publication remains gated by merge-result CI (`merge_group`), or exact-SHA CI on the private fallback (R12; TaskQ §7). The reviewer publishes the accepted result; this worker does not merge or replace a tag.

The affected runtime-unit check, `cargo test --locked -p agentq-runtime --lib agent::`, passed 57 tests, ignored one live check and stopped on `protected_existing_authority_snapshot_refuses_drift_before_launch`: its invented `.codex/config.toml` write at `runtime/src/agent/host/tests.rs:111` received OS error 1, `Operation not permitted`. This is sandbox-blocked, not PASS; the guarding assertion is unchanged. No owner permission ask or policy change was made.

Canonical checks ran after the one pre-push rebase, on implementation candidate `fd04a6959aef62503590e7626d66c7c42a871aeb`. The final amendment updates this evidence only; executable files and assertions are identical.

| Check | Observed outcome |
|---|---|
| `cargo test --locked` | Sandbox-blocked, exit 101 at the same host fixture write above; not PASS. AgentQ's 124 unit tests and compression/control/hook/release targets passed. Runtime unit tests: 62 passed, one live check ignored, one sandbox-blocked. Every quota-window and protected retained-operation unit check other than that fixture passed. |
| `cargo test --locked -p agentq-runtime --test agent` | PASS, all 42 fake-client scenarios, including exec launch, resume, child restart recovery, deferred persistence and protected app-server preparation/launch/recovery/no-replay. Run separately because the blocked unit target stopped Cargo before this target. |
| `cargo test --locked -p agentq-runtime --test history` | PASS, 21 tests. The subsequently expanded assistant-detail regression also passed on the final implementation. |
| `AGENTQ_SELFTEST_OFFLINE=1 cargo run --locked -- selftest` | PASS: `selftest: ok 0.0.0-dev macos-aarch64`. |
| `python3 <specq>/engineering/scripts/contract_lint.py agentq.md` | PASS, zero errors, using the workflow pin `7e9adf57287553176d8b3be61c2b22565543dc51`. |
| `git diff --check` | PASS. |

The final candidate SHA is recorded in the TaskQ result and PR. The sandbox-blocked protection still requires CI evidence. No full-suite retry or rebase follows a later main update (TaskQ §7).

## Limits

These are native-free adapter/process checks. They do not qualify native CLI versions, working profiles, TaskQ adoption or actual permission enforcement. No paid agent calls, service deployment, login/configuration edits or consumer adoption were performed. The published tag and cross-platform CI remain separate observations made by the delivery workflow and reviewer.
