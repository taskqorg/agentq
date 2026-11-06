# Typed runtime fidelity for PilotQ

Task: https://github.com/taskqorg/agentq/issues/255.
Owner request and acceptance: that task; source review: [pilotq#383 comment 6675596656](https://github.com/taskqorg/pilotq/issues/383#issuecomment-6675596656).

## Checkpoint before implementation

A1 was recorded on the task in [comment 6675746959](https://github.com/taskqorg/agentq/issues/255#issuecomment-6675746959).
Compared runtime-v0.7.0, immutable source `b5ef97c257a97664bcd087bfd5d3ec0797c931c7`, with the task baseline `d3179ecefa2ff4799882c232ef6c38caebf6babd`.
Both have these boundaries:

- `Event::ToolStart` carries ID, kind and name but no command/file target. PilotQ's typed mapping therefore stores `shell` instead of `touch x`, failing its retained #190 A6 audit guard.
- `Cursor::codex` maps all `thread/tokenUsage/updated` messages to context. `AppSession` checks ownership for its report, but `App.pending` also feeds the unfiltered typed cursor. A foreign notification can replace context or emit nulls that invalidate a good measurement.
- `Runtime::catalogue(home)` reaches `limits::codex_read`, which calls `executable(None)`. A configured executable has no typed input at this boundary.

Applied rules: R13 and agentq § 12 M12.9 keep native parsing inside adapters; P2 keeps missing measurements unknown; P3 requires the linked runtime detail before code. The existing audit assertions, grants, original-permit ownership and no-replay guards are unchanged.

## Failure and pass evidence

Linux development checks use `CARGO_HOME=/tmp/agentq-255-cargo` because the host Cargo cache is read-only in this worker sandbox. This changes build storage, not runtime policy.

| Acceptance | Check and failure before | Pass after |
|---|---|---|
| A2 | `cargo test --locked -p agentq-runtime --lib agent::events::tests`: `audit_detail_preserves_target_and_withholds_payloads` failed with null versus `touch x`. | Same filtered command: five tests pass. Codex exec/app-server and Claude retain the target. File changes retain paths without patches. Known Antigravity command/path fields are mapped; arbitrary MCP/dynamic payloads, output, content and credential-like inputs are excluded. |
| A3 | Same command: `context_requires_owned_thread` failed because unowned notifications emitted context. | Same command passes: owned values survive foreign values and foreign nulls, unknown owner emits no context, and owned missing numbers remain null. Cursor serialization is retained. `automatic_codex_turns_select_transport` also proves AppSession-to-progress ownership wiring. |
| A4 | `cargo test --locked -p agentq-runtime --test agent catalogue_and_account_reads_need_no_turn`: catalogue without a configured argument failed `app-server spawn: No such file or directory (os error 2)` when no Codex was available. | The same scenario calls `catalogue_with_executable` with a renamed native binary outside PATH and passes. Two catalogue pages use one session, account read uses another; no thread/turn starts. |

The final focused adapter command is `cargo test --locked -p agentq-runtime --test agent -- catalogue_and_account_reads_need_no_turn automatic_codex_turns_select_transport`: two tests pass.
The original Codex tool-kind and neutral-event regressions remain unchanged. PilotQ's audit guard is in its repository and is neither edited nor claimed executed here.

## Canonical checks and sandbox limitation

The implementation candidate `d65aa56b3c6793e07697cebb19185293c39f4244` was rebased once on origin/main before checks. The first evidence-only amendment changed no executable code or assertions. A later PM coverage note added NotebookEdit to the existing file-tool mapping and expanded targeted regressions, as recorded below.

- `cargo test --locked`: sandbox-blocked, not PASS. The binary tests (122), earlier integration targets and runtime unit tests (46 passed, one ignored live check) passed. The runtime contract target stopped at `lingering_process_is_stopped_after_its_turn_ends`. Its child log shows a fake-client write to the inherited native home failed with `Read-only file system` before the native terminal record. The single filtered rerun reproduced the same filesystem denial; its assertion was retained.
- `CARGO_HOME=/tmp/agentq-255-cargo CODEX_HOME=/tmp/agentq-255-test-codex cargo test --locked -p agentq-runtime --test agent`: all 35 runtime contract cases passed with an isolated fake home, including the unchanged lingering guard, protected retained recovery/no-replay, all adapters and external consumer compatibility. This does not relabel the original full-project run as passing.
- `CARGO_HOME=/tmp/agentq-255-cargo AGENTQ_SELFTEST_OFFLINE=1 cargo run --locked -- selftest`: `selftest: ok 0.0.0-dev linux-x86_64`.
- `python3 /tmp/agentq-255-specq/engineering/scripts/contract_lint.py agentq.md`, checked out at the workflow pin `7e9adf57287553176d8b3be61c2b22565543dc51`: PASS, zero errors.

The remaining canonical test targets and native Windows/macOS execution require CI evidence. No sandbox-blocked observation is reported as healthy (P2; R12).

## Consolidated PM coverage

Fresh inbox note `6675881959` consolidated held duplicate #256 into this outcome after the first push. Its explicit cases are included: Codex command and every changed path on both transports; missing detail null; Claude Write/Edit/NotebookEdit file paths; foreign context following owned context; and catalogue None preserving PATH fallback.

The final `cargo test --locked -p agentq-runtime --lib agent::events::tests` run passes six tests. The two focused adapter cases pass again, including the added legacy catalogue/PATH assertion. Offline selftest passes again after the NotebookEdit mapping change. No pre-existing guarding assertion was changed. The full-project sandbox limitation above remains open; the 35-case runtime run predates only the one-arm NotebookEdit addition and these targeted assertions.

## Consumer compatibility and publication

Runtime crate 0.8.0 adds `ToolStart.detail`; exhaustive field patterns must add the field or `..`. The old `catalogue(home)` call remains available, and the new `catalogue_with_executable(home, Some(path))` selects the caller's configured native executable without fallback. Checkpoint data is unchanged. No native transport, approval, sandbox or replay behavior changes.

PilotQ must carry detail through its existing secret scrubber into the audit target and retained findings, pass its configured executable, and pin the accepted immutable source. Unsupported or unsafe audit details stay absent; the field is local audit data and must not enter metadata-only telemetry.

The candidate commit and canonical check outcomes are supplied in the PR and TaskQ result. The existing main Test workflow publishes `runtime-v0.8.0` only after exact-source Linux, Windows, macOS and contract checks pass; the tag must be read back to its full source SHA before PilotQ pins it. A proposed version is not an observed published tag. Required merge-result CI remains the publication gate (R12; TaskQ § 7).

No working profiles were installed or modified. No native paid probes ran. Local fixtures do not qualify live Telegram; PilotQ integration, its audit guard execution and live qualification remain in #383.
