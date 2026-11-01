# Protected preparation and completion: task 199

[Owner task #199](https://github.com/taskqorg/agentq/issues/199) adopts all six criteria of the [immutable AssetQ draft](https://github.com/taskqorg/assetq/blob/241371d3c1015901907e547fe6994ad3cd3d7230/docs/blockers/4-protected-lifecycle.md). M12.8 and its runtime detail were updated before implementation (P3/R13). This is an optional AgentQ API, not another resolver, scheduler, policy store or TaskQ Rust dependency. AssetQ keeps orchestration, delegated effects, current-image-tool discovery and output acceptance.

## Source and authority boundary

The TaskQ prerequisite [#2151](https://github.com/taskqorg/taskq/issues/2151#issuecomment-6565625160) was merged as `2b8f14d8ae7c8986c8fb9021959ea0a37da3fbe1` with reported green merge-result CI. Its [published protocol](https://github.com/taskqorg/taskq/blob/2b8f14d8ae7c8986c8fb9021959ea0a37da3fbe1/docs/design/protected-app-server.md) adds independent preparation, one-shot scope ownership, sealed descendant completion and bounded original-operation cleanup. Its close explicitly retains PM native qualification as open. Neither that close nor portable AgentQ checks prove the new Mac paths.

`ProtectedHost::prepare` uses the original reserved slot and checked adapter to obtain current host-owned authority without native launch. `ProtectedPreparation::checkpoint` contains the private slot/key and independent preparation receipt. Persist it atomically in caller-protected storage before launch. `recover_preparation` restores only this checkpoint; no launch/observe/completion receipt supplies expected authority. The preparation digest remains the host's opaque equality requirement; AgentQ does not independently serialize a second resolver's authority or turn it into a grant.

`launch_prepared` retains that exact request, actual consumer incarnation, revision/account, adapter/native artifact and authority. The host independently revalidates under its lock. A separate trusted observe binds frontend, launchd supervisor, native incarnation and scope. Every input repeats the existing effective-authority checks and prepared equality checks. Unknown readback sends nothing; an observed binding change permanently fences input. The old `launch` API and ordinary runtime paths remain compatible with their prior adapters; they never synthesize required preparation or fall back from a prepared launch.

`reconcile` reads only the original operation through the checked host. `state == "completed"` requires the original prepared authority/host generation, a sealed native/supervisor/scope certificate, same-boot singleton counters, cleanup intent, bootout acknowledgment and verified cleanup. Otherwise the result is explicitly unknown or an error and the caller retains occupancy. Root/frontend/supervisor exit, EOF, cancellation and terminal model events are not completion. A recovered caller need not have the original process identity; recovery never launches or resends input. Native artifact availability is rechecked before launch/input, not demanded from an already ended native process during completion readback.

The runtime never releases a slot. After verified native/descendant completion, AssetQ must also finish its delegated/external effects, release only its original immutable slot with its original key and read back service acknowledgment. Lost or reordered observations do not create a replacement operation. Host cleanup acknowledgment loss remains unknown; the runtime never retries native dispatch, input or cleanup itself.

## Portable failure and regression evidence

The new published TaskQ supervisor is not the transport frontend. The original AgentQ validator equates the supervisor PID with `ProtectedAppServer::id()`, rejecting a correct split-process receipt. A process-double regression reproduces the defect before the change:

```text
cargo test --locked -p agentq-runtime --test agent -- protected_frontend
exit 101
exact frontend and separate live scope supervisor must verify:
protected native/supervisor/consumer incarnation changed; no native input
```

After binding the exact frontend separately and retaining live supervisor/native/consumer checks, the same regression passes. Its fixture cancels and waits for its owned fake processes even on the failing-before path. No real host app-server, model turn or paid effect is used.

The existing protected refusal/transport scenario is retained. The new preparation/completion scenario uses explicitly invented host state and covers:

- Caller PID reuse, expired readback, non-launching and duplicate preparation, invalid/foreign preparation checkpoints and recovery before launch intent.
- Profile drift between prepare and dispatch, host refusal before intent, exact saved slot/key/effect and independent prepared authority. Preparation is not replaced on drift.
- Separate frontend/supervisor incarnations, missing/reused frontend, changed host generation, input seal, dispatch acknowledgment, scope boot/identity and permanent fencing after a validated binding change.
- A non-generating fake `initialize`/`initialized` exchange, cancellation and root/frontend/supervisor exit with completion still unknown; lost readback after intent never replays even that handshake.
- Missing certificates, surviving-descendant counters, invalid/reordered counters, native/supervisor PID reuse, foreign operation/scope, cleanup before intent, missing bootout acknowledgment and missing cleanup verification. Every incomplete `completed` receipt refuses release evidence.
- Duplicate completion, private checkpoint reload and reordered unknown observations on the same operation, with identical original slot/key/effect and no replay.

Host crash/descendant/counter models and native output-pipe protection remain in TaskQ #2151; AgentQ tests its protocol boundary and does not duplicate kernel accounting or policy resolution. These are deterministic component/transport checks, not native containment, image capability, real slot release or PM qualification. Existing grant, artifact, unknown-readback recovery, changed-binding fencing and single-observation assertions are not weakened (specq Testing).

Development gate: `cargo test --locked -p agentq-runtime --test agent -- protected_ app_transport_ off_reaches_` passes the affected scenarios. Workflow-pinned contract lint uses specq `7e9adf57287553176d8b3be61c2b22565543dc51`, script blob `29609754df6067ed504621ed4fc11f5a4a6dd995`; existing contract headroom warnings remain. The candidate gate and immutable source readback are recorded below after freezing.

## Native gate and upstream finding

Reuse the accepted composed AssetQ `05476e88c53f5232cc17f37bd664aad0ba103a79` PASS. Do not repeat its handshake/boundary workload, a model/paid turn or the accepted crate. No new grant, Full fallback, ordinary installation, account/model/capacity change, `TASKQ_ATTEMPT` stripping or slot-CLI refusal bypass is performed or authorized here. Private host paths, PIDs, release keys and raw receipts stay off public boards.

Read-only source inspection found an additional prerequisite: TaskQ `protected_scope::launch` copies only `PATH`, `LANG`, `LC_ALL`, `TMPDIR`, `TERM` and `HOME` into the launchd environment. It omits `AGENTQ_OPTIMIZATION`, while `runtime::optimization` defaults a missing value to `inherit`. Thus the new supervisor boundary does not preserve a caller's explicit/inherited `off` as required by M12.6. This is a source finding; no native reproduction or fix is claimed. AgentQ still sends the intent to its checked frontend and does not work around the host whitelist with arbitrary environment or native arguments.

A fresh writer check found no open TaskQ protected-scope writer: #2157 owns runtime/app-control/accounts, #2159 owns Conversations, #2160 owns host/tray capacity; open PRs were screenshot refreshes. Cross-board `taskq finding` refused because its board must match configured state. Filing the prerequisite as a Held AgentQ coordination task then refused with `#0: configure create_claim_issue before idempotent create`. No task was created or started, no board/configuration guard was bypassed and no production configuration was changed. PM must repair supported intake and coordinate the upstream environment correction, then retain its reviewed immutable source and green merge-result CI.

After reviewed AgentQ source publication and the corrected checked TaskQ artifact, PM qualifies only the added caller-bound preparation/drift refusal, scope-control and stdio peer readback, inherited descendants through root/caller/frontend interruption, sealed singleton certification, acknowledged cleanup recovery and original-slot release/readback under the existing Mac Workspace authority. Any unsupported, missing or uncertain observation retains the original operation and occupancy. This worker cannot invoke the owner-only app-server/slot route from its retained `TASKQ_ATTEMPT` context and does not strip it. No TCC/native denial or native PASS is inferred without execution.

| Immutable draft criterion | Candidate evidence | Remaining acceptance |
|---|---|---|
| 1: non-launching caller-bound preparation | Checked optional `prepare`, private checkpoint, actual caller validation and process-double checks | PM exact-artifact native preparation |
| 2: launch/input revalidation | Independent prepared equality plus retained authority/artifact/incarnation checks | PM native drift refusal |
| 3: durable native/descendant completion | Host-owned scope/certificate protocol consumed fail-closed; unknown retains occupancy | PM native scope/certificate qualification |
| 4: original slot/recovery/no replay | Exact checkpoint/request, same-operation reconcile, no release/dispatch/replay in recovery | PM original-slot release and acknowledgment |
| 5: adversarial lifecycle models | Retained host models plus new runtime preparation/transport/completion regressions | Native failure/recovery receipts remain separate |
| 6: publication/CI and PM proof | Source candidate, gated branch and review handoff | Green AgentQ merge-result CI, upstream intent correction and PM native proof |

Task #199 and AssetQ #4 remain incomplete until criterion 6 and the native evidence above are satisfied. Portable success never closes them (P2, M12.8, R12, TaskQ §7).
