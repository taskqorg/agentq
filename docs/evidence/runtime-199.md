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

### Frozen candidate checks

Immutable runtime source is `33cd5046c234dacf25c2ad47f350acf35b9eebdf`. The required single pre-first-push rebase on `origin/main` completed without changes. The full candidate gate ran once with `AGENT_CONTEXT_HOME` and `npm_config_prefix` isolated in OS temporary storage and `AGENTQ_SELFTEST_OFFLINE=1`; `TASKQ_ATTEMPT` was retained.

`cargo test --locked` is **sandbox-blocked**, not PASS: 117 root tests passed and the unchanged `legacy::tests::edited_after_journal_is_stripped_not_refused` fixture failed at its temporary-file write (`agentq/legacy.rs:473`, `PermissionDenied`, `Operation not permitted`). Build and root execution took 90.951 seconds. No legacy fixture, guard, path or configuration was weakened; the full gate was not retried.

Because that root failure prevented the runtime suite from executing, `cargo test --locked -p agentq-runtime` selected the affected crate. Its 15 unit checks and 16 process scenarios passed, including all three protected cases, both Antigravity cases, ordinary transport, optimization and JSON turns. Its unchanged final `external_consumer_builds_and_plans_turns` scenario is **sandbox-blocked** while Cargo writes `find-msvc-tools`/`shlex` dependency files under `target/runtime-consumer`; it is not a successful external-consumer build. These permission failures do not establish their TCC cause and do not qualify another platform.

Offline `cargo run --locked -- selftest` passed on macOS arm64 (0.872 seconds). The four workflow Python suites passed: client compression 7 passed/1 skipped, provider 11, Hermes 2, telemetry qualification 4. Workflow-pinned contract lint passed with 0 errors and existing headroom warnings (233 lines, longest 396 characters). Changed document file links and `git diff --check` passed. `git diff --exit-code 33cd5046c234dacf25c2ad47f350acf35b9eebdf -- runtime/ Cargo.toml Cargo.lock` confirms later evidence edits retain identical runtime source, tests and dependencies. Green merge-result CI remains required; local permission failures are not waived or relabeled PASS (R12/TaskQ §7).

## Native gate and upstream finding

Reuse the accepted composed AssetQ `05476e88c53f5232cc17f37bd664aad0ba103a79` PASS. Do not repeat its handshake/boundary workload, a model/paid turn or the accepted crate. No new grant, Full fallback, ordinary installation, account/model/capacity change, `TASKQ_ATTEMPT` stripping or slot-CLI refusal bypass is performed or authorized here. Private host paths, PIDs, release keys and raw receipts stay off public boards.

Initial read-only source inspection found an additional prerequisite at TaskQ `2b8f14d8`: `protected_scope::launch` omitted `AGENTQ_OPTIMIZATION` from the launchd environment, while `runtime::optimization` defaults a missing value to `inherit`. That supervisor boundary did not preserve a caller's explicit/inherited `off` as required by M12.6. The published correction and remaining native boundary are recorded below. AgentQ sends the intent to its checked frontend and does not work around the host whitelist with arbitrary environment or native arguments.

The initial writer check found no open TaskQ protected-scope writer: #2157 owned runtime/app-control/accounts, #2159 Conversations, #2160 host/tray capacity; open PRs were screenshot refreshes. Cross-board `taskq finding` refused because its board must match configured state. Filing a Held AgentQ coordination task then refused with `#0: configure create_claim_issue before idempotent create`. No task was created or started and no guard or production configuration was changed. PM subsequently coordinated #2161; the historical intake error is no longer this task's blocker. Do not create another prerequisite.

### Published correction and installed-host readback

[PM note 6566506999](https://github.com/taskqorg/agentq/issues/199#issuecomment-6566506999) supersedes the intake ask: TaskQ #2161 result `53b9b7ed429d44b4cd3b06324db82a38c8e88f06` was reviewed and PR #2162 merged as `5292e63ffaeda41ca90d1b2e21b93f78fe2d3e39`, with green merge-result CI attributed to close receipt `6566262806`. Fresh issue readback confirms #2161 closed as completed. No new prerequisite, grant, auth copy, model activation or restart was performed.

The exact published `src/runtime/protected_scope.rs` blob `5655c6f8ed4995a6ad4a9ab308066f09d9a25819` now carries `AGENTQ_OPTIMIZATION` through `environment(home)` into the launchd plist and `job_sha256`. Its retained regression starts an isolated test child with `off`, asserts that the actual environment builder and plist preserve it, and asserts different job hashes for `on` and absent intent. The existing single-dispatch and transport guards remain. The producer protocol/authority implementation `src/runtime/protected_app_server.rs` is byte-identical at the old and corrected host pins (blob `775049413d2cf0431778d6b18d2b585e75c4a049`); AgentQ needs no wire, resolver or runtime-code change.

Fresh supported CLI readback reports installed version `4.6.59-dev.1793580776`. The installed command artifact has SHA-256 `4b6a54da85042483d3312d1b6ccfde7aa99dff3a9272abeae4b98e9abfefba7c` and 21,778,512 bytes; file identity/size/mtime matched before and after hashing. The exact published merge/result revision markers were not present in that artifact. This does **not** establish that the fix is absent: exact installed source remains unknown, and a dev version or running service is not an exact-source/native qualification receipt. TaskQ's published `build.rs` also distinguishes dev/test revision embedding from release provenance; no guessed source revision is substituted.

One non-launching invocation of the installed `service app-server --action prepare` with an empty slot was refused before request parsing with `Error: a worker cannot launch or control external app servers` (exit 1). The retained `TASKQ_ATTEMPT` boundary is the real remaining host blocker. No slot was reserved, native process or job launched, input submitted, cleanup attempted or capacity released. The worker did not retry or strip its attempt. PM must use the existing authorized owner route and retain private exact-source/artifact and native preparation/completion/recovery receipts after reviewed AgentQ publication; a new grant, forced restart or portable PASS cannot replace them.

Existing PR #200 head `42cc88268df4138e125abcff35a59090b80f18f7` has successful Test push run `41364562299` and PR run `41364627966` on Linux, macOS and Windows, including contract lint. Those are source checks, not `merge_group` or native proof. This continuation updates only the supported host pin and evidence; unchanged runtime/dependency readback against `33cd5046c234dacf25c2ad47f350acf35b9eebdf` passes. The previously run full gates and their sandbox-blocked outcomes remain; no rebase or full-suite rerun is warranted for these documentation changes (specq Testing, TaskQ §7).

After reviewed AgentQ source publication and the corrected checked TaskQ artifact, PM qualifies only the added caller-bound preparation/drift refusal, scope-control and stdio peer readback, inherited descendants through root/caller/frontend interruption, sealed singleton certification, acknowledged cleanup recovery and original-slot release/readback under the existing Mac Workspace authority. Any unsupported, missing or uncertain observation retains the original operation and occupancy. This worker cannot invoke the owner-only app-server/slot route from its retained `TASKQ_ATTEMPT` context and does not strip it. No TCC/native denial or native PASS is inferred without execution.

### Bounded publication handoff

[Owner/PM note 6571952645](https://github.com/taskqorg/agentq/issues/199#issuecomment-6571952645) directs independent review/publication of the bounded source result before Root's native qualification. It supersedes repeating the owner-route Ask; it does not authorize bypassing the worker refusal or claiming native A1/A3. The [owner-route packet](runtime-199/OWNER-ROUTE.md) now supplies the exact request shapes, checked caller module and one-shot checkpoint writer. Root obtains the real caller-bound receipt and creates the private checkpoint only through the published host under existing authority. No fabricated live preparation, new prerequisite or accepted-operation replay is supplied.

The module's SHA-256 is `16df6ecbd508faf3d18052f0a7dc4e46c0fa2e3679423392901a1be663899619`. Its files use exclusive creation, private Unix file mode, file and directory sync before dispatch, original slot/key/request comparison and a durable launch-intent barrier. A repeated call, changed request, interrupted write or any existing marker refuses without replacing files. Recovery restores the independent preparation and never calls launch. It adds no slot admission/release, native input, resolver, retry loop or environment override; `Intent::Off` is fixed at launch. Root/AssetQ retains the transport, effect orchestration and original slot.

The existing `protected_preparation_and_completion_recover_original_operation` process scenario compiles and calls that exact module with the fake host. It verifies the saved checkpoint/exact request, rejects request-key substitution and duplicate preparation/dispatch, and checks the durable request at the adapter boundary. Existing authority, same-operation recovery, completion, cleanup, unknown-occupancy and no-replay assertions remain. The scenario sends only the invented fake-client handshake; it starts no native host/model/paid operation.

The affected development check initially failed because its existing JSONL wait counted a partial second line as a complete handshake snapshot: the saved right-hand value ended at `{"method`, while the final left-hand value contained the complete `initialized` line. The fixture now waits for two newline-terminated records and retains that same completed read as the snapshot. The final no-replay equality assertion is unchanged. The same `cargo test --locked -p agentq-runtime --test agent -- protected_preparation` command passes after the fix. The frozen affected gate `cargo test --locked -p agentq-runtime --test agent -- protected_ app_transport_ off_reaches_` passes all five scenarios with the checked handoff module.

Production runtime sources and dependencies remain identical to `33cd5046c234dacf25c2ad47f350acf35b9eebdf`; the handoff module and its affected test fixture are new and require current-head CI. Historical full-suite sandbox-blocked outcomes above remain recorded, not relabeled PASS or reused as proof of these new checks. Workflow-pinned contract lint, document file links, artifact hash and whitespace are checked for this handoff. No rebase, native retry, forced restart, new grant, auth copy, model activation or live-attempt transfer was performed.

| Immutable draft criterion | Candidate evidence | Remaining acceptance |
|---|---|---|
| 1: non-launching caller-bound preparation | Checked optional `prepare`, private checkpoint, actual caller validation and process-double checks | PM exact-artifact native preparation |
| 2: launch/input revalidation | Independent prepared equality plus retained authority/artifact/incarnation checks | PM native drift refusal |
| 3: durable native/descendant completion | Host-owned scope/certificate protocol consumed fail-closed; unknown retains occupancy | PM native scope/certificate qualification |
| 4: original slot/recovery/no replay | Exact checkpoint/request, tested one-shot owner-route packet, same-operation reconcile; no release/dispatch/replay in recovery | PM original-slot release and acknowledgment |
| 5: adversarial lifecycle models | Retained host models plus new runtime preparation/transport/completion regressions | Native failure/recovery receipts remain separate |
| 6: publication/CI and PM proof | Source candidate, green prior-head push/PR CI and published TaskQ intent correction | Green AgentQ merge-result CI, exact installed source/artifact and PM native proof |

Task #199 and AssetQ #4 remain incomplete until criterion 6 and the native evidence above are satisfied. Portable success never closes them (P2, M12.8, R12, TaskQ §7).
