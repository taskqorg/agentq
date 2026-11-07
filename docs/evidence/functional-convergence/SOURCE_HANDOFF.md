# Initial functional source join

This is an implementation handoff, not acceptance or verification evidence. No build, Cargo, test, check, lint, selftest, formatter, probe, hash-verifier, independent review, native or Windows execution was run for this join. Earlier author reports remain historical evidence for their own revisions only.

## Exact provenance

The actual base is `d3b0513653c788ad2f9a06ce91b50c283080a3db`: retired runtime core `71a32aa`, Windows locator join `a96004c`, neutral AssetQ join `68adde6` and the native-scope P1 correction `d3b0513`.

The following complete cumulative source differences were applied from `f0d129fc223c3db42a7273a68385b22f7a0e9140`, without importing their parent histories wholesale:

- Owned background stop: `1ed1ca8699b781194a422d32d473e4d0cca527ba`, including `d786506` protected stop, `aad2f6c` previous-incarnation refusal and `1ed1ca8` flat ownership history. Author handoff: `/tmp/agentq-owned-background-control-flat-1ed1ca8/SOURCE_HANDOFF.md`.
- Durable hook spool: `51a1ae2a93a38d9c8c92a6c5c5d0df11cec4989a`. Author report: `/tmp/agentq297-frozen-d3d564411ea8/report.md`.
- Explicit version canary: `dece8c5a7d1212f5e66f803fc59cdf4fe3f1055f`, including `ae78c84` authority/evidence entry, `410c510` Runtime collection and `dece8c5` version-context/schema corrections. Frozen author reports were copied unchanged into `docs/evidence/canary-298/`.

No diagnostic `978`/`452` source, old raw fallback, compatibility shim, Intent API or consumer opt-out was imported. Original refs and frozen author artifacts were not rewritten.

## Manual functional joins

- `runtime/src/agent.rs` exports both the existing `InputStage`, `LaunchError`, `Binding`, `Image` and the added `BackgroundExecution`, frozen hook builders and public physical identity helper. The private diagnostic exclusion fixture and launcher attribution remain.
- `agentq/main.rs` retains both CLI entries, both modules, hook-spool doctor/service fields and canary offline selftest registration.
- Runtime trait/Adapter/history-only Reader carry the owned-stop operation alongside the base typed launch/resume errors. History-only execution still refuses. The canary's scoped `version_for` default remains nonexecuting; Adapter uses the shared preflight command boundary.
- Native schema decoding was moved from the canary consumer into `runtime/src/agent/schema.rs`, exposed through `Runtime::record_shape` as typed `RecordShape`. The original known-name list, recursive unknown-key/type hashes, numeric counts and drift handling remain; unsupported readers refuse. Canary receives opaque raw record strings only to return them to Runtime, never parses their native JSON, and persists only projected diagnostics. Existing schema/privacy fixtures now call this Runtime operation.
- `Report` and `Launch` use the actual base APIs without reverting typed start failures, images, no-Intent retirement, archive-install isolation, owner fences or scope-conflict refusal. Three-way source joins retain the base security/environment code and additive operations.
- The full owned source includes secure source traversal, source/prefix/alias proof, current receipt/controller/native identity fences and linear journal reconstruction. The spool includes private durable batches, protected registration, current revoke authority, checkpoint ordering, service drain and doctor projection. Source fixtures and callers accompany each delta; none was executed.

## Finite pending functional deltas

Central must provide frozen exact source for these existing producers; this integrator must not duplicate them:

1. Claude history facts: `ctx_4720af2a6ae3`.
2. Launcher: `ctx_42e409e867b3`.
3. Claude fork: `ctx_a9231c3bac43`.
4. Lineage: `ctx_a6ab5af09d22`.
5. After those functional joins, central supplies exactly one source to the TaskQ cold caller for adoption.

No unresolved source conflict is intentionally left in this initial join. This does not establish GLOBAL functional completion: the four frozen producer deltas and actual TaskQ caller adoption remain pending. Independent review and the single coordinated compile/test batch remain deferred until central GLOBAL functional completion. Native, Windows, service, deployment and external delivery acceptance remain separate and unknown.
