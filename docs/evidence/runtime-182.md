# Shared runtime extraction: task 182

Historical evidence at the cited revisions; optimization behavior below is retired. Current decision: [optimization retirement](../spec/OPTIMIZATION-RETIRED.md).

Scope: [task 182](https://github.com/taskqorg/agentq/issues/182), [owner request](https://github.com/taskqorg/assetq/blob/main/docs/intake/2026-11-01-owner-request.md) and [architecture](https://github.com/taskqorg/assetq/blob/main/docs/architecture.md). The implementation continues the existing task branch and PR 183; CoreQ and consumer installations are unchanged.

## Applied contract

Amended by this task: purpose, P4, § 1 (`agentq agent turn`), M6.9, § 11, new M12.1–M12.7 and PRODUCT revision 3/R13. The contract and PRODUCT change preceded implementation. The rework clarifies the read-only effective-state projection and publishes a full Git revision rather than depending on a future version tag.

M4.8 and M12.5 require corrupt state to remain unknown. Runtime observation and optimizer control now share the existing config/lifecycle validators with unchanged validation and defaults. M12.6/M12.7 require session off to survive each adapter while preserving already-transmitted replay. TaskQ § 7.4 requires a failing reproduction and the same passing regression; existing recovery, native-mode, grant and exactly-once protections remain.

## Failure before, pass after

The regression checks failed against `d7c7fd8ddeabb72ff989cfbcd6aa4a41444306ad` with only the new checks added. They pass after the fixes.

| Command / retained regression | Failure before | Pass after |
|---|---|---|
| `cargo test --locked -p agentq-runtime --lib child_home_and_inherited_off_are_observed` | Requested on with child off returned reason `not_attached`; child home was ignored | Explicit account home, child env home and inherited off agree with the launched child; requested intent remains visible |
| `cargo test --locked -p agentq-runtime --lib invalid_lifecycle_and_preferences_stay_unknown` | Lifecycle schema 999 reported `on/configured` | Invalid schema, lifecycle fields and compression preferences return `unknown/unreadable_state` |
| `cargo test --locked -p agentq-runtime --lib commented_provider_gets_off_header` | A valid TOML inline comment prevented the off header | Single/double quoted provider values with comments get the launch override without file edits |
| `cargo test --locked -p agentq-runtime --test agent` (`off_reaches_codex_queue_and_app_server`) | Queue argv omitted the off header | Queue and app-server retain the per-launch override; queue keeps its saved native policy; CLI/background/recovery/tool/draft/catalogue/accounting traces remain green |
| `python3 tests/provider_adapter.py ProviderChain.test_nested_launch_off_preserves_native_output_and_replay` | Nested response recursion passed no off environment to the compressor | Off reaches the compressor; native output is retained and later replay stays byte-identical |
| `python3 tests/runtime_publication.py` | An existing runtime tag at another SHA was silently accepted | Matching SHA is idempotent; a different SHA fails before mutation |

The recovery checkpoint format and original scope hash remain unchanged. The external consumer uses deterministic child clients, never paid calls or installed client credentials. It runs a Claude and Codex turn and checks final output, attributable token counts, launcher ownership and absence of optimizer installation.

## Publication and qualification boundary

The example manifest and lockfile identify the published immutable runtime source revision [`513193c8a96f3de1c71a86291a679230817ce0b2`](https://github.com/taskqorg/agentq/commit/513193c8a96f3de1c71a86291a679230817ce0b2). A local path dependency alone is not publication proof. The example is outside the workspace and builds without a path patch. The runtime gate also runs publication regression checks on each CI platform.

The project merge gate uses `cargo test --locked`, offline `cargo run --locked -- selftest`, the four existing Python adapter/checker suites and contract lint at the workflow-pinned specq `7e9adf57287553176d8b3be61c2b22565543dc51`. Legacy removal is isolated with `AGENT_CONTEXT_HOME` and `npm_config_prefix` in OS temporary storage. Actual gate results and the exact candidate SHA are attached to the TaskQ result and PR.

GitHub merge-result `merge_group` CI remains the publication gate under TaskQ R12/§ 7. The worker does not wait for or substitute a branch-only run. Linux/macOS/Windows fake-client CI does not qualify native consumer paths. Native CLI/background/app/app-server, Remote Control and Desktop UI qualification remain with TaskQ #2094 and PilotQ #265. No consumer adoption, model/effort/grant/capacity change, service restart, optimizer installation or namespace migration occurred.

## Local candidate checks

- Full `cargo test --locked`: 116 root unit tests passed; `legacy::tests::edited_after_journal_is_stripped_not_refused` is sandbox-blocked with `Operation not permitted` at fixture file creation. Cargo stops the remaining targets after that failure; it is not a full-suite PASS.
- The remaining targets were run separately: `cargo test --locked -p agentq-runtime` passed 11 unit checks and 12 deterministic runtime traces; `cargo test --locked --test compression` passed 2 checks; `cargo test --locked --test control --test hook` passed 3 control and 1 hook check.
- Offline `cargo run --locked -- selftest` passed. Provider/client/Hermes/telemetry Python suites passed 11/9/2/4 checks respectively; the client suite retained its one existing platform skip.
- Workflow-pinned contract lint passed with 0 errors; its contract headroom warnings remain.
- macOS denied process inventory (`ps`: `Operation not permitted`); no process-inventory or native installed-client qualification is claimed. This host gap needs no owner ask for this task.
- Published-pin readback: `cargo run --locked --manifest-path examples/runtime-consumer/Cargo.toml --target-dir target/runtime-consumer` passed both deterministic client turns from remote revision `513193c8a96f3de1c71a86291a679230817ce0b2`. The example manifest and lock resolve that exact Git source, with no path patch. `git diff --exit-code 513193c8a96f3de1c71a86291a679230817ce0b2 -- runtime/` passed; the final runtime matches the published pin. The 12 runtime traces passed again after changing the consumer dependency.
