# Temporary disable implementation plan

Add two reversible controls to AgentQ: pause automatic compression, and detach the client integrations completely. Both support the host and individual agents. Pausing preserves the current transport and integrations; detaching restores direct client access and removes AgentQ-owned registrations.

Status: implementation plan reviewed by Claude Fable, task fit PASS, quality PASS, overall APPROVE. Baseline: `6aacefcc134e50d591f977f1c8db757fe4562f5d`. The owner selected both modes. Implementation starts by amending [agentq.md](../../agentq.md) and [CONFIGURATION](CONFIGURATION.md); this plan does not amend either contract by itself. Planning did not change working profiles or authorize deployment.

Execution status, 2026-10-17: implementation and independent Fable fixes review are complete. The owner explicitly requested deployment to the existing Mac, Windows and FRA installations. Release 0.8.0 and that bounded rollout are authorized; remaining Desktop/reboot checks stay unqualified and do not require another test VM or approval. Delivery results are recorded separately after rollout.

## Commands and scope

```sh
agentq status [--json]
agentq optimization off|on [codex|claude|hermes] [--dry-run]
agentq disable|enable [codex|claude|hermes|claude-desktop] [--dry-run]
```

Without a target, `optimization` changes the existing host compression switch. `disable` selects every owned integration of this installation; `enable` selects the saved disabled profiles, including profiles whose active ownership entries were suspended during disable. An enable without a saved disable is unchanged, never an onboarding operation. Explicit invocation applies the operation; `--dry-run` previews targets, aliases, owned changes, conflicts and reload requirements without writing.

Clients sharing a configuration root are one target: Codex CLI and Desktop, Claude Code CLI and Desktop Code, and workers using those roots switch together. Honor explicit native configuration roots. Do not discover sibling installations, scan the home directory, or promise a switch for one worker. Desktop Chat supports integration detach only, not automatic compression.

Keep `direct` as the existing emergency routing bypass. Local control is the first deliverable. Do not extend remote mutation values, the enrollment grant or the DashQ UI as part of this change.

## Control behavior

| Action | Required behavior |
|---|---|
| Host optimization off | Set the host compression preference to off. Cover every automatic adapter and anonymous `agentq run`. |
| Host optimization on | Remove the host pause while preserving individual preferences. |
| Agent optimization off or on | Off writes `false`; on removes the preference entry, following the existing contract. Under a host pause, an individually enabled agent remains effectively off with the reason shown. |
| Disable selected integrations | Persist disable intent, suppress new compression, and detach only their owned routing, MCP, hooks, plugin activation and journaled instruction blocks. |
| Enable selected integrations | Restore the saved supported integration, preserving the independent host and agent compression preferences. |

Full disable preserves the installation, releases, engine pins, local data, originals, credentials, queued observations and enrollment. Existing retention limits still apply; enable does not promise indefinite recovery of expired originals. Newly launched detached clients have no AgentQ MCP retrieval until enable.

Do not replace native login, model, approvals, sandbox or foreign telemetry settings. A preexisting or externally modified same-name integration is a conflict, not permission to remove it. Keep shared managed entries while an enabled sibling depends on them. Remove owned Hermes plugin files through the existing restore operation; enable recreates them from the current embedded assets. Shared immutable engine and release assets remain installed.

## Implementation sequence

### 1 Amend the contracts and define transitions

Update the root contract first, then CONFIGURATION and [VERIFICATION](VERIFICATION.md); adjust [PRODUCT](PRODUCT.md) only where acceptance requires it. Specify global and individual preference precedence, saved restoration state, aliases, partial failures, reload requirements and platform qualification. CONFIGURATION currently permits setup but has no removal value, so explicitly define local detach authority without broadening existing remote operations.

Define persistent integration transitions and their restart behavior. Extend routing intent additively with `disabled`. Separate desired enabled or disabled state, observed attached or detached configuration, and runtime verification. A valid configured detach can coexist with runtime evidence that is unknown. Do not make file readback imply that a running client reloaded. Only explicit enable clears disable intent; onboard and service install report the disabled scope and direct the user to enable. Manual binary updates are allowed when they do not activate services or reconnect clients.

### 2 Reuse lifecycle storage and fix compression controls

Reuse [executor.rs](../../agentq/executor.rs), its lifecycle lock and durable operation records, atomic writes, and the existing `compression.enabled` and `compression.agents` settings. Preserve unrelated fields. A malformed or unwritable control store must refuse mutation without replacing its contents. Hold the lock for intent, journal and configuration writes; release it before waiting for clients or operating service units, then reacquire and revalidate the operation generation before any final state change.

Use the shared compression core for run, provider and Hermes adapters. Do not add independent adapter switches. Integration disable also suppresses new compression for that scope. Unknown run identity obeys the host switch; individual isolation requires a known agent identity. Already running commands are not retroactively changed.

Two existing gaps belong in this step:

- [configure.rs](../../agentq/configure.rs) blocks optimization-only changes whenever a project override exists. Apply `project_override` only to fields that write native client configuration; an optimization-only plan does not edit those files. Compute the blocked result from the selected fields and keep relevant ownership, root and readability checks. The local optimization command edits only AgentQ's own preferences under the lock. Keep remote authorization unchanged.
- [profiles.rs](../../agentq/profiles.rs) signs an agent preference and native config bytes but omits the host compression switch and Direct intent. Include `compression.enabled`, the agent's `direct` membership and integration lifecycle intent in revisions so an out-of-band switch invalidates stale plans and evidence.

Codex must keep the bytes of results already transmitted through its durable replay store. Only new results use the new preference. Pause and resume require no client restart or provider configuration write.

### 3 Implement reversible integration detach and restoration

Extend [onboard.rs](../../agentq/onboard.rs) ownership metadata only as needed to select a supported profile's entries, and reuse its place, unplace and restore operations. Historical non-routing journal entries lack an agent ID: map them through validated profile roots and supported owned keys, or report an unresolved scope. Never guess by a path substring or accept arbitrary journal paths.

Before any detach write, durably record its intent and restoration recipe under the lifecycle lock. Keep the smallest record of the managed fields present at disable, prior Direct and optimization preferences, selected profiles, operation generation and transition result. Detach uses scoped `restore_where(forget=true)`; archive the selected journal metadata needed for crash recovery before forgetting it, and remove resolved rollback metadata when it is no longer needed. Do not leave restored entries active, where doctor would report them as drift. Avoid duplicating whole native configurations or credentials into another store. Sensitive local state uses owner-only file permissions and never enters telemetry or snapshots. Keep unresolved transitions outside telemetry pruning and bound resolved records using existing control-storage limits.

Repeated disable preserves the first baseline. Repeated enable returns the recorded or observed result. Check that current detached values still match the operation's recorded outcome, then reapply current canonical `onboard::wants` and `routing_wants`, filtered to the fields that were actually present. Do not replay obsolete registration bytes, executable paths, ports or tool lists from an older release. Preserve later foreign changes and report conflicts, using the existing conflict checks. Journal reapply before file writes and read back actual values. Partial writes and interruption reconcile from durable stages; do not blindly replay an ambiguous mutation or overwrite a newer operation.

Disable must not call uninstall, legacy removal or onboarding. Enable restores only recorded supported integrations, including a prior Direct state, missing optional registrations and independent compression pauses. Prepare existing engines and the proxy, verify listener readiness, then restore routed client configuration. Do not implement new Claude provider routing or fetch unrelated dependencies as a side effect.

Make configure, onboarding, direct and service lifecycle respect persistent disable intent. Only enable may clear it. Onboard with `--yes`, service install/restart and remote configure cannot reconnect a disabled integration; report the disabled scope rather than treating a generic setup flag as enable consent. Specify a fixed rejection/conflict code for a configure request against a disabled profile in the contract amendment and fixtures. Direct may remove routing without clearing disabled intent. Manual binary update remains available, but activation/start paths honor the latch. Keep recovery possible through the explicit enable command even when the service is down.

### 4 Preserve running work and stop the host safely

Removing native registrations does not unload a running client's provider, hooks or MCP connection. Suppress new transformations through the existing policy boundary, preserve transmitted history, and expose `pending_reload` or `pending_drain` when appropriate. Never kill workers or active conversations to finish disable.

Keep any proxy required by existing sessions available while they drain. A bounded command returns the pending state rather than waiting indefinitely or force-closing connections. There is no forced-stop option in this deliverable. The existing 15-minute update deadline is not a disable deadline: an idle Codex process may reconnect using its cached loopback provider after any interval.

For supported clients, take a conservative inventory of client processes and sessions that may have loaded the old routing, using their native lifecycle source where available. Stop only after those dependencies are known to have ended or reloaded, a fresh dependency read finds none, and proxied connections read zero. Unknown process identity, unreadable lifecycle data or an unsupported reload probe leaves the operation pending; never infer safety from socket absence, elapsed time or the operator merely issuing disable. New direct-client qualification proves the supported behavior, not every historical saved session. Document that resuming an old conversation can require a new client session. Repeating disable after the dependency clears must finish the transition without replacing the saved baseline.

Do not fabricate a per-client verified-detached evidence source after its hooks have been removed. Record configured detach from disk readback and `pending_reload`, `pending_drain` or unknown runtime state independently; existing hook timestamps and TCP observations alone cannot verify the detached client. If a platform cannot determine the required dependencies, it cannot advertise automatic safe service shutdown for that path.

A host disable latch suspends AgentQ export (except the enrolled DashQ snapshot, agentq.md § 4), automatic update activation and remote configuration application while preserving queues, grants and the information needed for enable. These gates are necessary while a pending drain still keeps the existing service alive; after the service is stopped, no separate gate or daemon runs them. During draining, only the transport and recovery work required for the transition remain. Per-agent disable keeps the shared service working for enabled siblings and reports that scope.

For a host disable, stop or unload the existing service through coreq only when its transport dependencies are safely gone; keep the installation and the saved service-registration recipe needed by enable. Audit launchd, systemd and Windows task stop/load behavior rather than calling AgentQ uninstall. The latch must survive logon, reboot and explicit service restart without reconnecting integrations. Once a selected Codex port has drained and can stop, `routing::supervise` must not respawn its worker while the shared service continues for other profiles. Reuse the existing service for pending work; add no daemon or scheduler. Enable restores the previously enabled service/control-plane behavior in a safe order, with readiness before routing.

### 5 Add truthful status and documentation

Reuse profiles, routing and service observations. `agentq status` works with the service stopped through fresh local reads and durable intent. Report intentional service shutdown separately from stale or unreadable service evidence; an old `status.json` never proves running or verified.

Expose per profile: aliases, desired integration state, observed configuration, optimization preference and effective reason, runtime verification, reload or drain requirement, conflicts, and observation time. Summaries use on, off, mixed or unknown; unsupported compression profiles do not count as enabled. An intentionally detached profile is not accidental routing drift in `doctor`, but genuine failures remain visible.

Keep existing status and snapshot fields compatible. Add bounded optional lifecycle fields through [snapshot.rs](../../agentq/snapshot.rs), with no paths, raw config, credentials or arbitrary errors. An offline remote dashboard cannot claim a fresh stopped state. Update README and INSTALL with both modes, aliases, recovery, examples and limits.

## Verification and delivery

Reuse existing checks; add the smallest cases protecting the following distinct risks.

| Risk | Required proof |
|---|---|
| Pause changes the wrong scope | Host and agent precedence, anonymous run, sibling isolation, and compression-only changes with project overrides. |
| Pause alters command behavior or history | Native stdout, stderr and exit code preservation; unchanged prior Codex replay; new results bypass compression. |
| Enable changes the saved setup | Round trips for paused, Direct, absent and partially configured baselines; shared aliases and instruction entries; foreign values remain intact. |
| A crash loses recovery | Failure at persistence and file-write boundaries, duplicate operations, stale revisions, and interrupted or partial detach/restore reconciliation. |
| Disable reconnects itself | Update, service restart, reboot/logon behavior, onboarding and configure respect the latch; fixed disabled-profile rejection; a drained disabled proxy is not respawned for enabled siblings; queues and grants remain available for enable. |
| Status claims a false result | Stopped-service local status/enable; active or unknown dependencies; pending reload; unknown runtime evidence never verified. |
| Native clients stop working | Installed-client pause/resume and disable/enable, direct access, preserved native settings, original recovery and active worker continuity. |

Extend existing compression, configure, executor, onboard, provider and Hermes checks and fixtures. All lifecycle fixtures use isolated install/client roots, `AGENT_CONTEXT_HOME` and `npm_config_prefix`. Never exercise removal against real npm or installed profiles.

During implementation, run affected checks. The merge candidate runs `cargo test --locked`, `cargo run --locked -- selftest`, and the affected existing Python adapter checks. macOS qualification must cover native Codex and Claude CLI/Desktop aliases. Other advertised platform/client lifecycle paths need their own affected checks before claiming the new mode supported. Paid native calls, deployment and changes to working profiles need their own explicit qualification scope; none is part of this planning task.

Deliver one outcome through sequential stages: contract and status shape, compression pause, reversible detach/restore with safe service lifecycle, then qualification and documentation. The planning deliverable has no version bump, release or host rollout.

## Review record

Claude Fable review session: `6d5ed5ff-b778-4290-bf4c-b8664d3e9ad0`, requested model `claude-fable-5[1m]`, observed response model `claude-fable-5`. Read-only review against the baseline: task fit PASS, quality REQUEST_CHANGES. The revision incorporates scoped journal restore with forget, canonical reapply, additive disabled intent, exact revision inputs, false-or-absent agent preferences, per-field override checks, plugin-file restore semantics and short lock scope.

Two reviewer suggestions are deliberately replaced: an elapsed 15-minute drain cannot prove idle clients safe, so uncertain dependencies remain pending without a forced stop; and onboard with `--yes` does not clear disable intent, because restoring a deliberately disabled integration is the enable command's explicit purpose. Service export/update/remote-write gates apply only while the pending host transition still needs its transport. Keep the requested status command and the closed remote mutation envelope.

Final review: session `71467361-2a92-4ea8-93cc-1e1ef45a21cf`, model `claude-fable-5`, task fit PASS, quality PASS, overall APPROVE; no unresolved blockers or product choices. The reviewer accepted the conservative drain and enable-only intent rules. Its two non-blocking implementation notes are included above: a fixed disabled-profile rejection code and a no-respawn test for the drained selected proxy. Acceptance applies to the plan; platform and client behavior remains subject to the implementation gates.

## Implementation candidate and remaining qualification

Source candidate prepared on 2026-10-17 against the baseline above. The contracts, CLI, shared compression policy, scoped detach/canonical restore, service gates, status, snapshots and usage documentation are implemented. Fable approved the plan and completed an independent read-only implementation review. Its two status findings are fixed; the bounded follow-up (private artifact `fable-fixes-review.md`) confirmed both and found no introduced regression. Full native acceptance remains open.

The current isolated source gate (private artifact `integration-fable-fixes-gate.json`) records source hashes, binary hash, commands and logs: 94 unit and 6 integration tests, selftest and four Python suites passed. The Windows embedded-CLI case was skipped on macOS. The runner (private artifact `run-source-gate.py`) isolates lifecycle/legacy state and npm prefixes. Earlier gate files remain historical evidence, not the current artifact identity.

### Authorization and artifact boundaries

The initial eight-call ledger (private artifact `native-authorized-budget.json`) and subsequent four-call ledger (private artifact `native-continuity-budget.json`) remain unchanged. The owner then explicitly removed the QA model-call limit. Further calls are recorded in the unbounded QA ledger (private artifact `native-unbounded-qa.json`), with `limit: null`. No further call-budget approval is needed. Native model, approval, sandbox and managed authentication remain unchanged; credentials were not read or copied. The later explicit host rollout request supersedes the earlier isolation-only restriction for installation updates and reversible checks.

Native evidence is tied to each report's binary hash. The seven-phase Claude lifecycle and Codex pause/resume used `0cef4d9b3858c34678fbfaf35c815f227cbf7e47dcd47ab28fce6d1dea1fd851`; later changes add truthful hook/status metadata and operation ordering. Codex also passed a one-turn check on `c019c8c425c3e4c3186f54c9f0f5110d4163393f953994232e0a360cd0fe8094`. The Claude smoke binds the preceding watermark-fix artifact and PostToolUse-based verification. The current artifact additionally changes only the service-status evidence gate and its regression; its own read-only native service observation is recorded separately. These later status fixes do not requalify unrelated Desktop/platform paths.

### Observed native behavior

| Path | Result | Evidence boundary |
|---|---|---|
| Codex CLI attached → host pause → resumed | 7766 → 429, then native 7766, then 7766 → 429; correlated records, model-visible references and complete original recovery | Native pause/resume (private artifact `native-codex-post-tool-qa.json`); private proxy, fresh CLI sessions, no OS unit lifecycle |
| Codex one-turn artifact check | 7766 → 429 and complete original recovery on c019 | Artifact smoke (private artifact `native-codex-integration-final-qa.json`); compression/control code unchanged in current candidate |
| Codex app-server, four earlier turns | Attached compression; same thread/PID survived direct + disable; fresh direct access; enable preserved Direct and restored native MCP original recovery | Continuity (private artifact `native-codex-continuity.json`) and audit (private artifact `native-codex-continuity-audit.json`). Direct was prepared first; routed service-baseline restoration remains open |
| Claude Code 2.1.302 auto mode, seven phases | Attached success 7347 → 119; pause native; resume compressed; disable native in the same session; enable compressed in the same session; intentional failure native exit 1/full output; fresh enabled session compressed. Originals recovered; native tool IDs correlate | Lifecycle (private artifact `claude-auto-lifecycle.json`). First six phases share session/PID; managed policy unchanged; successful Bash uses PostToolUse replacement |
| Claude watermark-fix artifact and status | Successful automatic compression and actual PostToolUse verification after rapid optimization off/on | Current smoke (private artifact `claude-integration-review-smoke.json`); earlier same-second ordering failure (private artifact `claude-integration-final-smoke.json`) retained |
| Linux systemd user service in an isolated Docker VM | Actual stop/disable, restart rejection, user-manager restart and container reboot preserve disable; enable restores the real unit. Retained original survives disable/reboot/enable | Lifecycle (private artifact `systemd-native-lifecycle.json`), retention (private artifact `systemd-final-retention.json`). No host mounts, production units or credentials. Control plane only; no native routed client |
| Codex Desktop | Computer Use denied `com.openai.codex`; zero Desktop turns | Restriction (private artifact `desktop-qualification.json`). No alternate automation route around the denial |
| Claude Desktop | Native Local-environment editor rejects per-environment `CLAUDE_CONFIG_DIR`: it is allowed only for all sessions. The unsaved QA draft was discarded; zero Desktop model turns | UI preflight (private artifact `claude-desktop-native-qa.json`). Independent native user/VM isolation remains necessary; Desktop Code and Chat MCP stay unqualified |

All volume measurements are fixture observations, not billed-token savings or production-stability claims. A temporary named native Codex profile selected private QA configuration with managed authentication; the profile was removed. Earlier MCP loader checks (private artifact `native-loader.json`) prove loading only; the standalone Bash probe does not prove automatic hooks.

### Defects found and fixed

- During the owner-authorized 0.8.0 rollout, Windows dry-run found repeated legacy MCP receipts with identical owned values. Detach incorrectly evaluated each receipt against the original disk file, so an already simulated removal appeared as a conflict on the next duplicate. Version 0.8.1 evaluates the simulated file instead; the regression failed before and passed after, proves all duplicate sources leave the active journal, canonical restoration works, and later foreign values remain untouched. The private `receipt-compat-gate.json` records the full source gate; no working ownership journal was manually deduplicated.

- Service stop now checks actual loaded executable/arguments as well as recipe ownership, preventing a same-name foreign registration from being unloaded. The CLI regression failed before and passed after; Mac/Linux/Windows parser cases are covered. A read-only native Mac probe (private artifact `native-service-identity.json`) confirmed identity without changing the unit.
- Paused Claude hooks return an empty response and preserve native command classification.
- Native Claude auto mode rejected the old rewritten Bash command with `[Auto-Mode Bypass]`. The safe fix uses the documented `PostToolUse.updatedToolOutput` for successful Bash results, without rewriting commands or changing native permissions. The capability probe (private artifact `claude-post-output-probe.json`) and lifecycle above passed. PostToolUseFailure cannot replace failed output: failed, interrupted and image results stay native. Outside auto mode the existing PreToolUse/run adapter remains.
- Canonical Claude onboarding includes PostToolUse. Older PreToolUse-only auto profiles need explicit hook configuration; enable preserves an absent optional registration rather than silently adding it.
- PostToolUse failures now appear in hook diagnostics/heartbeat. Claude effective status names PostToolUse in auto mode and requires that event for verification; PreToolUse alone cannot verify result replacement.
- Two configuration writes in one second selected an older revision. A private monotonic journal sequence now orders verification and pruning under the existing lifecycle lock, including wall-clock rollback. The extended executor regression failed before and passed after; legacy records keep timestamp ordering.
- A preexisting hook timestamp after clock rollback could falsely verify a later apply. Applied readback now captures a private per-profile watermark and requires a strictly newer event; legacy records without the watermark remain `no_evidence`. The regression failed before and passed after, followed by native Claude compression/status readback on that watermark-fix artifact.
- Disabled onboarding allocated ports before the lifecycle lock and intent check. Port allocation now shares the existing lock with preferences, journal and native writes. The regression proves disabled onboarding leaves settings byte-identical and never calls start/restart. These additional regressions (private artifact `additional-review-regressions.json`) are included in the current gate.
- The conservative client dependency inventory also made independently observed service status unknown on a developer machine with Node/Electron/Python. Service status now uses its own process/heartbeat/unit evidence; dependency uncertainty still blocks profile runtime/drain claims. The extended real-CLI regression failed before and passed after for running, stopped and actual process-read failure. A read-only Mac check (private artifact `native-mac-readonly-service-status.json`) confirmed the existing 0.7.2 service as running without deployment. The current Linux systemd proof (private artifact `linux-service-status-qa.json`) confirms the actual candidate stays running during an interpreter-blocked disable and reports intentional stop only after that explicitly owned QA dependency ends; the offline container is stopped afterwards.
- Current Codex Desktop executable `ChatGPT` is retained by conservative drain inventory. The regression failed before and passed after; parser proof does not qualify Desktop reload/shutdown.

### Independent implementation review

Claude Fable `claude-fable-5` completed read-only review in session `5d1dea40-6bf8-4bb6-8a5e-f1589736a60f`: report (private artifact `fable-implementation-review.md`). It found the old-hook watermark issue (F1) and service/dependency-status conflation (F2); both are covered above with failing-before/passing-after regressions. Code quality passed. The completed follow-up (private artifact `fable-fixes-review.json`), observed model `claude-fable-5`, confirms F1 and F2 resolved and the onboarding lock correct, with no introduced regression; task fit and code quality are good. It checks the current fixes and onboarding-lock delta only and cannot approve missing native release gates. A failed background-copy attempt produced no transcript; the bounded one-shot report is the authoritative follow-up. The original review read some files before the concurrent watermark fix, so its old line references remain historical.

### Linux gate limits

The unprivileged Linux full unit run passed 91 tests and failed `resource::tests::live_probe_overhead`: Docker's overlay root is absent from the physical-filesystem probe. It is not a full-suite pass, and the assertion was not weakened. The affected Linux gate (private artifact `linux-focused-corrected-gate.json`) passed integration tests, selftest and four Python suites after correcting the QA `/tmp` noexec mount. Linux lifecycle/retention evidence names its own artifact; subsequent executor/status and onboarding-lock changes do not alter the tested lifecycle path. The affected Linux checks (private artifact `linux-additional-review.json`) separately exercise the executor and onboarding-lock changes, and the latest native systemd check (private artifact `linux-service-status-qa.json`) exercises the subsequent service-status fix on its own current-source Linux artifact. The owned QA container is offline and is stopped after evidence collection.

### Remaining acceptance

| Requirement | Proven scope | Remaining gate |
|---|---|---|
| Pause/resume and native output | Mac Codex and Claude CLI, exact original recovery; failure output preserved | Affected Desktop and native Hermes paths |
| Reversible detach/restore | Four-kind fixtures; Direct/paused/optional baselines; foreign edits; interrupted transitions; native Codex Direct baseline and Claude same-session enable | Codex originally routed service-baseline restoration; Desktop aliases |
| Persistent latch and original retention | Guards/queues/grants fixtures and actual Linux systemd restart/reboot/enable | Actual macOS/Windows stop/load and logon/reboot; other advertised platforms |
| Conservative drain | Active/unknown inventory, ChatGPT detection, failed sockets and no-respawn regression; native CLI continuity | Real routed provider shutdown/reload and old-session resumption |
| Truthful status and privacy | Current source checks, bounded projection, actual Claude PostToolUse verification | Fresh affected Desktop/platform evidence |

The owner confirmed that no separate native test environment is currently available and subsequently requested deployment to the existing hosts. Desktop and macOS/Windows reboot/logon acceptance remain unqualified; do not reboot working hosts or bypass the Desktop tool denial to close these gaps. The [manual native worksheet](INTEGRATION-CONTROL-MANUAL-QA.md) provides fixtures, supported isolation prerequisites, exact Desktop steps, OS-manager readback and evidence requirements. Missing isolation/access remains unknown; unlimited model calls do not resolve the denied Desktop tool path.

Before the explicit rollout request, no release or working installation was changed. The owner now authorizes 0.8.0 release and deployment to Mac, Windows and FRA, with the gaps above retained. Required CI checks still gate publication. Report installation, service and reversible-control readback per host separately from full native acceptance. Retained originals/evidence remain available.
