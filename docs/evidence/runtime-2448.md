# TaskQ runtime observations

Contract: [runtime](../spec/RUNTIME.md), supporting [TaskQ #2448](https://github.com/taskqorg/taskq/issues/2448) and the parser follow-up to [AgentQ #312](https://github.com/taskqorg/agentq/pull/312). Runtime version: 0.14.0.

## API and compatibility

- `Window.rejected` records native rejection independently of `exhausted`. A Claude named top-level rejection marks its unified binding window; siblings without their own explicit status read false. Missing status without binding evidence, Codex utilization and old serialized windows read unknown. `Report.reached` retains pre-work refusal/reset behavior.
- `Report.detail` retains Claude terminal result subtypes and Codex failed-turn messages, including app-server failures. Typed errors, admission and terminal outcomes are unchanged. A terminal detail supersedes an earlier assistant failure detail without changing its typed error.
- `account_limits_with_executable(home, executable, now)` uses the configured Codex executable with the existing credential-before/after guard. The old API keeps native lookup. Other runtime adapters ignore this override; third-party trait implementations retain a default delegating to their old read.
- `History::with_native_session_alias(format, source, session, native_session)` explicitly binds one caller-validated Claude copy relationship before observation. Checkpoints retain that alias. Records naming the bound session or its alias are accepted; any third identity fails without advancing. Aliased records retain the caller session, expose the original `native_session`, and report `session_binding: "alias"`. Raw records and event IDs retain their source binding. Without the explicit alias, the same mismatch still fails. File replacement, truncation, prefix validation and forward/backward bounds are unchanged.

The caller must validate the copy relationship, such as TaskQ's hand-in/job binding; neither filenames nor native history alone authorize job adoption, input, cancellation or completion. Exhaustive Rust `Window` constructors must add `rejected`; old serialized windows remain readable. History consumers must handle the new explicit `alias` binding value.

## Regression evidence

All four additions were checked before implementation:

| Regression | Before | After |
|---|---|---|
| `window_rejection_is_independent_of_exhaustion` | assertion failed: missing rejection was null instead of false for the non-binding sibling | pass |
| `terminal_native_detail_survives_classification` | assertion failed: `error_max_turns` detail was absent | pass |
| `catalogue_and_account_reads_need_no_turn` with configured account executable | compilation failed: new method absent | pass; configured executable is absent from PATH and named differently from Codex; no thread or turn starts |
| `copied_claude_job_requires_explicit_bound_alias` | compilation failed: alias API absent | pass; strict mismatch, forward/backward equality, checkpoint recovery, third-ID refusal, malformed aliases and prefix rewriting covered |
| App-server native failure detail | assertion failed: detail was absent | pass; pre-work refusal and post-work failure retain their original outcomes |
| Fake protocol through public history | assertion failed: its registered format was refused | pass; colliding `assistant`/`result` records remain unknown under that protocol |

The fake runtime now uses an isolated launch root and records the call whose usage it reports. Setup registration again asserts that describing edits creates no files. Native start-limit, terminal, call usage, final text, report classification, history identity/normalization and Codex rollout parsing are selected by protocol callbacks. Common code only assembles runtime-neutral reports and validates observation boundaries.

## Parser comparison

The #312 review harness was reused for a refactor-only build before the additive observation changes, against the #312 source. It compared typed events, log records, report, quota windows, session records and history on 300 real Codex, 400 real Claude and 74 real Antigravity files. 771 live reads were byte-identical. The remaining 2 Codex files and 1 Claude file were changing during the reads; both binaries were then run against fixed copies of each, and all 3 comparisons were byte-identical. This isolates the parsing move from the deliberate new rejection/detail/alias fields.

## Real local transcript shape

Read-only inspection on macOS found 5,177 Claude project transcripts and 4 files containing native `sessionId` values different from their filenames. Every mismatched file contained exactly two IDs: the filename session and one original session. Counts of original-session / filename-session records were 1,801 / 284; 115,472 / 589; 87,629 / 1,470; and 41,283 / 1,275. These are observed transcript shapes, not proof that a particular TaskQ copy job created them: no matching saved job state was present.

The first real mismatched file was read through the public API without changing its bytes: strict observation refused `history native session differs`; the explicit alias returned the first 128-record page, including 126 original-session records, and its checkpoint recovered. No message text, credentials, session IDs or local paths are included here.

## Local checks

On macOS arm64, after rebasing onto main with CoreQ v0.23.4:

- `cargo test --locked`: pass, including 96 CLI unit tests, 68 runtime unit tests (one existing live test ignored), 42 portable native-client integration tests, 23 history tests and 2 new observation tests; supporting tests and doc test pass.
- `cargo run --locked -- selftest` with `AGENTQ_SELFTEST_OFFLINE=1`: pass.
- `python3 <pinned-specq>/engineering/scripts/contract_lint.py agentq.md`: pass at workflow pin `7e9adf57287553176d8b3be61c2b22565543dc51`.
- `python3 tests/hermes_adapter.py`: 2 pass.

No native paid turns, deployment, installed AgentQ changes or working-profile edits were performed. These checks qualify the API and read-only parser behavior; TaskQ adoption and end-to-end copied-job execution remain consumer work.
