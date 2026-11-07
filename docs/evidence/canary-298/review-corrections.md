# AgentQ 298 static review corrections

This source-only descendant corrects P1 and P2 from `/tmp/agentq-canary-review-410c510/report.md`. Parent `410c510d4d7ed7eb8704ec700dcec7bbcc4c3208`, blocked ancestor `ae78c84af03069b55a766446dc3633ee0d6ffa15`, their reports and their attributed evidence remain unchanged. No historical result is relabeled as a pass of this correction.

## Audit completed before correction

The separate read-only audit was completed and reported before this source task. Its frozen supplement is `/tmp/agentq298-provenance-audit-20261107T130359Z.md`, SHA-256 `85c29c4535adb1a7d67328e29e549de7ccb8152e16505899b9b120fbb3431c3f`. It records the precise early command window, temporary-install fix and approved nonsecret current grant projection. This code change neither changes host state nor clears the historical read/chmod/write/export unknowns. No host artifact is reused as a fixture.

## P1: version and native execution share one context

The bare `canary::version_at(Path)` command is deleted, including its bypassing unit-test executable path. The collector constructs the exact Launch before any version observation. Both explicit version observations call `bound_version` with that same Runtime and Launch; authority and executable identity are rechecked under the existing lifecycle lock before and after the observation.

`Runtime::version_for(&Launch)` is the minimal version-only seam. Adapter routes it to the version operation extracted from existing Runtime readiness. That operation uses `preflight::command`, which validates the requested command/policy, applies the same native environment preparation, removes registered inherited session/auth context for explicit homes, selects the explicit client home and sets Launch.root as cwd. The existing 5-second probe and result/minimum-version semantics are reused. It adds no login, retry, repair, cache, timeout or fallback. A runtime without this operation returns unknown without executing a command.

Existing readiness reuses that same version function before its existing separately scoped login probe. The canary still hashes the exact canonical executable and requires matching observed versions before submission; actual native spawn retains its authority/byte check. Version evidence remains separate from actual scenario and applied-condition evidence.

## P2: known metadata names stay known and content-free

The fixed diagnostic schema-name allowlist now includes `cwd` and the already-supported optional session-lineage field `forked_from_id`. Neither value is copied, hashed as content, interpreted as a filesystem path or opened by projection. Unknown keys and type discriminators remain hashed, and persisted evidence is still reprojected through fixed keys.

The existing source inventory found ordinary session headers with `id` and `cwd` in runtime/tests/agent.rs (including the review-cited 1124 case), id-only headers across history/media fixtures, optional `forked_from_id` in history_steps, and turn_context with `turn_id` and `model`. All those schema names are now covered; existing `cli_version`/version and model fields were already allowlisted. This is a finite baseline inventory, not a claim that every future native metadata extension is known. Arbitrary new names still drift.

## Regression source prepared, not executed

The permanent real-Runtime fake chain now sets conflicting owned synthetic HOME/CODEX_HOME/cwd, invented OpenAI/Codex/Claude authentication variables and unrelated session markers, with RAII restoration on failure. Every fake executable entry records an invocation before any assertion or mode-file read, so an invalid receipt cannot evade the no-exec assertion merely by crashing the probe early.

Before any synthetic native/config data is read, every fake entry asserts the selected account home, absent registered auth/session variables and isolated cwd. Context receipts then require each native exec to have a version observation in its exact cwd. These checks cover both explicit collector probes, Runtime readiness/preflight probes and the one native turn. The former bare version command would receive the conflicting ambient context and fail this chain before native execution.

The normal fake session header now includes ordinary cwd and optional null forked_from_id. The normal scenario must remain unknown solely for unavailable applied-condition evidence, with five true scenario dimensions and no false drift finding. Existing new-record and nested-new-field modes must remain drift; wrong answer/model mismatch and missing usage/hooks/history protections remain. Assertions exclude the synthetic cwd/path strings from returned/persisted diagnostic evidence. A pure projection case includes an invented cwd value and confirms neither that path nor the native ID survives.

The bare version-executable mini-test was removed because it exercised the unsafe bypass rather than the production Runtime boundary. Numeric-version rejection remains in the pure parser checks; the real-Runtime fixture already retains changed-version and executable-byte refusal coverage. No guarding assertion is weakened.

## One-pass helper/context inventory

| Entry or helper | Context and correction scope |
|---|---|
| main canary arm / request_with | Exact protected durable receipt gate precedes collector; no scheduler/default entry added |
| collect::parameters / executable | Full explicit Policy/home/session/executable binding, no home or executable fallback |
| collect::bound_version, both call sites | Same constructed Launch, explicit Runtime version_for, pre/post authority and byte checks |
| Runtime::version_for / preflight::version | New minimal public seam with unknown default; Adapter reuses existing command/environment boundary; history Reader retains the unknown default and never falls back to an executable |
| Runtime::ready_for / preflight::ready | Existing account-bound readiness; shares extracted version probe, retains separate login semantics |
| Runtime::ready convenience | Existing legacy current-directory convenience is unchanged and is not called by canary |
| preflight login, workspace/model RPC and native launch | Existing shared scoped command/Launch preparation; no new probe context or auth read is introduced |
| canary hook/history/record/finding helpers | Existing approved data/native-home or owned workspace paths; no new executable entry or host artifact fallback |
| canary selftest / shape / numeric_version | Pure functions; no executable or host lookup |
| host::version_in | Pure parser reused; unrelated host-tool command probes are not canary callers and are unchanged |
| Command::new empty program | Existing final authority-refusal branch, not an executable fallback |

## Exact remaining validation and acceptance gaps

**No build, Cargo check, test, selftest, lint, formatter or executable regression was run for this descendant.** Inspection was static source/diff reading only. The source is uncompiled and the prepared regression is unexecuted. Earlier 410 author execution and reviewer compile-only receipts remain limited to their original revisions.

Central should include these existing selections in the one coherent final batch, not start per-commit qualification: compile the joined candidate; select the canary fixture chain (`cargo test --locked -p agentq --bin agentq canary:: -- --nocapture`); retain relevant Runtime readiness/history checks and offline selftest; run the workflow-pinned contract lint once for the final contract set. The new public default method and extracted readiness operation require inclusion in that joined compile/readiness coverage. Cross-platform/native execution remains separately gated.

Same-reviewer static follow-up is pending on this exact corrective commit. Independent integration acceptance, native per-host upgraded-client/paid scenario proof, Windows host proof and separately approved deduplicated board delivery remain open. No helper, installed-client/auth/profile/service/grant action, export, board message, push, PR or CI was run. Source preparation does not authorize any of those operations.
