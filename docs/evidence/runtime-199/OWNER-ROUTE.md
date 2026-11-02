# Protected owner-route handoff

This is the bounded source handoff requested by [PM note 6571952645](https://github.com/taskqorg/agentq/issues/199#issuecomment-6571952645). It advances independent review/publication of PR #200 while native A1/A3 remain unknown (M12.8, R12, TaskQ §7). Root performs native qualification after reviewed source publication. The worker does not invoke this route against a real host or remove `TASKQ_ATTEMPT`.

## Artifact and prerequisites

The artifact is [owner-route.rs](owner-route.rs), SHA-256 `16df6ecbd508faf3d18052f0a7dc4e46c0fa2e3679423392901a1be663899619`. It is a caller module, compiled and exercised by the existing runtime process-double test. It is not a second runner, resolver, admission loop or scheduler. The production runtime source remains `33cd5046c234dacf25c2ad47f350acf35b9eebdf`; consume the complete reviewed/published AgentQ revision containing this module, not that older revision alone.

Root must first retain the accepted AgentQ merge SHA and green `merge_group` receipt, then the independently reviewed TaskQ source/build/signature/artifact receipt. Supported TaskQ source is `5292e63ffaeda41ca90d1b2e21b93f78fe2d3e39` (#2151 plus #2161); its launchd job hash includes `AGENTQ_OPTIMIZATION=off`. A version or executable hash alone does not establish source provenance or native behavior. Do not use a guessed installed revision or change `current`, accounts, model, capacity, grants, authentication or service state to satisfy this recipe.

Use the actual long-lived consumer under the existing Mac AssetQ Workspace authority. Its already journaled `taskq.slots/1` request must bind its real PID/creation/boot, canonical registered project and original caller/operation/key. Use the supported slot identity/reserve/observe protocol and fresh `reserved` acknowledgment from the [pinned host contract](https://github.com/taskqorg/taskq/blob/5292e63ffaeda41ca90d1b2e21b93f78fe2d3e39/docs/design/external-slots.md). This module neither reserves nor releases capacity. Do not attach it to an accepted generation, replay a previous operation, replace an uncertain operation ID, or transfer a live attempt to another process. An existing operation with launch intent is recovery-only.

Choose an existing caller-owned private operation directory outside the resolved native write set, including its cache/temp exceptions. Root verifies that storage boundary; mode 0600 alone does not isolate same-user native code. Keep the original slot/key, host trust anchor and all receipts private. No live preparation or secret-filled checkpoint can be manufactured by this worker: only the trusted host's later caller-bound response supplies them.

## Exact request and one-shot checkpoint

Include the reviewed module in the actual consumer. In the following Rust expressions, `host` is `ProtectedHost::taskq(operator_selected_canonical_artifact, independently_verified_sha256)`, `slot` is the original reserved JSON request, and `directory`/`private_log` are trusted private paths. The consumer's existing runtime dependency supplies `Instant`/`Duration` from `std::time`. These are sequential calls in that same owning process, not separate shell invocations with different PIDs.

1. Prepare without native dispatch:

   ```rust
   let preparation = owner_route::prepare(&host, &slot, &directory)?;
   ```

   The exact adapter argv is `service app-server --action prepare --request <JSON>`, where `<JSON>` is `{"schema":"taskq.app-server/1","slot":slot}`. No other request fields, executable, grant, environment, model or native flags are supplied. The host obtains fresh authority independently; a launch/observe receipt is never its replacement.

   The module creates and syncs these immutable private files:

   | File | Exact content and meaning |
   |---|---|
   | `prepare-intent.json` | Original complete `slot` JSON, durable before host preparation |
   | `preparation.json` | Unmodified `ProtectedPreparation::checkpoint()`: `{"slot":slot,"receipt":fresh_prepare_response}` |
   | `launch-request.json` | Unmodified `preparation.request()`: schema, original slot, host revision/account and independent preparation digest |

   Repeated preparation, an existing file, partial write or failed sync refuses; nothing is replaced or deleted. A failure retains the original operation for private reconciliation. Missing preparation never permits launch.

2. After retaining the private prepared facts and confirming the reviewed artifacts/current authority, dispatch once:

   ```rust
   let mut server = owner_route::launch(&host, &directory, &private_log)?;
   let receipt = server.verify(Instant::now() + Duration::from_secs(5))?;
   ```

   Before calling the existing `launch_prepared`, the module rereads the checkpoint and exact request, rejects changed slot/key/request fields, and creates/syncs `launch-intent.json` containing that exact request. It passes only `Intent::Off`; it never forwards arbitrary environment. The exact adapter launch argv is `service app-server --action launch --request <contents of launch-request.json>`. Keep native stdin/stdout/stderr in the returned protected transport. Every send still requires fresh effective authority. The host independently enforces one dispatch per original slot.

   Any launch error is uncertain. The intent file remains, so another call cannot retry dispatch, even if no native identity was observed. Do not remove or replace any checkpoint/intent, alter PID/boot/key, resume by launching again, or derive expected authority from that launch's receipt.

3. Retain the protected handle in the original consumer. Root uses only the existing approved non-generating lifecycle fixture through that transport. No model turn, paid work, crate replay or speculative image-tool discovery is part of this handoff. Reuse AssetQ `05476e88c53f5232cc17f37bd664aad0ba103a79` composed PASS; do not repeat its accepted generation or unchanged boundary workload.

## Recovery, completion and release

Recovery never calls `prepare` or `launch` again. A later owner process can restore the exact private checkpoint with:

```rust
let preparation = owner_route::recover(&host, &directory)?;
let receipt = host.reconcile(&preparation, Instant::now() + Duration::from_secs(5))?;
```

For cancellation use `host.cancel(preparation.request(), deadline)` or the retained handle's `cancel`; this is exact-child stop authority, not completion. Root saves each raw readback in its existing private journal. `reconcile` observes/cleans up only the original operation and never relaunches or resends input. Unsupported/missing/reordered evidence, partial checkpoints, changed binding, absent certificate or uncertain cleanup means unknown and retains occupancy. Do not repair a failed readback by removing attempt/grant/slot guards.

Root's native acceptance retains caller-bound prepare and drift refusal, exact peers/scope-control, a surviving inherited descendant after root/caller/frontend interruption, sealed singleton certification, acknowledged cleanup recovery and no replay. A failed input or interrupted fixture never authorizes another dispatch under the original or a replacement identity. Root/EOF/supervisor absence and model terminal events alone do not prove native completion.

Only a validated `state == "completed"` host reconciliation proves native scope/descendant cleanup. AssetQ must additionally finish delegated/external tool effects. It then releases only the original whole-operation slot/key through the existing slot protocol and observes `released` acknowledgment; a lost acknowledgment is observed with the identical request, never a new operation. This module contains no slot release API. No unknown operation is released, and no live attempt is moved.

Public handback includes source/artifact digests, reviewed/merge-result CI identity, path-free command outcomes and unknowns. Exact host paths, native identities, keys and raw receipts stay private with Root/PM. The source handoff and portable checks do not establish A1/A3 or close AssetQ #4.
