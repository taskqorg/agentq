# Protected runtime consumer handoff

Task: [AgentQ #186](https://github.com/taskqorg/agentq/issues/186). Owner request and acceptance: “Let AssetQ and other consumers launch an app-server through existing protected host authority and receive evidence bound to the actual native operation.” Applied rules: agentq.md M12.2–M12.8, P1–P3; TaskQ R8/R9 and §3; specq Testing, development/merge-candidate run policy. Native host qualification remains separate from portable runtime checks.

## Immutable inputs

- AgentQ compatibility baseline: `513193c8a96f3de1c71a86291a679230817ce0b2`, merged as `b2887ec28f258ac31466024d1065dc41ad59c344`. Protected runtime source: `b16c9e95d7e2292d0980e5e8a7af42b80b2d22c1` (source tree `b61fcfade20f0a28c7a6074055fac220b0dd75fd`). Select that immutable `rev`, not a moving branch; publication and CI are separate receipts below.
- CoreQ primitive pin: `e84761cdab43723b871a42b3021e9c39afcd9484` (`v0.18.0`), unchanged in the runtime lockfile. No upward dependency, TaskQ crate or policy store was added.
- Optional TaskQ adapter: consume corrected reviewed source `56b1a1eba274bf88520e4d668461372fe3048b00`, published by [PR #2139](https://github.com/taskqorg/taskq/pull/2139) as merge `e6555153df9515974d7b5ec3c93af7cb5a9c481f`. [Corrected protocol and PM probe recipe](https://github.com/taskqorg/taskq/blob/56b1a1eba274bf88520e4d668461372fe3048b00/docs/design/protected-app-server.md). GitHub readback confirms green [exact-source CI 41322863513](https://github.com/taskqorg/taskq/actions/runs/41322863513) and [merge-result CI 41323567673](https://github.com/taskqorg/taskq/actions/runs/41323567673). PM's staged signed Mac artifact from the reviewed source is SHA-256 `de4c406b9a8378623ee113d634f9f6808a5cc2ba8ab5ae23db77c0b5bd25fa0a`; this is a separate artifact receipt, not a digest of a rebuilt merge binary. Direct Git blob comparison confirms identical adapter/protocol files between source and merge; whole trees differ in unrelated icon code, icon evidence and root contract, so no whole-artifact equivalence is claimed.
- Historical protocol baseline: source `deb1f66e2d97fad475623e03985c896a706d9b03`, [PR #2132](https://github.com/taskqorg/taskq/pull/2132) merge `8d75d224133cd18aba592a03031b0da597dcd28b`, with green merge-result CI. Its existing-host configuration failure and subsequent failed native receipts are retained below; do not consume this superseded adapter.
- That adapter's Windows debug artifact receipt is SHA-256 `18bc46c97cbbac6e653984cfe4f5debd536fbbc2e6c1a87b354624920950fd14`. It refuses real protected launches on Windows. This is neither a Mac artifact nor the binary to use for Mac qualification.
- Blocking AssetQ input: [protected-launch gap](https://github.com/taskqorg/assetq/blob/18a03fd52fdde584ea4043f2a43c66484558cd76/docs/blockers/4-protected-launch.md). AssetQ #4 stays incomplete until it adopts and qualifies this supported path. The retained crate is never regenerated or replayed.

## Consumer recipe

This recipe is compiled portable API evidence against the unchanged runtime pin. Use the corrected immutable TaskQ adapter above; PM has qualified that adapter on its Mac, as attributed below. The composed AgentQ/AssetQ native path still requires consumer qualification.

The operator supplies the reviewed, immutable TaskQ adapter path/digest and independently expected canonical native executable/digest, account home, grant approver and generated-profile digest. Never take that trust anchor from a task, environment override or operation receipt. A digest identifies reviewed code; it does not make arbitrary code an authority. Protect the binary and its directory using the existing host boundary.

The consumer manifest selects the additive runtime source explicitly:

```toml
[dependencies]
agentq-runtime = { git = "https://github.com/taskqorg/agentq", rev = "b16c9e95d7e2292d0980e5e8a7af42b80b2d22c1" }
serde_json = "1"
```

1. Through the existing PM/TaskQ channel, reserve and observe a `taskq.slots/1` slot for the canonical project, Codex runtime and real consumer PID/creation identity. Retain its caller/op-id, boot, admission expiry and private release key with the original AssetQ job. Do not create a development-board task for the asset operation.
2. Read the existing project Workspace grant and selected account through that same operator channel. Supply its exact current policy revision/account as match requirements; never reuse a historical receipt's revision. No new grant, Full fallback, capacity/account/model/configuration change or Windows enablement is authorized. The corrected TaskQ adapter accepts only the existing Mac Codex path, including the existing `-s workspace-write -c sandbox_workspace_write.network_access=true` host request under the required outer boundary.
3. Durably retain the exact `taskq.app-server/1` request before launch. Its only fields are `schema`, `slot` (the existing binding plus key), `policy_revision` and `account`. The runtime rejects extra environment/native argument/grant fields, and TaskQ independently authenticates the slot and resolves authority.
4. Construct `ProtectedHost::taskq(adapter_path, adapter_sha256)` and `ProtectedIdentity { profile_sha256, executable, executable_sha256, account_home, approver }`. Call `host.launch(request, expected, Intent::Off, private_log)`. An error is uncertain: recover the same operation with `host.observe`, never relaunch or substitute a new operation ID.
5. Call `server.verify(deadline)`. Save its private receipt with the job, keeping requested, saved and verified-effective facts separate. Require the expected canonical project/profile, Codex runtime/account/image/digests, grant revision/approver, caller/operation, host generation and actual live supervisor/native creation identities. No copied JSON can open the runtime's send gate. Host-side validation repeats at actual native delivery.
6. For qualification, use `AppSession::initialize`, `server.send`, `server.receive`, `AppSession::observe`, then `initialized`. This is a non-generating handshake. Never call `turn/start`, generate an image or replay the retained crate during qualification. Perform the bounded native filesystem/network/child probes in the pinned PM recipe only through the existing approved boundary.
7. During later authorized consumer work, retain the usual `AppSession` thread/turn checkpoint and pass native tool/approval requests to the consumer. `send` validates authority before every message; timed-out writes are uncertain and fenced. Record through `AppSession::record` using the original CoreQ store/context for one idempotent `agent.turn`; readiness emits none. The host's launcher marker suppresses a duplicate optimizer model span. `server.optimization()` is only configuration evidence.
8. Use exact native `AppSession::interrupt` for turn interruption and `server.cancel` (or `host.cancel` after recovery) for host process cancellation. Neither proves completion or releases capacity. Retain the existing slot until the entire turn and probe/tool children are reconciled, then use its original whole-turn release and observe acknowledgment. EOF, process exit and missing/unknown receipts never authorize release or replay.

The APIs use `serde_json::Value`, `std::path::Path` and bounded `Instant` deadlines, matching existing runtime consumers. No caller-built native `Command`, arbitrary environment or serialized authority enters this protected API. The old unprotected `AppServer::start` remains compatible for callers that already own their native process boundary.

The non-generating exchange uses the same `AppSession` API as an ordinary transport:

```rust
use agentq_runtime::agent::{AppSession, ProtectedAppServer, ProtectedHost, ProtectedIdentity, Spec};
use agentq_runtime::optimization::Intent;
use serde_json::{Value, json};
use std::{path::Path, time::{Duration, Instant}};

fn handshake(host: &ProtectedHost, request: Value, expected: ProtectedIdentity,
             spec: &Spec, private_log: &Path) -> Result<(ProtectedAppServer, AppSession, Value), String> {
    // The caller has already durably saved this job/request and its private slot key.
    let mut server = host.launch(request, expected, Intent::Off, private_log)?;
    let deadline = Instant::now() + Duration::from_secs(10);
    let receipt = server.verify(deadline)?;
    // Keep receipt with the private job, outside public telemetry; it grants no authority.
    let mut app = AppSession::new(spec)?;
    server.send(&app.initialize(json!({"clientInfo":{"name":"assetq","version":"qualification"}}))?, deadline)?;
    app.observe(&server.receive(deadline)?.ok_or("initialize reply unknown; reconcile same operation")?)?;
    server.send(&app.initialized()?, deadline)?;
    // Retain server, app and the original slot in the real job until whole-turn reconciliation.
    Ok((server, app, receipt))
}
```

Supply `Spec::root` and `Spec::home` matching the canonical request/project and expected account home. Native arguments and environment do not come from `Spec` on this protected path. Retain the returned transport/session/receipt with the existing job and perform exact cancellation/reconciliation; dropping them never releases the slot. An error retains the original operation/request for reconciliation.

## Portable evidence

Before: the pinned baseline `AppServer::start` directly calls `Command::spawn` after adding native arguments; there is no protected-host API. This is the missing feature, not a claim that an existing unprotected API enforced host authority. No real host launch or paid native turn was used as a reproduction.

Development: `cargo test --locked -p agentq-runtime --test agent -- protected_ app_transport_ app_server_ off_reaches_` passed four affected scenarios on Windows. The protected process-double scenario rejects unknown enforcement; missing/mismatched project, profile, account, image, adapter, approver/revision and caller/operation; stale consumer/native/supervisor identities; grant/env/receipt fields; changed host generation; copied release key/receipt and duplicate consumed launch. It tests interrupted readback without resubmission, a non-generating initialize exchange, fake native thread/turn/tools/drafts, exact cancellation and one observation across recovery. Assertions protect the trust/input boundary and existing operations; none was weakened. The fake protocol turn generates no paid request or asset.

Frozen source `b16c9e95d7e2292d0980e5e8a7af42b80b2d22c1` passed the Windows project gate after the required pre-first-push rebase: `cargo test --locked` (113 root unit, 4 root integration, 11 runtime unit and 13 runtime process scenarios), followed by offline `cargo run --locked -- selftest`, in 249.734 seconds. `AGENT_CONTEXT_HOME` and `npm_config_prefix` were isolated in OS temporary storage; no real legacy package/profile was removed. The four workflow Python suites passed 9/11/2/4 cases. Contract lint from workflow-pinned specq `7e9adf57287553176d8b3be61c2b22565543dc51` passed with zero errors; existing contract headroom warnings remain (226 lines, 43639 bytes).

Read-only Astra security review found the recovery defect below and confirmed its correction on the committed candidate, with no remaining blocking findings. It inspected source/protocol/tests and did not rerun the gate. Only this evidence/consumer recipe changes after the frozen-source gate; runtime code, tests and dependencies stay identical to that immutable pin. Runtime tests, source CI, staged host artifacts, native containment and ordinary deployment are distinct evidence. Portable checks do not qualify a Mac host.

Published-pin readback: the exact TOML/Rust blocks above were extracted into an isolated temporary external consumer. `cargo generate-lockfile` then `cargo check --locked --manifest-path Cargo.toml` passed in 34.55 seconds; its lockfile resolves remote `agentq-runtime` at `b16c9e95d7e2292d0980e5e8a7af42b80b2d22c1` without a local path patch. The handshake function was compiled, not executed; the only warning is unused example function. This proves the additive published API composes with the recipe, not native authority or Mac qualification. `git diff --exit-code b16c9e95d7e2292d0980e5e8a7af42b80b2d22c1 -- runtime/` verifies unchanged gated runtime source.

Review regression: `cargo test --locked -p agentq-runtime --test agent -- protected_` failed before the recovery correction: after initial verification, an `unknown` receipt permanently faulted the transport, so fresh same-binding `verify` failed with `protected authority missing, unknown or mismatched; no native input`. The same regression passes after preserving the original binding without permanently faulting on unavailable evidence. No input is sent during unknown readback; verified binding changes and uncertain writes remain permanently fenced. Retained risk protection: recover the original operation without replay or enlarged authority. The read-only security review requested this correction; no existing assertion was relaxed.

## Native qualification history and remaining gap

This Windows attempt has no Mac route. The following historical failures and PM's subsequent pass concern the TaskQ adapter. They do not establish that AgentQ's protected transport or AssetQ's consumer has executed on Mac. Keep exact source/build/artifact and authority receipts private with the job; publish only safe summaries/digests.

No deployment, ordinary installation, Full/new grant or Windows containment is claimed. AssetQ's retained crate and active consumer work are untouched. A5/A8 permit carrying remaining live Mac consumer probes explicitly in Open; AgentQ merge-result CI remains the publication gate under TaskQ §7/R12.

PM steer 6559108968 confirms PM owns the isolated exact-source Mac build and live existing-grant probes; this worker must not duplicate them. The note supplies TaskQ merge-result CI run `41313940774` and main artifact/check runs `41314626334`/`41314626375` as green. Those upstream receipts do not qualify this runtime or the actual Mac boundary.

Fresh PM steer 6559257028 supersedes any assumption that the published adapter accepts this host. PM built/signed the exact Mac adapter, admitted its slot, then observed `unsupported host app-server config` for the existing `-s workspace-write -c sandbox_workspace_write.network_access=true` request before native execution. The original slot release was acknowledged; no native/model turn or authority/settings change occurred. [Native failure receipt summary](https://github.com/taskqorg/taskq/issues/2131#issuecomment-6559239507); the private receipt remains with PM. Mac artifact SHA-256: `bff5934da3ba639f3e49381b690a38bd6ba8e1d6b132f6a4fe269a1f946b86d2`.

[TaskQ #2135](https://github.com/taskqorg/taskq/issues/2135) delivered the correction without a second authority parser, host setting change or Full/Windows fallback. Its intermediate Mac attempts also remain relevant: source `6774621e6db68005bbeaab90725121585e1ed502`, artifact `0560569c6549fd1ff3c8ab88482c5bfc4933903d2c57ce7b2a2588268bfaaa36`, created the real native process but refused unknown sandbox readback before initialize/input. Diagnostic source `66753e0126217110a6cd23cf13c2a3e292210bce`, artifact `adae97f481cf1845d26d8a970cd00c5d6f1a4a307e7f6601bd5536fa9b86ad12`, attributed the failure to an absent workspace marker. PM's paired OS fixture showed an absent-path kernel query returning denied despite successful creation under the same profile. The corrected adapter queries existing trusted objects with before/after identity checks; it retains all named fail-closed queries and path-free diagnostics. These are attributed upstream observations, not tests executed by this worker.

PM handoff in [AgentQ #186, note 6560280069](https://github.com/taskqorg/agentq/issues/186#issuecomment-6560280069) reports actual Mac PASS on source `56b1a1eba274bf88520e4d668461372fe3048b00` and staged signed artifact `de4c406b9a8378623ee113d634f9f6808a5cc2ba8ab5ae23db77c0b5bd25fa0a`. Native Codex app-server negotiated `0.159.3`; resolved native executable SHA-256 `4d210f7c5a18fd0386434df23b5bdbb8c0e7257d3e8a2b30b0769c8bbe99a878`, generated profile SHA-256 `e8b98ac093505e108a3decde3f879ee115304da0cce71be9a4951e66bf1791c4`, existing project Workspace readback revision `32`. PM retained private raw authority, identity, slot, probe program and artifact receipts. This note was read from the fresh TaskQ inbox; it is operator evidence, not independent local execution.

PM reports the original host request launched, separate trusted observe verified effective authority and matching native/supervisor incarnations, and native `initialize`/`initialized` completed. Non-generating standalone `command/exec` parent and child read, wrote, renamed and deleted inside an owned project fixture and reached loopback HTTP. Outside-set write/create/unlink/rename and symlink-alias writes were denied with unchanged disposable canaries. Named kernel policy/launcher/account/outside queries denied; production controls were not mutated. Mismatched policy refused; a second launch of the consumed operation refused without replay. The owned standalone command was interrupted with final exit `137`; exact cancellation/reconciliation found no surviving native incarnation or descendants; ended-authority readback stayed unknown. Original slot release was acknowledged and probe fixtures removed. PM reports zero model turns and paid effects, no grant/settings/account/model/effort/capacity/ordinary-installation change, and no retained-crate replay.

Open: live Mac qualification of the composed AgentQ protected API and AssetQ consumer against these pinned artifacts remains unexecuted by this Windows worker. PM/consumer must bind fresh existing authority to the real job, repeat the non-generating handshake and bounded read/write/network/child, outside write/delete/rename/symlink and control-store checks through that composed path, retaining interruption, no-replay and unknown-liveness reconciliation evidence. The TaskQ adapter pass does not close AssetQ #4 or qualify image/3D generation. Do not duplicate PM's completed adapter probes or replay the retained crate. AgentQ PR #188 merge-result CI remains pending at this handoff; prior branch/pull-request CI is green, not publication proof.

Resumption checks: `git diff --exit-code b16c9e95d7e2292d0980e5e8a7af42b80b2d22c1 -- runtime/` confirms that runtime code, tests and dependencies still match the gated pin. `git diff --check` and workflow-pinned contract lint pass. The corrected upstream resolver/readback/input-gate source preserves the protocol fields this API validates. Read-only Astra review found no blocking code or contract mismatch. Only this evidence file changes on resumption; unchanged full portable gates are reused under specq Testing, rather than rerun after documentation changes. No native launch was performed by this worker.
