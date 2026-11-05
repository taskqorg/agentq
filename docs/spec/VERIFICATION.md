# Minimal product acceptance

Status: target gate for [PRODUCT revision 3](PRODUCT.md). Test observable outcomes through real interfaces. A scenario may cover several requirements and use a small data table; do not write a test per function.

## Core scenarios

Temporary-control acceptance ([contract](../../agentq.md#temporary-controls)): host/agent optimization precedence, anonymous run and unchanged stdout/stderr/exit/replay; optimization-only changes with project overrides; disable/enable round trips preserving Direct/paused/absent fields and foreign edits; aliases/sibling isolation; interrupted journal/config writes and duplicate operations; disable intent across onboarding/configure/update/service restart; pending/unknown dependency inventory without forced stop; no respawn of a drained disabled proxy; local status/enable while stopped; retained grants/queues/originals. All fixtures isolate client/install roots. Native Mac CLI/Desktop qualification must prove active-work continuity and direct access under the same settings/authentication. Other platforms/clients need their own affected evidence before the new mode is advertised as qualified.

Candidate execution, 2026-10-17: [gate, artifact boundaries and remaining qualification](INTEGRATION-CONTROL-PLAN.md#implementation-candidate-and-remaining-qualification). The current Mac source gate passed 94 unit and 6 integration tests, selftest and four Python suites. Native Codex pause/resume, cached-session continuity, fresh direct access and enable preserving Direct passed with original recovery. Native Claude auto-mode seven-phase lifecycle passed, including pause/resume, same-session disable/enable, fresh enabled execution and preserved failure output; successful commands use PostToolUse without changing native permissions. Actual isolated Linux systemd stop/restart/reboot/enable and original retention passed. The Linux full unit run retains one overlay-filesystem resource-probe failure; affected integration/selftest/Python checks passed. Desktop, native macOS/Windows lifecycle and Codex routed service-baseline restoration remain open. Independent Fable implementation review found two status defects; both fixes passed failing-before/passing-after regressions and a focused Fable follow-up. The current candidate also passed native read-only Mac service status and actual Linux service-status/drain checks with an unrelated interpreter. The owner confirmed no separate native environment is available; Claude Desktop UI prohibits per-environment config-root isolation, and Codex Desktop tool access remains denied. Current-artifact Claude PostToolUse verification and rapid-operation ordering have dedicated evidence; see the linked record for hashes and scope.

| ID | Chain and expected outcome | Requirements |
|---|---|---|
| A1 | One-link onboarding → selected real client round trip → correct installation/agent visible centrally; repeat preserves unrelated configuration | R1, R5, R9–R10 |
| A2 | Eligible task → native tool/optimization → correct final result; protected code/JSON/constraints unchanged; requested original recoverable | R2–R4 |
| A3 | New recipe/release → verified stage → active-work deferral → switch or rollback; identities/history unchanged and actual version visible | R2, R6, R8–R10 |
| A4 | Tool/proxy/exporter unavailable, duplicate delivery or full storage → work preserved or direct access restored; no double count; truthful backlog/incident | R4, R8–R10 |
| A5 | Dry-run → uninstall → native client access; only owned components removed, reused tools/later edits preserved; repeat no-op | R7 |
| A6 | Two owners/profiles/runtimes → correlated usage/health → dashboard and agent query agree; members cannot read another owner's data | R1, R9–R10 |

Start with six scenario families, not six claims covering every possible input. Parameterize supported clients/OS and critical boundary cases. Use the same release artifact and approved versions.

## Necessary extra checks

The § 3 compression chain (`tests/compression.rs`, task [#120](https://github.com/taskqorg/agentq/issues/120), approved behavior) starts with writable isolated storage and a native runner. It proves one execution, stderr/exit preservation, bounded filtering, smaller output including its recovery ref, MCP original recovery and one local record per call. Protected text, engine error, timeout and larger output return native bytes; missing or corrupt stats remain unknown. The process double checks the adapter, not RTK quality or client integration.

The profiling chain (`agentq/profile.rs` tests, task [#122](https://github.com/taskqorg/agentq/issues/122)): stats records and export samples rank worst first by failure, time and instability budgets; an empty store, a corrupt stats record, corrupt or non-object export state (every export) and a malformed sample (that export only) and a corrupt trace directory or line read unknown for their own operations only; a corrupt config or invalid `trace_until` reads the switch unknown, never off; the trace switch is bounded and keeps other config keys. Live send and delivery timings on a host are not qualified by it.

Include actual Claude success/failure hook formats and Codex string/typed/nested Responses output; validate protected fields, HTTP/SSE/WebSocket where claimed and cancellation. Vendor tests alone do not qualify our adapter. Shared evidence rules: [specq verification](https://github.com/taskqorg/specq/blob/main/engineering/verification.md).

## Automatic compression matrix

R3/R4: [compression qualification](COMPRESSION-BENCHMARK.md) records the reused research, offline real-engine benchmark, native carrier chain and opt-in installed-client check. `python tests/provider_adapter.py` runs in CI without paid clients.

Mandatory local telemetry acceptance of the same native run, with optional connected-mode delivery verification: [telemetry qualification](TELEMETRY-QUALIFICATION.md), ACCOUNTING O4. Read back the actual compression event, not only `agentq.verify`; correlate native session/tool IDs and report usage coverage separately.

## Installed CLI compression check

A2: `python3 scripts/check-client-compression.py both` tests the installed Codex CLI and Claude Code CLI with their existing configuration and authentication. Native `cargo test` must run through the automatic adapter; explicit `agentq run` in the prompt cannot qualify it. PASS requires a new successful compression record matching the native client session, smaller bytes, the model quoting its recovery reference and unique failure marker, and `get_original` returning all fixture cases and the measured original byte length. Missing prerequisites or incomplete execution read unknown, never pass. The optional check uses paid model access; CI runs `python tests/client_compression_check.py` to check false-positive prevention without a model.

Mac check, 2026-10-15 10:17–10:18 UTC, installed agentq 0.6.1, Codex CLI 0.159.3 and Claude Code 2.1.299: both native clients ran the fixture once, reported 180 passed / 1 intentional failure (native exit 101), and returned no original reference. Each client exited 0. Each produced zero new local compression records; both integration checks FAIL. Codex session `01a13f11-8ace-7242-9623-10e899330ecd`; Claude session `99ea34e3-b341-4466-96f0-643614935529`. No install, routing, hook or service changes. This proves a missing automatic compression path on this Mac, not a Windows or FRA result.

## Optimization experiment

Paired baseline/candidate tasks use the same model/auth/client, frozen inputs and independently checked outcomes. Include all input/cache/output, tool/schema/instruction overhead, retries, retrieval/rework, latency and monetary source. Reverse run order and distinguish cold/warm cache and state. A smoke case is not statistical proof.

Reuse selected LongBench/RULER cases and official scorers for content quality; use a few executable coding/terminal tasks for task success. Do not run full research benchmarks on every deploy.

Quality, latency, resource and savings budgets must be agreed before engine selection/release. No code-level test suite or numeric target inherited from the old implementation.

Worker runtime benchmarks (`python3 scripts/benchmark-workers.py`, [COMPRESSION-BENCHMARK.md](COMPRESSION-BENCHMARK.md#worker-runtime-benchmark-methodology-and-viewer)) execute the four standardized worker tasks (`T1-context-navigation`, `T2-instruction-adherence`, `T3-noisy-command-compression`, `T4-code-patch-verification`) across `codex`, `claude` and `antigravity` with `optimization` `off` vs `on`, storing dated/versioned JSON reports outside the repository and rendering them in the generated viewer.

## Accounting qualification

Live check for [ACCOUNTING O4](ACCOUNTING.md#o4-qualification) (#108). Persistent chain checks: `accounting::tests::consumer_fixtures` runs every published case, producer-path cases included; `tests/compression.rs` requires equal accounting from local stats and their queued spans, unknown for an empty store, and a gap, reset or unknown reason after a failed append, a lost journal or a corrupt one, and unknown for a deleted day file under an intact journal. A call that can write neither its record nor a failure marker leaves no evidence, so local observed intervals carry reason `unproven` and local totals stay lower bounds (`compression::tests::no_evidence_chain`).

Run 2026-10-14 on Windows 11 Pro 10.0.26100, candidate commit a05b667 (debug build, `agentq 0.0.0-dev`, unreleased), RTK 0.51.0, cargo 1.99.0, git 2.46.0.windows.1, in an isolated install under the OS temp dir; no profile or working install changed. Client: Claude Code 2.1.299 headless (`claude -p`, model `claude-opus-5-5`, user settings excluded with `--setting-sources project`, two allowed Bash commands). In one real turn the model's Bash tool ran `agentq run -- cargo test` in a 150-test crate and `agentq run -- git --version`; `AGENTQ_AGENT=claude` and `AGENTQ_SESSION` = the client's session ID. The client received RTK's summary with the `get_original` ref line plus native stderr. Then `agentq telemetry verify` posted the queue through the real sender to a loopback Langfuse stand-in (Python 3.12.10); its stored body is the delivered events.

| Check | Local stats (`--from` journal `since`) | Delivered events |
|---|---|---|
| State | complete, observed from the first record through now, no gap (since then: partial, `unproven`, see below) | partial: exported events carry no coverage |
| Calls, outcomes | 2: 1 compressed, 1 passed, 0 failed | same |
| Reasons | compressed 1, not_qualified 1 | same |
| Volume run/rtk | 1 paired: 5222 bytes in, 132 out (stdout; stderr is never compressed), reduction 0.9747 | same |
| Cohorts | agent `claude`, adapter `run`, runner `cargo test`/null, engine `rtk` 0.51.0/`none`, version 0.0.0-dev, profile unknown | same |
| Event IDs | 2, session = client session | the same 2 `agentq.compress` spans, plus 3 `agentq.verify` |

No host path, command argument or tool output appeared in either aggregate or the delivered body. Unsupported adapters: the Claude Code `PreToolUse` rewrite into `agentq run` is still a no-op hook, so the client ran `agentq run` as a command; the Codex loopback provider and the Hermes plugin do not call the core; hook and provider recipes are unqualified (`not_qualified`). Gaps: native turn and tool call IDs are unknown; no Langfuse or DashQ ingestion read-back (no authorization to write test events to a production project); profile is unknown on every record; macOS, Linux and WSL untested.

## Resource sample qualification

Procedure for the live task (#86) on each authorized host, at most 30 minutes per host; contract: [agentq.md § 6 Resource samples](../../agentq.md#resource-samples). Linux, macOS, Windows and WSL 2; WSL 1 reports CPU and memory unknown by design. An untested host is reported as untested, never as passed.

1. Install the candidate release; record OS and version, `agentq --version` and the release commit as the producer revision.
2. `agentq telemetry verify` passes.
3. Wait four buckets (20 minutes). From the Langfuse public API list traces named `agentq.resource` since the install; keep those whose `agentq.installation` equals `<install>/data/installation`. Pass: at least three consecutive buckets, one trace each, start times 300 ± 30 s apart.
4. Within 60 s of one sample, read native values. Linux: `vmstat 300 2` (us + sy + st over the same interval), `free -b` (total, available), `df -B1 /`. macOS: `top -l 2 -s 300 -n 0` CPU usage, Activity Monitor Memory Used, `diskutil apfs list` for the `/` container. Windows: Task Manager CPU and Memory In use over the same window, `fsutil volume diskfree C:`. WSL 2: the Linux commands inside the distribution; its values are the VM's, never compared with the Windows host's. Pass: CPU within 5 points, memory within 2 % of total, disk total exact and free within 1 GiB.
5. Privacy: the exported attributes hold no username, home path or volume name.
6. `agentq service restart`: the next bucket holds one sample, CPU known or `warmup`; no bucket holds two traces.

Record per host: OS, producer revision, three trace IDs, exported and native values, verdict. A graph claim needs at least three samples.

## Update restart qualification

Live check for agentq.md § 5 Update (#111, #116): from a service running the previous release, `agentq update` on the host, then poll `agentq service status` every 2 s for at most 90 s. Pass: within 60 s the service pid changes and its exe is `<install>/releases/<new>/agentq` answering `--version` with the new release, with no manual restart. A 0.5.6 service lacks the restart code, so the first proof runs 0.5.7 to 0.5.8; hosts reached 0.5.7 by their own hourly update.

Run 2026-10-14, 0.5.7 to 0.5.8, CLI `agentq update`. No host routes an agent, so no proxy worker port was open and no turn was in flight; each old service logged `agentq: restarting into 0.5.8 (no open proxied connection)`.

| Host | OS | Old pid | New pid | Update (UTC) | Serving 0.5.8 (UTC) | Verdict |
|---|---|---|---|---|---|---|
| Mac | macOS (Darwin 27.0.0 arm64) | 1069 | 13906 | 02:56:40 | 02:57:09 (+29 s) | pass |
| FRA | Linux 6.8.0 x86_64, user hermes | 1151912 | 1166920 | 02:57:16 | 02:57:56 (+40 s) | pass |
| WSL | WSL 2, Linux 6.18.33.2 x86_64 | 39456 | 43375 | 02:57:56 | 02:58:30 (+34 s) | pass |
| Windows | Windows 11 Pro 10.0.26100 | 9020 | 35096 | 02:58:53 | 02:59:17 (+24 s) | pass |

Drain with an open proxied connection (deferral log, then restart once it closes) is covered by unit tests only; no routed host was available.

## Heartbeat routing and hook state (#134)

Live check for agentq.md § 6 `agentq.installation` (`agentq.routing`, `agentq.hooks`), release v0.6.0 (tag at bd2b4d9, release CI green), run 2026-10-15 on Windows 11 Pro 10.0.26100.

- Isolated install (OS temp dir, no Langfuse keys, so spans stay queued): the released `agentq-0.6.0-windows-x86_64.exe` (sha256 matches `SHA256SUMS`) ran `hook codex Stop` with invalid UTF-8 stdin (answered `{}`, exit 0), then `service run`. Its start heartbeat carried `agentq.hooks` `[{"agent":"codex","event":"Stop","state":"failed","last_ok_at":null,"last_failure":{"at":…,"error":"stdin: stream did not contain valid UTF-8"}}]` and `agentq.routing` `{"codex":{"routing":"direct","reason":"config.toml selects no agent_context provider","intended":"direct","intended_reason":"not onboarded"}}`. After one successful `hook codex Stop`, the next heartbeat read `state` `ok` with the same `last_failure`, and `agentq.hook_failures` read `none`.
- Working install: `agentq update` moved the service from 0.5.9 to 0.6.0. A forced `hook codex Stop` with invalid UTF-8 stdin made `doctor` report the failure. The 0.6.0 start heartbeat after it was delivered: `telemetry status` showed the last delivery 13 s after the start, error none.
- Langfuse readback (`GET /api/public/v2/observations?name=agentq.installation`, the installation's own keys), 2026-10-14 UTC. v0.6.0 published 19:28:57; each service took it on its hourly update check and its 0.6.0 start heartbeat carried `agentq.routing` and `agentq.hooks`:

| Host | Runtime | Last 0.5.9 heartbeat | First 0.6.0 heartbeat | `agentq.routing_problems` | `agentq.hooks` |
|---|---|---|---|---|---|
| DESKTOP | windows | updated by `agentq update` | 19:30:57 | codex unknown (proxy not answering) | `[]`, then codex Stop failed |
| macbook-m2.local | macos | 20:17:57 | 20:25:56 | codex unknown (proxy not answering) | codex SessionStart and Stop ok, `last_failure` null |
| hermes-fra1 | linux | 20:25:42 | 20:25:53 | codex unknown (proxy not answering) | `[]` |
| DESKTOP (WSL) | linux | 20:26:09 | 20:26:23 | codex unknown (proxy not answering) | `[]` |

- Forced failure: on Windows `hook codex Stop` with invalid UTF-8 stdin failed at 19:31:07; the next heartbeat (19:32:11) carried `agentq.hooks` codex Stop `failed` with `last_failure` `{"at":1792006267,"error":"stdin: stream did not contain valid UTF-8"}` and `agentq.hook_failures` `codex Stop (stdin: stream did not contain valid UTF-8)`; the hourly one (20:32:15) still did, with `agentq.routing_problems` `none`.
- Found: every 0.6.0 start heartbeat read codex `unknown`, because the sender beats before the headroom proxy binds (37 s and 48 s measured on Windows); the hourly heartbeats read `agentq.routing_problems` `none` (Windows 20:32:15; hermes-fra1 21:25:54, mac 21:26:13, WSL 21:26:29). The start heartbeat now waits until no agent reads `unknown`, at most 5 min (agentq.md § 6); it ships in the next release.
- Not checked: dashq Attention rendering (dashq#116) and a forced failure on mac or hermes-fra1 (no host access).
- Found: `agentq service restart` on this host ended the `AgentQ-host` task and then failed `schtasks /Create` with Access denied, leaving no service. `schtasks /Run /TN AgentQ-host` restored it.

Hermes onboarding ships the §4 `agentq` transform plugin through the existing ownership journal. `hermes_keeps_foreign_enabled_and_disabled_plugins_on_install_and_restore` proves repeat install/removal preserves other plugins; foreign source conflicts prevent activation before writes. `python tests/hermes_adapter.py` checks native transform metadata, envelope preservation and fail-open behavior. Installed gateway dispatch and model visibility require separate native qualification.

## Shared worker runtime

PRODUCT R13 and agentq.md M12.1–M12.7: `cargo test --locked -p agentq-runtime` runs deterministic Claude/Codex clients on each CI platform. It retains CLI/background/app/app-server, Remote Control, input/terminal recovery, dynamic tools, private drafts, catalogue, account/policy and idempotent attributable usage protection from CoreQ v0.18.0. JSON turns need no optimizer installation; launch intent and host-disable tests preserve native output and prior replay bytes. The external consumer in `examples/runtime-consumer` builds from a published immutable runtime revision and runs both fake clients. These checks do not qualify a consumer's native installation or UI; adoption and rollback remain [consumer-owned](RUNTIME.md#adoption-and-rollback).

## Antigravity and worker benchmark candidate

PRODUCT R13, agentq.md § 12: regression checks cover native hook `toolCall.args`, permission delegation, start/resume arguments, transcript boundaries and partial records, terminal tool-call handling, usage deduplication, missing counters, limits and configuration attachment. Benchmark smoke histories are protocol checks only. Native Antigravity cancellation, account isolation, usage semantics and installed compression remain unqualified until observed with the caller's native environment. The HTML viewer requires visual inspection at desktop and narrow widths; fixture scores do not qualify native quality or savings.

Candidate execution, 2026-11-01, macOS arm64, runtime source `4f4a43d` (task #187):

- Before: `hooks::tests::native_claude_rewrite_preserves_input_and_ids` failed on the documented Antigravity `toolCall.args` input; `usage_counts_each_message_once_and_unknown_stays_null` failed because omitted cache counters became `Some(0)`. After: both same regressions pass. Native permission delegation and unknown usage protection remain.
- `cargo test --locked`: 116 root unit tests passed; `legacy::tests::edited_after_journal_is_stripped_not_refused` is sandbox-blocked at `agentq/legacy.rs:473`, `PermissionDenied: Operation not permitted`. This is not a full-suite pass. Both `AGENT_CONTEXT_HOME` and `npm_config_prefix` were isolated in OS temporary storage. No legacy behavior or assertion was changed.
- The checks Cargo did not reach were run separately: `cargo test --locked -p agentq-runtime --lib` (12 pass), `cargo test --locked -p agentq-runtime --test agent` (13 traces pass, including native submission/receipt, transcript completion, resume and timeout), and `cargo test --locked -p agentq --test compression --test control --test hook` (6 pass). An inherited `AGENTQ_OPTIMIZATION=inherit` initially failed the JSON harness's clean-environment assertion; clearing that variable in its existing isolated setup made the same trace pass without changing the assertion or inherited-off protection.
- `cargo run --locked -- selftest` passed with `AGENTQ_SELFTEST_OFFLINE=1`. Contract lint from the workflow-pinned specq `7e9adf57287553176d8b3be61c2b22565543dc51` passed with zero errors; size/headroom warnings remain. The injected rules checkout contains prose but no lint script, so the exact pinned script was fetched through the GitHub contents API into OS temporary storage.
- The four existing Python adapter/checker suites passed (26 cases, one platform skip). `python3 scripts/benchmark-workers.py --selftest` verifies missing usage, failed native execution without a fake fallback, executable fixture failure/patch checks and immutable history. Historical smoke matrices retain every cell and keep token/savings values unknown; they are not live worker comparisons. A final smoke run isolates both parent and child homes and records its runner and candidate binary hashes.
- Isolated `agentq hook antigravity PreToolUse` CLI readback: on rewrites the native command and delegates permissions with `decision: allow`; off returns `{}`. No working profile or service was changed.
- Chrome accessibility readback loaded the standalone viewer over loopback with all 24 records, history/filter controls, evidence disclosures and explicit synthetic/unknown labels. Screenshot capture timed out, and a subsequent interaction stopped when the user changed the app. Pixel layout, narrow-width behavior and interactive filter acceptance remain unverified. Browser automation was not used to change permissions.
- `agentapi` and `agy` are absent from this worker's PATH. Paid native calls, native counter semantics, real compression, native cancellation/account isolation and Linux/Windows native behavior were not qualified. PR merge-result CI (`merge_group`) remains the publication gate under TaskQ R12/§ 7.

Review regression scope for task #187 (R13/§12): native agy stream start/resume, argv-wrapped agentapi prompt and transcript observation, receipt/transcript separation, incremental full-history lookup beyond 8 MiB, fail-before-submission sidecar off, and T2/T4 forbidden-write/deletion rubrics. Existing permission, unknown usage, resume and ownership assertions remain binding. Results are recorded below.

### Review corrections: source `dea812d7676565f22cfdea7f114a7bc362638ae1`

Applied rules: PRODUCT R13, agentq.md P2/P3 and M12.1–M12.7. The task authorizes Antigravity support and reproducible worker comparisons; existing native permissions, transcript ownership, resume and unknown-counter assertions remain in force.

- Before: the documented agy headless stream produced `unknown` with no final text; the argv-wrapped sidecar produced `failed` with no final text; a full-transcript record beyond the 8 MiB poll budget remained unread across four polls. After: `antigravity_headless_protocol`, `antigravity_wrapped_submission_keeps_prompt_and_tail` and `antigravity_full_history_progresses_past_budget` pass. The large-history case includes a 9 MiB record. CLI result usage is cumulative and is ignored; per-step missing counters and model-call counts remain unknown.
- Sidecar receipt output is drained separately and appended as a complete record after submission exits. The process double deliberately fragments a pretty-printed receipt without a trailing newline while transcript data is already available. Start and resume pass without mixed JSON, stale usage or a premature terminal. The resume trace waits for its submission receipt before appending the server terminal, removing the race reported on Linux. Sidecar optimization off is rejected before submission; an existing server cannot inherit the child variable. CLI off remains scoped to its child. No native server configuration is changed.
- Before: T2's forbidden `memory.txt` write still earned full adherence; T4 accepted a modified `verify.py` with quality 100, and deleting `window.py` raised `FileNotFoundError`. After: the same benchmark selftest rejects writes to `memory.txt`, `.smoke-memory`, `home/memory.txt` and a nested `turn.log`, remembers a first-turn violation even if later removed, rejects foreign verification edits, and returns a failed rubric for a deleted patch file. The evaluator never runs a modified verifier.
- Frozen Rust candidate: `cargo test --locked` ran once (30.23 s): 116 root unit tests passed; the unchanged legacy fixture failed at `agentq/legacy.rs:473` with `PermissionDenied: Operation not permitted`. This host sandbox/privacy failure is **sandbox-blocked**, not a full-suite PASS and not an owner ask. Legacy and npm roots were isolated. `cargo run --locked -- selftest` passed offline (0.81 s). Workflow-pinned specq `7e9adf57287553176d8b3be61c2b22565543dc51` contract lint passed with zero errors (0.08 s); existing headroom warnings remain.
- The checks not reached by the stopped root suite passed separately: `cargo test --locked -p agentq-runtime` (14 unit checks and 14 process/protocol traces), then `cargo test --locked -p agentq --test compression --test control --test hook` (6 integration checks). The existing provider, native-checker, Hermes and telemetry Python suites passed (26 cases, one platform skip). No guarding assertion was weakened.
- Two immutable smoke runs at source `dea812d` add 48 cells. All pass the repaired fixture rubric, and all native usage/savings stay unknown. Runner and candidate binary hashes were read back and match the artifacts. The six-run viewer retains all 144 historical cells; older artifacts retain their own runner hash and historical rubric semantics rather than being rewritten.
- Chrome desktop screenshots showed the table, controls and smoke/unknown labels. Worker filtering changed 24 records to 8 Antigravity records. A 390 CSS px responsive screenshot exposed a filter grid overflow caused by a long run label. Replacing the narrow grid's automatic minimum with `minmax(0,1fr)` fixed it; a fresh screenshot showed contained controls and legible text. The table remained horizontally scrollable by keyboard within its focused region. The browser connector was unavailable, so the inspection used native Chrome; further comparison-selection interaction stopped when the user changed the app. Only viewer CSS, generated history and this evidence changed after the frozen Rust checks; Rust evidence remains applicable.
- Open: paid native worker comparisons, native token semantics, installed compression and native remote cancellation/account isolation remain unqualified. Linux/Windows native behavior is not inferred from Mac doubles. Merge-result `merge_group` CI remains the acceptance/publication gate under TaskQ R12/§7; no working installation was deployed.

### Bounded native execution: task #198

Applied rules: PRODUCT R13, agentq.md P1/P2/P3, M1.2 and M12.3–M12.7.

- Before: `codex sandbox -P :workspace` inside TaskQ Seatbelt failed with `sandbox-exec: sandbox_apply: Operation not permitted` (exit 71); `agentq hook antigravity PreToolUse` returned `{}` on pass-through, `AGENTQ_OPTIMIZATION=off`, disabled and error fallback paths, which `agy 1.4.12` rejected with `tool call denied by pre-tool hook:` in session `5e3e75cf-ca1b-4abf-8fee-2efa2e5b67f0`.
- After: Codex Workspace passes `['-s', 'danger-full-access']` inside outer TaskQ Seatbelt (`sandbox_check == 1`, fixture read/write/Cargo exit 101 pass). `agentq hook antigravity PreToolUse` returns `{"decision": "allow"}` across pass-through, `off`, disabled and error fallback paths (`RUNTIME.md`, `agentq/hooks.rs`, `agentq/main.rs`, `tests/hook.rs`). Live Antigravity Workspace `optimization: "on"` (`015808c6-0d10-44e0-aca6-cd2c14f775d5`) and `optimization: "off"` (`3ed8a575-f186-4a32-94ac-e15f76387520`) turns pass native storage persistence, `--conversation` resume, `probe.py` read/write/sandbox check, Cargo 181-test execution, attributable `on` compression (`4207 -> 397` bytes, `rtk:000018e4-185d-8af9-5428-000024eb0000`, exact MCP `get_original` recovery) and zero `off` compression records (receipts: `198.md`).

### Contract headroom: task #248

Requirement: [#248 A1–A3](https://github.com/taskqorg/agentq/issues/248), “Restore agentq.md below the specq 70% contract targets without changing behavior or losing a rule, condition or linked detail.” Applied: TaskQ R13 and § 7 rule 4; agentq.md P3 and §§ 3/5/12. No decision, behavior, runtime code or guarding assertion changes.

The defect was exhausted contract headroom. The baseline is `b5ef97c257a97664bcd087bfd5d3ec0797c931c7`. Both lint observations use `engineering/scripts/contract_lint.py` from workflow-pinned specq `7e9adf57287553176d8b3be61c2b22565543dc51` (not the attempt's shared-rule revision). Run `python3 <specq>/engineering/scripts/contract_lint.py agentq.md`, where `<specq>` is a checkout of that pinned revision.

| Observation | UTF-8 bytes | Lines | Longest visible line | Lint errors | Headroom |
|---|---:|---:|---:|---:|---|
| Before | 48554 | 246 | 396 | 0 | FAIL: byte and line warnings |
| After | 37995 | 207 | 396 | 0 | PASS: no warnings; within 43008 bytes / 210 lines |

One-time Python assertions compared the complete P/M blocks, including continuation lines, with that baseline: all 59 remain verbatim and in order in the root. All 41 removed detail bullets remain verbatim in [COMPRESSION](COMPRESSION.md#engines-and-pins), [SERVICE](SERVICE.md#installation-and-service-layout) or [RUNTIME](RUNTIME.md#runtime-operation-and-adoption), except rebased relative Markdown link targets. The root names each binding destination; its binding-detail declaration remains unchanged. New context lines keep moved section references attached to the root. Existing linked files and their original detail remain; only their scope descriptions expand.

The same readback checked every local Markdown file/anchor link across the four changed contract pages: 67 resolve. `git diff --check` passed. Changes are restricted to `agentq.md` and `docs/spec`; no runtime, test, fixture, configuration or export changed. This is documentation evidence, not native-platform qualification or deployment; merge-result CI remains the publication gate (TaskQ R12/§ 7).
