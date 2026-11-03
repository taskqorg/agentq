# Agent runtime

This page holds the binding detail of [agentq.md § 12](../../agentq.md#12-agent-runtime). The library moved unchanged from CoreQ v0.18.0 (`src/agent.rs`, `src/agent/`, `tests/agent.rs` and `docs/design/agent.md`), where CoreQ R47-R51 stated its rules; those rules hold here unchanged. Normal CLI behavior is TaskQ's CLI adapter (`src/runtime.rs` and `src/runtime/accounts.rs` at TaskQ v4.6.57), moved unchanged; compatibility APIs preserve its background and app boundaries.

## Runtime trait

M12.9/M12.10 apply the [specq agent-runtime rule](https://github.com/taskqorg/specq/blob/main/engineering/agent-runtimes.md). The public surface is `agent::Runtime`, `agent::Descriptor`, `agent::Policy`, `agent::Launch`, `agent::Run` and the runtime-neutral `Report`, `Usage`, `Snapshot`, `Accounts`, `Context` and `history` types. `agent::runtimes()` lists every adapter; `agent::runtime(name)` finds one by descriptor name (alias `agy` selects `antigravity`). Callers treat names and session IDs as opaque data.

Each adapter implements every operation. The sections below remain the binding detail of what each adapter does natively.

| Operation | Contract in this runtime |
|---|---|
| `launch(&Launch)` | Check `quota` when `Launch.accounts` is set (none free: error `quota-exhausted`), map `Policy` (refusal names the part), apply optimization intent, start one turn and return a `Run`. |
| `resume(session, &Launch)` | The same for an existing opaque session ID. |
| `input(run, text)` | Send more user text to a running session. A one-process CLI turn returns cause `input-unsupported`; the next turn is a `resume`. |
| `cancel(run)` | Request the stop; returns whether exit is already confirmed. `poll` confirms it later. It never proves server-side stop (see Arguments). |
| `poll(run)` | `running`, `finished`, `failed` or `unknown` from the existing end detection. |
| `report(run)` | The `Report` of a finished run; a running one returns cause `running`. |
| `progress(run)` | Complete native JSON records appended to the run's log since the previous call, as raw values, each read at most once. |
| `session_id(run)` | The native session once named, else none. |
| `session_url(run)` | Claude: the native Remote Control `https://claude.ai/code/session_*` URL when one was observed, else cause `not-observed`. Codex and Antigravity: cause `no-session-url`. No session yet: cause `session-unknown`. |
| `transcript(run)` | The caller log the run appends to. |
| `ready(home)` | `install` (CLI resolves), `version` (`--version` within 5 s) and `login` (native credential file in the home); each `ok`, `missing` or `unknown` with a reason. Absent credential files read `unknown`, because keychains also hold logins. |
| `quota(&Accounts, now)` | The account a launch takes, whether every account is disabled, and the hold end when every enabled account is limited. |
| `running()` | Runs of this adapter in this process that have not reached a terminal poll and were not dropped. |
| `protected_paths(home)` | Native home folders and files a run must not overwrite: Claude home and `.claude.json`, Codex home, Antigravity home and its sibling `config`. |
| `home_env(tool, label)` | The home variable pointing at `<base home>/<tool>-accounts/<label>`; none for `default`, which keeps the caller's own home. |

Descriptor data, the only runtime metadata a caller reads:

| `name` | `label` | `brand` | `family` | `capabilities` |
|---|---|---|---|---|
| `claude` | Claude Code | Anthropic | `claude` | `turn`, `resume`, `caller-session-id`, `compact-hook`, `session-url`, `quota-snapshot` |
| `codex` | Codex CLI | OpenAI | `gpt` | `turn`, `resume`, `quota-snapshot` |
| `antigravity` | Antigravity | Google | `gemini` | `turn`, `resume` |

### Policy map

`Policy` has `read` and `write` scope (`native`, `none`, `project`, `any`), `network` (`native`, `on`, `off`), `tools` (`native` or an allowlist) and `approval` (`native`, `plan`, `edits`, `auto`, `bypass`). `native` keeps the runtime's own default. Every value without a row below refuses launch with `policy <part> unsupported by <name>`.

| Part | Claude | Codex | Antigravity |
|---|---|---|---|
| `read` | `native` | `native`, `any` | `native` |
| `write` | `native` | `none` → `--sandbox read-only`, `project` → `--sandbox workspace-write`, `any` → `--sandbox danger-full-access` | `native` |
| `network` | `native` | `on` with `project` → `-c sandbox_workspace_write.network_access=true`; `off` with `none`/`project`; `native` | `native` |
| `tools` | `native`; allowlist → `--allowedTools <tool>..` | `native` | `native` |
| `approval` | `plan`, `edits` (`acceptEdits`), `auto`, `bypass` (`bypassPermissions`) → `--permission-mode <mode>` | `bypass` with `write` `any` → `--dangerously-bypass-approvals-and-sandbox` instead of `--sandbox` | `bypass` → `--dangerously-skip-permissions` |

`Launch.model` maps to Claude `--model <m>`, Codex `--model <m>` and Antigravity `--model=<m>`. `Launch.compact_hook` maps to Claude `--settings` with a `SessionStart` `compact` hook; other runtimes lack capability `compact-hook` and refuse it. `Launch.wrapper` is a caller sandbox command placed before the native executable. The adapter places these mapped arguments where caller `<args>` stood in the Arguments table, so the native argument order is unchanged.

## Session model

- One native session belongs to one conversation or one task attempt.
- A new start passes a caller-chosen session ID to Claude (`agent::new_session` gives a UUID v4). Codex names its thread in the first `thread.started` event and Antigravity names its conversation in `conversation.started` (or `response.newConversation.conversationId`); `new_session` gives `-` until then.
- Each turn is one client process. The process reads the turn text from stdin (or for native `agentapi`, receives the prompt argument and tails `<home>/brain/<session>/.system_generated/logs/transcript.jsonl`) and exits when the turn ends. The next turn resumes the same session ID.
- The client compacts its own context. For Claude, the caller passes `agent::claude_settings(Some(hook), env)` in `--settings`: a `SessionStart` hook on `compact` runs the caller's command, which prints the rules again. The settings `env` survives resume.
- Output of every turn appends to one caller log per session. A turn reads only the bytes it appended.

## Arguments

The caller builds the base command: the client executable from `agent::executable`, or a sandbox wrapper around it. `agent::command` appends the arguments, sets the project root as working directory, removes `CLAUDE_CODE_SESSION_ID`, `CODEX_THREAD_ID` and `ANTIGRAVITY_CONVERSATION_ID`, adds the caller's environment and sets the account home.

| Client | Start | Resume |
|---|---|---|
| Claude | `-p --output-format stream-json --verbose <args> --session-id <id>` | `-p --output-format stream-json --verbose <args> --resume <id>` |
| Codex | `exec --json <args> -C <root> -` | `exec --json <args> resume <thread> -` |
| Antigravity CLI (`agy`) | `--input-format stream-json --output-format stream-json <args>` | `--input-format stream-json --output-format stream-json <args> --conversation <session>` |
| Antigravity sidecar (`agentapi`) | `new-conversation <args> <prompt>` | `send-message <args> <session> <prompt>` |

- `<args>` are the caller's client arguments in this position: TaskQ passes `--permission-mode <mode> --settings <json>` to Claude, sandbox arguments to Codex, and optional `--model=<flash_lite|flash|pro>` / `--profile=<profile>` / `--title=<title>` flags to Antigravity.
- `agent::which` resolves a name on `PATH`; on Windows it also tries `<name>.exe` and the npm shim `<name>.cmd`. `agent::user_bin_path` appends `~/.local/bin` when `PATH` lacks it.
- `agent::codex_native` replaces an npm `codex.js` launcher with its native executor on macOS and Linux, so a kill reaches the client. It never scans other installations.
- For `Client::Antigravity` (`"antigravity"`, alias `"agy"`), `agent::executable` resolves `ANTIGRAVITY_AGENTAPI_EXE` when set, else `agy` on `PATH`, else `agentapi` on `PATH`, else `<home>/bin/agentapi`. A caller-selected `command` remains authoritative. The default is `agy` when available; explicit `ANTIGRAVITY_AGENTAPI_EXE` remains authoritative. `agy` and its wrappers receive one JSON `user.message.content` on stdin. Native `agentapi` (including an executable argument inside a sandbox wrapper) takes the prompt as its final argument. Its stdout receipt and stderr are drained before transcript records append, so partial receipt writes cannot corrupt events. The caller supplies the existing server address, credentials, project and account environment; the runtime never discovers another running server or changes its configuration. [Native command reference](https://antigravity.google/docs/sidecars).
- Native `agentapi` submission exit is not turn completion. The runtime tails complete new records from `<home>/brain/<session>/.system_generated/logs/transcript.jsonl`; a resumed turn captures its boundary before submission. Partial records wait, malformed records, file replacement, truncation and changed consumed prefixes fail observation. Each poll reads at most 8 MiB of new bytes from each transcript; partial records and full-history lookup retain progress across polls, including histories or records larger than that budget. Truncated records wait for the corresponding complete `transcript_full.jsonl` step. Session IDs used as locators contain only ASCII letters, digits and hyphens.
- `agy` stream `init` identifies the session, `step_update` supplies per-step usage, and terminal `result` supplies status and final text (`SUCCESS`, `ERROR`, `CANCELED`, `INTERRUPTED`, `INVALID`). `WAITING`, `RUNNING` and unrecognized statuses never prove completion. [Headless protocol](https://antigravity.google/docs/cli/headless). Result usage is cumulative and is never summed as per-turn usage. Missing counters and model remain unknown.
- An existing `agentapi` server does not inherit submission environment. A launch with optimization `off` (explicit or inherited) fails before submission rather than reporting suppression it cannot deliver; use the owned `agy` CLI for off/on comparisons. No server configuration is edited.
- `kill` and a deadline stop the owned submission process and observation. They do not prove that server-side execution stopped; callers retain native cancellation and capacity ownership until their own readback. A submission without a session or terminal remains unknown. Native platform/account qualification remains open until a real caller-owned turn is observed.
- Optimization attachment reads enabled matching hooks in the selected home/global config and the caller project's `.agents/hooks.json`; no file is edited.
- Antigravity hooks use `toolCall.args.CommandLine`, `conversationId` and `stepIdx`; the response shallow-merges `overwrite.CommandLine` when wrapping a qualified simple command, and always delegates to native permissions with `decision: "allow"` (never `auto_approve`), including pass-through, `off`, disabled and error fallback paths so `PreToolUse` never denies native execution (P1, M1.2, M12.6, M12.7). Shell composition and Windows command shells remain native. [Hook schema](https://antigravity.google/docs/hooks).


## End of a turn

- A turn ends at a Claude `result` event, a Codex `turn.completed` or `turn.failed` event, or an Antigravity `PLANNER_RESPONSE` step with `status: "DONE"` and no `tool_calls` (or `status: "ERROR"` / `"CANCELLED"`).
- A process that is still running after its turn-end event is stopped: Codex on Windows kept running after its turn ended.
- `Turn::wait(deadline)` stops the process at the deadline; the turn then reads `cancelled` with error `timeout`.
- `Turn::poll` serves a caller loop that does not block.

## Outcome

`Turn::report` classifies the turn in this order; `Report::read` alone has no deadline or exit code:

| Condition | Outcome | Error |
|---|---|---|
| Deadline passed | `cancelled` | `timeout` |
| Limit reached before work started | `refused` | `usage-limit` |
| Codex login rejected (refresh token or 401) | `failed` | `auth-rejected` |
| Codex `turn.failed` naming model capacity | `failed` | `model-capacity` |
| Codex `turn.failed` or Antigravity `PLANNER_RESPONSE` `status: "ERROR"` | `failed` | `turn-failed` |
| Claude `result` with `is_error: true` | `failed` | `client-error` |
| Turn-end event | `ok` | `none` |
| No turn-end event, nonzero exit | `failed` | `exit` |
| No turn-end event, exit 0 | `unknown` | `no-end` |

A reached limit returns `Report::reached`: the reported reset in unix seconds, or none. Claude reports it on the first `assistant` event (`apiError: usage_limit_reached` or `quotaLimits.status: rejected`, reset `quotaLimits.resetsAt` or `apiErrorParams.rate_limit_info.resetsAt`). Codex and Antigravity report it as `error` or `turn.failed` / step error text containing `usage limit` or `quota exceeded`, without a reset. A limit after work started is not a start refusal.

## Usage

- Claude: each `assistant` message ID counts once, because one message spans several lines. Uncached input is `input_tokens`, cache read `cache_read_input_tokens`, cache write `cache_creation_input_tokens`, output `output_tokens`. The model is the one of most calls.
- Codex: each `turn.completed` counts. Its `input_tokens` include `cached_input_tokens`, so uncached input is the difference. Codex reports no cache writes (0), no call count and no model in `exec --json` output.
- Antigravity CLI: per-step `usage` of completed stream `step_update` records counts once by `step_index`; terminal result counters are cumulative and are ignored. No model-call count is inferred. Antigravity sidecar: each completed `PLANNER_RESPONSE` step counts once, using the latest record for its `step_index`. Uncached input is `input_tokens`, cache read `cache_read_tokens`, cache write `cache_write_tokens`, output `output_tokens`, and model is `model`. Missing counters stay unknown; no usage or model is inferred from the requested model. Native counter formats need platform qualification.
- A counted call without a value makes that sum unknown, never smaller. No counted call: every count is null with reason `no-usage`.
- A model ID is a code of at most 64 characters from `[A-Za-z0-9._:-]`; any other value is not reported.

## Limit snapshot

| Client | Source | Used percent | Reset | Window |
|---|---|---|---|---|
| Claude | last `rate_limit_event` of the turn | `rate_limit_info.utilization` (0-1) × 100 | `rate_limit_info.resetsAt` | `rate_limit_info.rateLimitType` (`five_hour`, `seven_day`) |
| Codex | last `rate_limits` of the turn output, else the last `token_count` in the thread rollout `<home>/sessions/<yyyy>/<mm>/<dd>/rollout-*-<thread>.jsonl` | `primary`/`secondary` `used_percent` (0-100) of the window with higher use | that window's `resets_at` (or now + `resets_in_seconds`) | that window's `window_minutes` as `<n>m` |

- Claude also sends `unifiedWindows` with each window; the top-level window is the binding one, so only it is recorded.
- Codex `exec --json` output carries no limit snapshot today; the rollout of the account home does. A rollout snapshot is the latest one of the thread, which can be from an earlier turn.
- Percent is rounded to a whole number.
- Reasons: `not-reported` (no snapshot), `no-rollout` (Codex rollout not found), `partial-snapshot` (percent or reset missing).

## Observation

`agent::run` appends one observation per turn; `Report::observation` builds it for a caller that runs turns itself.

- Keys: `product`, `version`, `operation` and `kind` = `agent.turn`, `started_ms`, `duration_us`, `outcome`, `error`, `project`, `runtime`, `account`, `task`, `attempt`, `route`, `model`, `input_tokens`, `cache_read_tokens`, `cache_write_tokens`, `output_tokens`, `limit_used_percent`, `limit_resets_at`, `limit_window`, `unknown`.
- `trace` and `span` are null with reason `not-traced`. A task turn has `route` null with reason `task-turn`; a route turn has `task` and `attempt` null with reason `no-task`.
- A failed append is kept in `Report::recorded`; it never changes the turn's result.

## Accounts

- `Accounts { list, limits }` holds one client's ordered accounts and their recorded limits. The caller stores both.
- An empty list is one enabled `default` account. A label is 1-32 of `[a-z0-9-]`, not starting with `-`.
- A limit ends at its reported reset, else `PROBE` (1800 s) after it was seen.
- `pick`: the first enabled account whose limit has ended, else the first enabled account; none when all are disabled. `spare`: the account a limit moves to, only with more than one account. `hold`: the earliest live limit end of an enabled account and whether it was reported.
- Homes: `default` keeps the caller's own home (`CLAUDE_CONFIG_DIR` / `CODEX_HOME` / `ANTIGRAVITY_HOME`, else `~/.claude` / `~/.codex` / `~/.gemini/antigravity`; a Windows service uses `USERPROFILE`). Any other account is `<home>/<tool>-accounts/<label>`, with `<tool>` the consumer's `Tool.name`. `command` sets the home variable for the child.

## Compatibility sessions

Native CLI turns keep the v0.17 arguments and behavior inside the adapters. Background, desktop queue, app-server and protected modes are adapter internals selected by `Launch.mode`; `Turn`, `NativeTurn`, `AppSession` and the native parsers are private (M12.9). These modes never switch Claude to `-p` or replace an app session with `exec`.

- `background_command` passes `--bg <caller args>` on start and only `--bg --resume <session>` on resume. Start arguments include the caller's settings, name, Remote Control name and permission mode in their original order. The native client assigns the start's session and job IDs. Input is the caller's file reference, not private task text: Claude forwards it to child argv. The caller durably binds and retains that input before submission.
- `background_launch` uses bounded stdin exchange and returns launcher output or failure without interpreting either as completed work. A timeout, lost reply or malformed reply is uncertain; retain the original input and recover without resubmitting. `background_jobs` uses `agents --json --all --cwd <root>` in the same account. `BackgroundJob::read` verifies exact canonical root, job and session; readiness means a known native state, not accepted input or completion. Remote Control identity comes only from that session's native `bridge_status` URL.
- `app_queue` submits the caller's file reference with `queue --thread <session> --message <reference>` in the same account. New desktop sessions use the existing normal CLI handshake to obtain a thread before queueing; handshake observations belong to that turn, never to the queued turn. The caller opens/focuses the app and verifies its presence. A failed queue response is uncertain and must not cause another queue call.
- `NativeTurn` observes a caller-selected transcript from a byte boundary captured before submission. Claude requires its exact user input then `system/turn_duration`; Codex desktop requires session metadata for the exact root/thread, `task_started`, the exact user input and matching `task_complete` or `turn_aborted`. Duplicate matching inputs, conflicting terminal results, foreign identities, truncation, replacement, invalid or partial records stay unknown. Each bounded poll continues reading without resubmission.
- `NativeTurn::checkpoint` and `recover` preserve the original input boundary, account, session, timing and observation ID. They contain private locators and input; callers protect and durably store them outside telemetry before submission. A new background launch uses a caller alias and an absent transcript locator; `attach_background` binds the verified job and its native transcript after discovery, retaining the original input. Recovery validates file identity and the read prefix and rebuilds observation only; it grants no stop authority. Each poll reads at most 8 MiB of new records, each record at most 8 MiB; prefix verification has a 5-second bound and failure retains ownership. `record` commits the terminal observation using the saved ID so an acknowledgment lost across restart cannot duplicate it.
- `app_server_command` builds `app-server <caller args>` with the shared environment/account handling; the caller owns transport lifetime. `AppSession` supplies initialize/initialized, thread start/resume, turn start, interrupt and raw notification observation. All JSON parameters, including instructions, tools, model, effort, provider and grants, are caller inputs and are preserved. A server request remains with the caller and is never auto-approved. A missing thread cannot silently start another one. A request-correlated native turn ID is required before a matching `turn/completed` ends the turn without ending the server; process/transport exit alone remains unknown. Before that reply, up to 256 notifications / 8 MiB can be retained for correlation; exceeding either bound preserves the uncertain original submission.
- `AppServer` owns the shared stdio transport: start, bounded send/receive, process polling and explicit direct-child stop. Each record is at most 8 MiB; a 16-record queue applies backpressure without dropping events. A timed-out write is uncertain and never retried. Stderr goes only to the caller's private log. Native notifications and server requests reach the caller unchanged, including dynamic-tool calls and private `item/agentMessage/delta` drafts. `AppSession::request` also carries native `model/list` and `account/rateLimits/read` requests; callers select models, page/filter catalogue results and handle tools without a second transport or an MCP replacement. No terminal event automatically kills the server. Callers retain their existing process-group supervision and startup serialization.
- Stop requests do not prove exit or completion. `background_stop` targets only a verified job and returns the request response; the caller reads jobs and its owned process identity back before releasing capacity. App-server interrupt targets the exact accepted thread/turn. Desktop native cancellation remains a caller-owned UI/control operation using `NativeTurn::turn_id`; the runtime observes its matching abort. No process tree or grant is inferred from these identities.

Compatibility reports reuse `Report`, `Usage`, `Snapshot`, `Context` and `agent.turn`. Claude usage counts assistant IDs within the matched input's turn. Codex usage adds attributable model-call increments (`last_token_usage` / `last`) once per distinct cumulative snapshot (`total_token_usage` / `total`); it never reports the thread total as turn usage. Missing counters or inconsistent increments make the affected sums unknown. App-server usage/limits normalize native field names; absent values remain null with reasons. Native formats without attributable usage return `no-usage`; unreported duration is null with `not-reported` in the observation. Readiness and launcher/queue responses emit no completed-turn observation. Protocol references: [Codex app-server](https://learn.chatgpt.com/docs/app-server), [OpenAI token accounting](https://github.com/openai/symphony/blob/main/elixir/docs/token_accounting.md).

Fake clients verify these boundaries on Linux, macOS and Windows. Native consumer qualification and UI behavior remain with each consumer (TaskQ #2094, PilotQ #265).

## Protected app-server

M12.8 adds `agent::ProtectedHost`, `ProtectedIdentity` and `ProtectedAppServer` alongside the unchanged `AppServer` API. There is no TaskQ Rust dependency or mandatory service for ordinary turns. The optional TaskQ adapter uses the existing operator grant resolver, protected process boundary and external slot protocol; the runtime owns neither grants nor slot admission/release.

The caller selects an immutable, reviewed host executable by canonical path and SHA-256, outside task/environment/receipt inputs. `ProtectedHost::new` checks that artifact (the constructor names no product; the host wire schema stays `taskq.app-server/1`) before every invocation. This is a trust anchor supplied by the operator, not proof that arbitrary caller-selected code is a host authority. It does not defend against an unrestricted same-user operator replacing the trusted executable between verification and execution.

For adapters predating required preparation, `launch` accepts the existing `taskq.app-server/1` request and a `ProtectedIdentity` match requirement: canonical account home and native executable/digest, generated profile digest and grant approver. The request binds canonical project, Codex runtime, account, policy revision and the existing slot's caller/operation/process identity. It accepts no native arguments, environment, model, executable override or serialized grant. Missing, extra or malformed request fields refuse before spawn; the host validates the protected grant and slot independently.

Before calling `launch`, the caller durably retains that exact operation/request and its private slot key. A launch attempt is never automatically retried, even on an error. The returned handle initially has unknown authority. `verify` obtains a separate, bounded trusted readback and checks requested, saved and verified-effective facts, the expected identity, exact supervisor/native creation identities and host generation. Every `send` repeats this check; no handshake, turn or tool response is sent before it passes. The host additionally revalidates authority under its own lock at actual native input delivery. A receipt returned to the caller is private evidence, never a bearer grant or an input that opens the gate.

In `taskq.app-server/1` readback, `launch.authority.resolved` carries the host resolver's `runtime`, `requested`, `grant`, `effective`, `enforcement` and `refused` fields. The resolved Workspace check requires `resolved.effective == "workspace"`, alongside all existing saved-grant, verified-effective and identity checks. Missing or null `effective`, Full or another value, and a mode-only object refuse before native input; `resolved.mode` is not an alias. This wire check neither adds a TaskQ dependency nor changes the protected API. Diagnostics expose fixed field/category names only, never raw receipt values, paths or keys.

Unknown, failed or mismatched readback refuses sending and retains the original binding; `verify` may reconcile that same binding without resubmission. A fully validated readback with a changed binding (authority, native/supervisor incarnation, profile or host generation), or an uncertain write, permanently fences the handle. `ProtectedHost::observe` and `cancel` support recovery of the original operation without launching again. Cancellation uses the host's exact-child authority and never releases a slot or proves descendants/turn completion. Dropping a handle, EOF, a timeout or supervisor exit leaves work uncertain; callers retain ownership and reconcile before the original whole-turn slot release.

Native stdio uses the existing bounded transport. `AppSession` still owns thread/turn correlation, interruption, checkpoints and its one idempotent `agent.turn` observation. Server requests and native account/model/permission parameters remain caller-owned. The adapter preserves the host's model/account and approved outer boundary; caller protocol parameters cannot widen it. Optimization intent travels only as child `AGENTQ_OPTIMIZATION`; the host prepares native `off` env/header and launcher marking. `optimization` reports configuration, never actual compression or enforcement.

The supported host protocol/source pin and consumer sequence are in [the protected consumer recipe](../evidence/runtime-186.md); [receipt wire correction evidence](../evidence/runtime-193.md) supplies the corrected runtime revision. Its current adapter supports existing explicit Mac Codex Workspace grants only; there is no Full fallback, new grant or Windows containment enablement. Portable fake-adapter checks prove the runtime gate and compatibility, not Mac containment. Live Mac image/sandbox/stdio, filesystem/network/child and outside write/delete/rename/control-store probes remain a separate native qualification gate. Do not replay the retained AssetQ crate.

### Prepared operation and completion

`ProtectedHost::prepare(slot, deadline)` calls only the checked adapter's non-launching `prepare` action. It authenticates the existing immutable slot and actual caller PID/creation/boot through the host, returning `ProtectedPreparation`: independent project, revision/account, approver, native image/hash/home, profile and adapter authority. The host owns resolution and admission; AgentQ adds neither resolver nor scheduler. Preparation is an equality requirement, never a grant. The caller durably retains `checkpoint()` and the original private slot before any launch. `recover_preparation` validates that private checkpoint without dispatch; an operation receipt cannot replace it.

`launch_prepared` reuses the original slot, preparation digest, revision and account. It validates the caller and artifacts before spawning the adapter; the host revalidates saved preparation and authority before intent and native input. The checked readback binds the frontend transport incarnation separately from the launchd supervisor and native incarnation, plus the prepared host generation and exact authority. A prepared transport never falls back to legacy launch. The original `launch` API stays available for previously qualified adapters; it cannot synthesize preparation for a host that requires it.

`ProtectedHost::reconcile(preparation, deadline)` invokes only the same operation's bounded host reconciliation. A `completed` result requires its saved authority and host generation, sealed native/supervisor/scope certificate, same-boot singleton counters and acknowledged exact-generation cleanup. Unknown, malformed, missing or mismatched evidence never proves completion. No live-root absence, EOF, terminal model event, cancellation or frontend/supervisor exit substitutes for that certificate. Recovery does not require the original caller to remain alive and never launches or resends input. Interrupted cleanup stays unknown unless the original host can verify its durable acknowledgment.

Completion covers the owned native scope and inherited descendants only. AssetQ still owns orchestration, delegated/remote effects, current-image-tool discovery and output acceptance. It releases only the original immutable slot after its entire operation ends and reads back the existing service acknowledgment; AgentQ never releases it. Duplicate/reordered observations cannot authorize replay. An uncertain readback retains occupancy and the original checkpoint/key.

The [owner-route handoff](../evidence/runtime-199/OWNER-ROUTE.md) supplies the checked caller module, exact prepare/launch request shapes and one-shot private checkpoint writer. It uses the same APIs, preserves `off`, consumes dispatch intent before launch and contains no admission, release, retry loop or worker-environment override. Root/PM uses it only after reviewed source publication under existing authority; it creates no native qualification claim or live checkpoint in a worker.

The additive TaskQ prerequisite and launchd optimization-intent correction are published at `5292e63ffaeda41ca90d1b2e21b93f78fe2d3e39` ([#2151](https://github.com/taskqorg/taskq/issues/2151), [#2161](https://github.com/taskqorg/taskq/issues/2161)). The supervisor job carries explicit/inherited `AGENTQ_OPTIMIZATION=off` in its hashed environment; publication does not prove the installed host source or native behavior. Publication of AgentQ requires reviewed immutable source and green merge-result CI; PM then qualifies only the new preparation/completion/recovery paths under existing Mac Workspace authority. Reuse AssetQ `05476e88c53f5232cc17f37bd664aad0ba103a79` composed PASS. No model/paid turn, crate replay, new grant, Full fallback, ordinary install, account/model/capacity change, `TASKQ_ATTEMPT` stripping or slot-CLI refusal bypass is authorized. Portable checks alone cannot close this native gate. Recipe and evidence: [task 199](../evidence/runtime-199.md).

## Worker capabilities and bounded history

`agent::history` is the shared read-only observation API for TaskQ and other callers.
It does not submit input, change sessions, grant authority or migrate live workers.
Existing `Turn`, `NativeTurn` and `AppSession` remain the execution and receipt APIs.

### Required functions and ownership

| Function | Runtime boundary and receipt |
|---|---|
| Launch, resume, session ID | `command`/`Turn`; native session receipt, never a generated ID treated as acceptance |
| Input, files, images | Caller retains input and file/image references; CLI passes caller arguments; app-server passes typed input unchanged |
| Ask, reply, result | Native tool/request records are observations; app-server caller replies with original request ID; CLI has no added interactive reply channel |
| Tools and outputs | History blocks retain native calls/results and IDs; dynamic tools remain app-server caller-owned |
| Heartbeat | History progress is source progress only; `poll` process readiness is not useful work or completion |
| Cancel and recovery | Existing ownership/checkpoint APIs apply; sidecar submission kill never proves server cancellation |
| Model, effort, accounts, permissions | Caller arguments/environment/account home or app-server parameters; never inferred from requested values |
| Usage and errors | Existing `usage`/`Report` parsers; missing counters and malformed observation stay unknown |

A history cursor is not an acceptance, completion or cleanup receipt (M12.3/M12.8).
Recovery reopens the original source and validates identity and consumed bytes; it never resubmits.
The caller protects logs, cursors, raw content and attachment references outside telemetry.
No history content is exported as `agent.turn` metadata.

### Capability matrix

S = supported by adapter tests, L = limited with the reason below, U = unsupported,
T = native behavior untested. Adapter support and native qualification are separate axes.
`history::capabilities` returns the selected format, caller-observed native version and OS,
feature status/reason, and evidence. An absent version or an unqualified tuple stays T.
Supplying a version is observation input, never a native probe or a qualification claim.

| Runtime / transport | Required functions | History | Native version / OS evidence |
|---|---|---|---|
| Codex exec JSON | S launch/resume/session/args/cancel/report; L interactive replies/media | S items, tools, lifecycle; L user input absent from exec output | T version/OS; portable fake clients only for this API |
| Codex desktop rollout | S `NativeTurn` observation/recovery; L caller-owned app submission | S response/event records; L evolving native schema | T version/OS for history; existing runtime evidence remains separate |
| Codex app-server | S caller-owned requests/replies/tools/input/cancel/recovery; L protected host authority | S caller-recorded JSONL notifications; L unrecognized notifications raw-only | `codex-cli 0.159.3` / macOS arm64: initialize/thread-start capture only; model turns T; protected qualification remains M12.8 |
| Claude stream JSON | S launch/resume/session/args/cancel/report; L interactive replies | S message blocks, tools, result | `Claude Code 2.1.316` / macOS arm64: initialize control capture only; model turns T |
| Claude background/Remote Control | S existing launch/job/native receipt/recovery; L caller stop authority | S native message JSONL; L server state and media not observed unless emitted | T version/OS for history |
| Antigravity agy stream | S launch/resume/session/report; L caller args, media and replies | L text deltas/tool output/result; DONE may have no text | agy 1.4.12 / macOS arm64: existing #198 execution/resume evidence only; new history API T |
| Antigravity agentapi sidecar | S submission/session/tailing; L cancel/recovery and native controls | L USER_INPUT/PLANNER_RESPONSE/SYSTEM_MESSAGE/tool calls/results; GENERIC author unknown, truncated fields and raw-only attachments | agy 1.4.12 reported by #198 / macOS arm64: original sidecar fixture parsing only; transcript embeds no version; native history API T |
| Any other runtime/transport/version/OS | U transport not enumerated; T unqualified version/OS | U unknown schema normalization; raw retained | No inferred parity |

All enumerated formats run offline contract checks on the host; CI supplies Linux/macOS/Windows adapter coverage.
Other OSes remain untested. Headless Antigravity streams do not expose every persisted step field.
Use a caller-selected sidecar transcript for full content; never locate another account or substitute a different session.
Media support means retaining native references/blocks, not downloading, decoding or inventing captions.
Permissions, effort/account catalogue and interactive replies unsupported by a transport remain native caller controls.

### History wire contract

`History::new(format, source, session)` binds an immutable caller source label and session.
`poll(path)` returns a page of at most 128 records and 16 MiB of source records,
reading at most 16 MiB plus one lookahead byte (excluding prefix verification).
Each UTF-8 JSONL record, including its newline, is at most 16 MiB; larger records
return `history record exceeds 16 MiB` without advancing.
The caller polls at its own bounded cadence while `more` is true,
including partial records, and resumes observation when the source grows after EOF.
A partial final record waits without advancing; malformed JSON, oversized records,
replacement, truncation and consumed-prefix changes return an error without advancing the cursor.
Prefix verification reuses the existing five-second native guard.
A checkpoint records format/source/session, file identity, byte boundary and prefix digest.
`recover` validates the checkpoint and the next poll validates the original file. No process starts.

`poll_before(path)` pages backward from the checkpoint's byte boundary (exclusive).
On a fresh, unbound cursor it starts at EOF. Each page retains ascending physical order,
the same event IDs and the same 128-record / 16 MiB bounds as forward polling.
The cursor moves to the first returned record, and `more` means older bytes remain;
at byte zero, repeated backward polls stay empty. A trailing incomplete record is
excluded and reported in `pending_bytes`; a fresh forward observer can read it once complete.
Backward reads use at most 16 MiB plus one boundary byte, excluding prefix verification.
Save/recover the checkpoint between pages to retain source/session/file binding.
Appending records does not move an existing backward cursor. Replacement, truncation,
prefix changes, invalid cursor boundaries and malformed complete records fail without
advancing. Callers keep separate cursors for incremental forward and backward browsing.

```rust,ignore
let mut older = History::new("codex-rollout", "worker-log", native_session)?;
let newest_page = older.poll_before(&caller_selected_path)?;
let saved = older.checkpoint();
let mut older = History::recover(&saved)?;
let preceding_page = older.poll_before(&caller_selected_path)?;
```

Each event includes source label, exact byte range, raw UTF-8 record (newline included),
source SHA-256, source timestamp (`timestamp`, `time`, sidecar `created_at` or app-server `emittedAtMs`) when present (otherwise null), author (otherwise `unknown`),
native item/step ID when present, physical byte order, normalized blocks and lifecycle kind.
An emitted native session ID must match the bound session; otherwise the page fails without advancing.
`session_binding` is `native` only on a record carrying that ID; otherwise it is `caller`, never identity proof.
`pending_bytes` names unconsumed bytes in a forward read, or excluded incomplete tail bytes
in a backward read; an empty page alone is not a health claim.
Event IDs bind source/session/offset/raw hash; repeated reads of the same record have the same ID.
Physical duplicate records remain visible in source order; native item IDs link updates without merging deltas or discarding repeated inputs.
The API never sorts by timestamps or treats observer time as native time.
Blocks use `text`, `image`, `attachment`, `tool_call`, `tool_result` or `unknown` types.
Each typed block retains its native JSON; text and tool fields are projected without rewriting strings.
Claude result records expose their final text alongside the native lifecycle envelope.
Claude stream control request/response records are lifecycle observations, correlated by
their native request ID; they add no reply channel. Codex rollout `UserMessage`
records retain user authorship and validate the enclosing native `thread_id`. App-server `thread/started` is a
lifecycle observation, not a turn acceptance receipt.
Codex tool blocks correlate by `call_id` when present; `item_id` retains the separate native item identity.
Unknown message/media blocks set `unsupported-content-block`; unknown agy step types retain
unknown author/kind and `unsupported-source-shape`. Only known system/checkpoint step types are lifecycle.
The agy history capability is limited because completed steps can omit text; sidecar history
is limited by truncated fields, unknown `GENERIC` authorship and raw-only attachments.
Sidecar untyped `tool_calls` with string `name` and `args` project as calls, retaining
absent IDs as null; future typed shapes stay unknown. The sidecar `tools_outputs`
capability is limited: `GENERIC` output has no known tool/actor/call ID, and the
`TOOL_RESPONSE` envelope is covered by schema fixtures only.
`SYSTEM_MESSAGE` exposes its text as system lifecycle; `GENERIC` exposes native text
with unknown author/kind and `generic-author-unknown`, unless truncation or malformed
truncation already requires a limit. No tool result or actor is inferred from generic prose.
Capability readback reports those limits.
Markdown and code text retain exact characters; image/attachment blocks retain native references without fetching content.
Unknown block/schema content remains in raw source with an explicit limitation, never guessed as text.
Antigravity `truncated_fields` absent, null or an empty array means no omitted fields;
a nonempty array marks `truncated-source`, and any other shape marks
`unsupported-truncation-field`. Truncated content is not presented as complete;
callers can select the full source.
History lifecycle events are descriptive only; existing terminal/identity parsers remain the sole turn-completion authority.

## JSON command

`agentq agent turn` reads one JSON request on stdin, runs the turn with `agent::run` (or `Turn` without a store) and prints one JSON report. It uses the same `Spec`, `Report` and optimization code as the library. A request error prints `Error:`/`Next:` and exits 1; a failed turn is a report and exits 0.

| Request field | Meaning |
|---|---|
| `client` | `claude`, `codex` or `antigravity` (`agy`) (required) |
| `root`, `log` | Absolute project root and caller log; output of every turn appends to the log (required) |
| `session`, `resume` | Native session; null starts a new one (`new_session`); a resume names it |
| `args`, `env`, `home` | Caller client arguments, child environment (strings) and account home, as in `Spec` |
| `command` | Optional base command (sandbox wrapper or another executable); default `agent::executable` |
| `input` | Turn text on stdin of the client (required) |
| `timeout_s` | Optional deadline, 1-86400 s; past it the turn reads `cancelled`/`timeout` |
| `optimization` | `inherit` (default), `off` or `on` |
| `context` | `product`, `version` (required), `project` (default `other`), `task` `[task, attempt]` or null, `route`, `account` (default `default`) |
| `store` | Absolute CoreQ telemetry store path for the `agent.turn` observation; null records nothing |

The report has `schema` 1, `session`, `outcome`, `error`, `text`, `exit`, `started_ms`, `duration_us`, `usage` (`calls`, `input`, `cache_read`, `cache_write`, `output`, `model`, `unknown`), `limit` (`used_percent`, `resets_at`, `window`, `unknown`), `reached` (`{"reset": n|null}` or null), `recorded` (`ok`, `not-requested` or the append error) and `optimization`. Background, app and app-server modes stay library APIs: they need caller-owned checkpoints and transports that one stdin request cannot carry.

## Optimization intent

`optimization::prepare(intent, product, spec, install)` resolves the effective state and adapts the child; `observe` resolves only. Both read and never write. The JSON command selects `optimization::install_root()` (agentq.md § 5 layout); library callers pass an installation or `None` for no installation; the client home is `Spec::home`, else the last child `env` home override, else the client's own home. An inherited `AGENTQ_OPTIMIZATION=off` is observed as `off` and carried into every Codex launch mode.

| Order | Condition | Effective | Reason |
|---|---|---|---|
| 1 | Explicit or inherited `off` | `off` | `launch_off` |
| 2 | No `<install>/data` | `off` | `not_installed` |
| 3 | `data/integration.json` or `data/config.json` unreadable or invalid under the shared lifecycle/config validators | `unknown` | `unreadable_state` |
| 4 | Host disabled or the client's profile disabled (agentq.md § 4) | `off` | `integration_disabled` |
| 5 | `compression.enabled` false | `off` | `host_off` |
| 6 | `compression.agents.<client>` false | `off` | `agent_off` |
| 7 | No client home known | `unknown` | `no_client_home` |
| 8 | Codex `config.toml` top-level `model_provider = "agentq"` (inline comments allowed), Claude `settings.json` has a compression hook command, Antigravity `hooks.json` (`<home>/hooks.json` or `<home>/../config/hooks.json`) has an enabled named group with a `PreToolUse` command calling `hook antigravity PreToolUse` | `on` | `configured` |
| 9 | Client configuration unreadable | `unknown` | `unreadable_client_config` |
| 10 | Otherwise | `off` | `not_attached` |

- `inherit` and `on` resolve the same way: `on` records the caller's request and never enables a switch, an integration or a disabled profile.
- Explicit or inherited `off` adds child env `AGENTQ_OPTIMIZATION=off`. Claude and Antigravity hooks, `agentq run` and `agentq compress` inherit it and pass through with reason `disabled`. For a Codex home routed through agentq it also adds `-c model_providers.agentq.http_headers.x-agentq-optimization="off"` to CLI, app queue and app-server commands; the loopback provider passes `off` to the compressor, and replayed bytes of earlier calls stay unchanged (agentq.md § 3).
- Every launch adds `AGENTQ_LAUNCHER=<product>` unless `TASKQ_TASK` or `AGENTQ_LAUNCHER` is already set, so agentq queues no second `agentq.model` count (agentq.md M6.9).
- Effective `on` is the configuration at launch. Actual compression is proven only by the session's `agentq.compress` records.

## Adoption and rollback

Consumers of CoreQ `agent` at the CoreQ v0.18.0 baseline (default branches read when this page changed):

| Consumer | Current use | Adoption |
|---|---|---|
| PilotQ | `coreq` v0.18.0 `agent` in `src/claude.rs`, `src/host.rs`, `src/manager.rs`, `src/main.rs`, `src/slots.rs` | Its own contract change, then the runtime revision; PilotQ #265 keeps its native qualification |
| TaskQ | No `agent` use on main; in-flight #2094 (PR #2108) adopts CoreQ `agent` | #2094 keeps its native-proof scope; TaskQ adopts the runtime revision in a later task, never as a second writer on that surface |
| DashQ, ServerQ | `coreq` v0.11.0, no `agent` use | None |
| AssetQ | No Rust dependency and no agent step yet | Its agent steps adopt `agentq agent turn` or the runtime revision through its own contract |

- Compatibility example source pin: `513193c8a96f3de1c71a86291a679230817ce0b2`; its manifest/lockfile retain that baseline. Protected API source pin: `aab1a190ab76f1e932578520d7a8787ef1d4b660`; [consumer recipe and publication evidence](../evidence/runtime-186.md).
- Publication: publish the runtime source commit and pin its full immutable Git SHA (`rev`) in `examples/runtime-consumer`; its checked lockfile must resolve that same remote SHA, without a local path patch. The example builds and runs both clients from that pin on each platform. Publication does not authorize consumer adoption or live installation. Optionally, after a merged main commit passes the `Test` workflow on Linux, macOS and Windows, `scripts/tag_runtime.py` checks that exact run and SHA, creates the missing tag `runtime-v<runtime/Cargo.toml version>` and reads it back. It never replaces an existing tag and rejects an existing tag at any other SHA. Consumers pin `agentq-runtime = { git = "https://github.com/taskqorg/agentq", tag = "runtime-v<version>" }`.
- `examples/runtime-consumer` is the minimal consumer outside the workspace; the runtime contract tests build and run it on each platform.
- Forward: a consumer edits its contract, switches `coreq::agent` to `agentq_runtime::agent` at one immutable revision, passes its own fake-client gates and qualifies its real native paths. A revision with the runtime trait (M12.9) replaces direct `Client`/`Spec`/`Turn` calls with `Runtime` operations, `Policy` and `Launch`; native behavior is unchanged. A fake-client pass never substitutes for native qualification.
- Rollback: the consumer selects its previous immutable build. Native sessions, persisted checkpoints and active work stay valid because native arguments, checkpoint formats and observation IDs are unchanged; no paid replay, credential, model, grant or capacity change.
- CoreQ keeps its `agent` module frozen and compatible during adoption. Its retirement is a separate CoreQ task, only after every consumer above adopted and qualified.
- This repository changes no consumer's live install; installed services keep running until their owners adopt.

## Caller responsibilities

- Rule text, sandbox policy, permission mode and attempt logs are consumer decisions; the caller passes them in.

History consumer example (caller-owned source and protected checkpoint storage):

```rust,ignore
use agentq_runtime::agent::history::{History, capabilities};
let support = capabilities("agy-stream", Some("1.4.12"), "macos")?;
let mut history = History::new("agy-stream", "worker-log", native_session)?;
let page = history.poll(caller_log)?;
// Persist this checkpoint only after durably accepting the page.
let saved = history.checkpoint();
let recovered = History::recover(&saved)?; // Observation only; no input replay.
```

Source-fixture provenance, adapter conclusions and remaining native gaps:
[worker history evidence](../evidence/runtime-202.md).
