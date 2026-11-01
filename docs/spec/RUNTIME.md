# Agent runtime

This page holds the binding detail of [agentq.md § 12](../../agentq.md#12-agent-runtime). The library moved unchanged from CoreQ v0.18.0 (`src/agent.rs`, `src/agent/`, `tests/agent.rs` and `docs/design/agent.md`), where CoreQ R47-R51 stated its rules; those rules hold here unchanged. Normal CLI behavior is TaskQ's CLI adapter (`src/runtime.rs` and `src/runtime/accounts.rs` at TaskQ v4.6.57), moved unchanged; compatibility APIs preserve its background and app boundaries.

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
- Antigravity hooks use `toolCall.args.CommandLine`, `conversationId` and `stepIdx`; the response shallow-merges `overwrite.CommandLine` and delegates to native permissions with `decision: "allow"` (never `auto_approve`). Only qualified simple commands are wrapped; shell composition and Windows command shells remain native. [Hook schema](https://antigravity.google/docs/hooks).


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

Normal `Spec`, `Client`, `Turn`, `args`, `command` and `run` retain the v0.17 CLI interface and behavior. Compatibility APIs never switch Claude to `-p` or replace an app session with `exec`.

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

The caller selects an immutable, reviewed host executable by canonical path and SHA-256, outside task/environment/receipt inputs. `ProtectedHost::taskq` checks that artifact before every invocation. This is a trust anchor supplied by the operator, not proof that arbitrary caller-selected code is a host authority. It does not defend against an unrestricted same-user operator replacing the trusted executable between verification and execution.

`launch` accepts the existing `taskq.app-server/1` request and a `ProtectedIdentity` match requirement: canonical account home and native executable/digest, generated profile digest and grant approver. The request binds canonical project, Codex runtime, account, policy revision and the existing slot's caller/operation/process identity. It accepts no native arguments, environment, model, executable override or serialized grant. Missing, extra or malformed request fields refuse before spawn; the host validates the protected grant and slot independently.

Before calling `launch`, the caller durably retains that exact operation/request and its private slot key. A launch attempt is never automatically retried, even on an error. The returned handle initially has unknown authority. `verify` obtains a separate, bounded trusted readback and checks requested, saved and verified-effective facts, the expected identity, exact supervisor/native creation identities and host generation. Every `send` repeats this check; no handshake, turn or tool response is sent before it passes. The host additionally revalidates authority under its own lock at actual native input delivery. A receipt returned to the caller is private evidence, never a bearer grant or an input that opens the gate.

Unknown, failed or mismatched readback refuses sending and retains the original binding; `verify` may reconcile that same binding without resubmission. A fully validated readback with a changed binding (authority, native/supervisor incarnation, profile or host generation), or an uncertain write, permanently fences the handle. `ProtectedHost::observe` and `cancel` support recovery of the original operation without launching again. Cancellation uses the host's exact-child authority and never releases a slot or proves descendants/turn completion. Dropping a handle, EOF, a timeout or supervisor exit leaves work uncertain; callers retain ownership and reconcile before the original whole-turn slot release.

Native stdio uses the existing bounded transport. `AppSession` still owns thread/turn correlation, interruption, checkpoints and its one idempotent `agent.turn` observation. Server requests and native account/model/permission parameters remain caller-owned. The adapter preserves the host's model/account and approved outer boundary; caller protocol parameters cannot widen it. Optimization intent travels only as child `AGENTQ_OPTIMIZATION`; the host prepares native `off` env/header and launcher marking. `optimization` reports configuration, never actual compression or enforcement.

The supported host protocol/source pin and consumer sequence are in [the protected consumer recipe](../evidence/runtime-186.md). Its current adapter supports existing explicit Mac Codex Workspace grants only; there is no Full fallback, new grant or Windows containment enablement. Portable fake-adapter checks prove the runtime gate and compatibility, not Mac containment. Live Mac image/sandbox/stdio, filesystem/network/child and outside write/delete/rename/control-store probes remain a separate native qualification gate. Do not replay the retained AssetQ crate.

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
- Explicit or inherited `off` adds child env `AGENTQ_OPTIMIZATION=off`. Claude hooks, `agentq run` and `agentq compress` inherit it and pass through with reason `disabled`. For a Codex home routed through agentq it also adds `-c model_providers.agentq.http_headers.x-agentq-optimization="off"` to CLI, app queue and app-server commands; the loopback provider passes `off` to the compressor, and replayed bytes of earlier calls stay unchanged (agentq.md § 3).
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
- Forward: a consumer edits its contract, switches `coreq::agent` to `agentq_runtime::agent` at one immutable revision (same API and behavior), passes its own fake-client gates and qualifies its real native paths. A fake-client pass never substitutes for native qualification.
- Rollback: the consumer selects its previous immutable build. Native sessions, persisted checkpoints and active work stay valid because the API, checkpoint formats and observation IDs are unchanged; no paid replay, credential, model, grant or capacity change.
- CoreQ keeps its `agent` module frozen and compatible during adoption. Its retirement is a separate CoreQ task, only after every consumer above adopted and qualified.
- This repository changes no consumer's live install; installed services keep running until their owners adopt.

## Caller responsibilities

- Rule text, sandbox policy, permission mode and attempt logs are consumer decisions; the caller passes them in.
