# Agent runtime

This page holds the binding detail of [agentq.md § 12](../../agentq.md#12-agent-runtime). The library moved unchanged from CoreQ v0.18.0 (`src/agent.rs`, `src/agent/`, `tests/agent.rs` and `docs/design/agent.md`), where CoreQ R47-R51 stated its rules; those rules hold here unchanged. Normal CLI behavior is TaskQ's CLI adapter (`src/runtime.rs` and `src/runtime/accounts.rs` at TaskQ v4.6.57), moved unchanged; compatibility APIs preserve its background and app boundaries.

Current APIs expose worker preflight, launch/resume, observation, control and session history; the session-archive operation remains accepted scope awaiting implementation under [the shared history/archive contract](https://github.com/taskqorg/specq/blob/main/engineering/agent-runtimes.md#step-history-and-archive).

## Agent registry

`agent::registry` is the single catalog consumed by the worker library and the installation CLI. One module per native client owns its descriptor, aliases, policy and login mapping, native home lookup, profile metadata, files, journal edit requests, record-hook action, read-only profile inspection and legacy provider diagnostics. Desktop Chat is an integration-only entry with no worker runtime; unsupported operations stay explicit.

The build discovers entry modules and generates references only; it does not run a native client or touch an installed profile. Stable ordering is entry data, preserving the existing runtime, onboarding and profile output order. The existing Runtime, Descriptor, Launch and optimization compatibility APIs retain their behavior.

Core profile code still owns HMAC identities, revisions, error classification and JSON projections. The journal still validates edits and preserves later foreign changes. Hook code still owns bounded recording and fallback responses; an entry selects the native action. CLI presence is checked before setup writes a profile. Discovery skips and names profiles without a CLI; an explicitly selected missing CLI refuses. Configure and enable retain their pre-write gate. Presence alone does not verify login or a native turn.

Each entry supplies every required callback. Registry validation rejects missing identity/profile metadata and duplicate IDs or aliases. Behavioral tests exercise capability operations in both directions: declared operations reach their implementation rather than an unsupported refusal, while undeclared operations return their documented unsupported/absence result. These checks use isolated files and inert commands, not login or paid calls. The fake module uses the shared Adapter and a scripted CLI through launch, events and report.

Private execution dispatches registered protocol callbacks for native events, reports, sessions, transcript observation and quotas. Client identity is opaque registration data; a new protocol needs no enum variant or parser arm in core. Codex app-server, Claude background and Antigravity sidecar details belong to their runtime modules. Shared code retains process ownership, immutable records, runtime-neutral report assembly and transport/recovery checks. Native start-limit detection, terminal classification, call usage, final text, history identity/normalization and Codex rollout parsing dispatch through the selected protocol; an unrelated runtime using `assistant` or `result` records does not inherit another runtime's semantics. Existing native behavior assertions are preserved.

## ACP sessions

The shared ACP adapter speaks JSON-RPC 2.0 over native stdio. Registration supplies command arguments, environment, home selection, mode mappings and rejected server versions; the adapter contains no runtime names. It checks protocol version 1 and message envelopes, preserves unknown updates and native request data in the private log, and exposes only capabilities actually reported by the server.

| ACP method or update | Runtime meaning |
|---|---|
| `initialize` | Protocol version, native version, authentication methods and live capabilities; unsupported protocol or a rejected version refuses before a session or prompt. |
| `session/new`, `session/load` | Start or resume the opaque native session; load replay is retained as replay evidence and never emitted again as live progress or counted as new usage. |
| `session/prompt` | Submit the ordered text/image input once; only a checked response correlated to the submitted prompt is terminal evidence; malformed consumed fields, replay, EOF and process exit have no terminal authority. |
| `session/cancel` | Request native cancellation and answer every pending permission request with the ACP cancelled outcome, including requests racing cancellation; only the prompt's terminal reply confirms the turn outcome. |
| `session/set_model`, `session/set_mode` | Set a model or declared native mode; refusal stays a native error. |
| `session/request_permission` | Typed pending request, with original ID and offered choices; a reply selects only an offered option or cancels. Questions never receive an automatic approval. |
| `agent_message_chunk`, `agent_thought_chunk` | Assistant text or thinking; typed history uses the shared `agent::history` step projection, including explicit-source reads through `Runtime::history`. |
| `tool_call`, `tool_call_update` | Stable native tool ID, kind, input, output and state; missing fields remain unknown. |
| `usage_update`, prompt `usage` | Observed native usage only; absent tokens and model-call counts stay unknown. |

Antigravity uses the installed `agy acp`; there is no downloaded server or managed pin. ACP versions 1.4.2 through 1.4.5 refuse because they may write built-in skills into the workspace. Every public Antigravity home is a Gemini root (`GEMINI_HOME`, else `~/.gemini`), including account homes, quota, identity, credentials and protected paths. Tokens live in `<root>/antigravity-cli/antigravity-oauth-token` or `<root>/oauth_creds.json`; isolated roots never scan a different home. Its ACP home is selected explicitly for an isolated chat; the server does not inherit a different scanned home. The canonical worker preflight probes the selected executable version before starting ACP, refusing unreported, malformed, below-minimum and known-bad versions even when initialize omits agentInfo. Checked native payload types cover initialization, session IDs, prompt completion, updates, permissions and usage; unknown extensions remain raw.

Live progress, retained classification and log events share the checked session/prompt interpretation; a stored terminal marker grants no authority. Rejected messages remain raw evidence. Malformed JSON, invalid UTF-8 and incomplete final records retain their exact bytes in the private diagnostic file before an error returns, never in telemetry.

A null-ID JSON-RPC error is retained and emitted as a protocol-error observation without being mistaken for a matching reply. A qualification records the exact CLI version, empty-folder write inspection and source-profile preservation.

The existing stream and agentapi native seams remain available until ACP has side-by-side task, tools, resume, cancellation, usage and history proof on macOS and Windows. These low-level seams serve comparison proof only; `launch` and `resume` always select ACP, without command-name opt-outs or sidecar fallback. Consumers must provide an ACP-capable executable and pass a Gemini root in `Launch.home` (or `GEMINI_HOME`); the old agentapi data directory is not an ACP home. Authentication uses the separate login flow; launch never starts interactive sign-in. A missing login or native-platform proof remains an explicit qualification gap.

ACP imposes no record-size, queue-size or shutdown-grace limit. EOF closes its owned server; poll and progress never wait for process exit. A terminal native turn closes stdin and uses `try_wait`; an unexited server reports null exit with `session-process-running`. An actual owned-child wait confirms direct process termination, including a signal with no numeric code; the native turn outcome stays unchanged. Transport sentinels and AgentQ cleanup codes never become numeric native report exits; legacy stop observations retain their existing representation. Dropping the run owns final process cleanup. The shared `stop_event` observation uses the same run stop identity and ACP log boundary; a native terminal with a live server reads `turn_end_process_live` without changing poll/report authority. On Windows an owned, non-breakaway Job Object binds the native server plus an already-started cmd/Node shim subtree before protocol input; startup enrollment verifies the live parent and creation relationship through held native identities before suspension or assignment, and resumes only its own suspension. A stale snapshot or unknown relationship refuses attachment without touching an unrelated process. An attachment refusal returns only fixed operation labels and numeric/boolean native identity, query, requested-rights and held-parent wait metadata; it emits no success-path diagnostics or telemetry and grants no authority from diagnostic observations. Cancellation teardown and drop terminate that job, including native descendants; caller process flags and hooks are preserved. ACP owns a stateful stdio connection. A process checkpoint reads `no-checkpoint`; recovery never recreates that connection or resubmits its prompt. A later explicit resume uses `session/load`. The retained stateless CLI seam keeps its process-identity recovery contract.

## Additional native launch registrations

The catalog exposes qualification independently of protocol support. An entry with missing native proof is `unqualified`: its descriptor and reason remain inspectable, but runtime lookup and the offered runtime list do not return a launchable adapter. Installing a CLI, protocol-double checks and CI never promote it. Qualification binds the exact native version and platform and includes the canonical worker preflight before native input.

| Runtime | Protocol and command | Minimum verified upstream release | Home | Candidate qualification |
|---|---|---|---|---|
| Grok | ACP, `grok agent stdio` | Unknown locally | `GROK_HOME` | Unqualified: no complete native proof |
| OpenCode | ACP, `opencode acp` | 1.18.31 | `XDG_DATA_HOME` / native OpenCode account layout | Unqualified: native proof at or above the minimum is missing |
| OpenCode2 | ACP, `opencode2 acp` | 2.0.14 | Native OpenCode account layout | Unqualified: no installed client or native proof |
| OMP | ACP, `omp acp` | 17.0.5 | `PI_CODING_AGENT_DIR` | Unqualified: no installed client or native proof |
| Pi | JSONL RPC, `pi --mode rpc` | Unknown locally | `PI_CODING_AGENT_DIR` | Unqualified: no installed client or native proof |

ACP entries reuse the shared adapter. OpenCode disables unsupported questions rather than presenting a reply path the server cannot honor; OMP disables PTY tools in a headless session. Entry metadata records these differences; product code adds no runtime-name branch.

Pi's native RPC uses `get_state` for its opaque session-file handle, `prompt` for one input, `abort` for cancellation and `--session` for explicit resume. `message_update`, `message_end` and `tool_execution_*` become neutral events; only `agent_settled` completes a turn; `agent_end` may precede automatic retry or compaction. Per-message native usage becomes model-call evidence; a command acknowledgement or EOF never proves completion. Native errors, unknown records and missing fields retain their meaning and raw source. Canonical stop observations retain the per-run identity and current RPC log boundary, independently of turn completion. Its permissive native policy cannot enforce narrower filesystem, network, tool or approval grants and refuses them before spawn.

Pi submits input only when native state explicitly reports `isStreaming=false`, `isCompacting=false` and numeric `pendingMessageCount=0`; missing, null or malformed queue state refuses before input. The first native settlement freezes messages, calls, usage and outcome. Later records remain private raw evidence and produce no further progress or accounting for that turn.

Candidate registrations declare their native provider authentication and session environment variables. The existing account cleaner removes inherited authentication for an explicit home and always removes inherited session markers; explicit caller overrides remain authoritative. Grok declares `XAI_API_KEY`; the multi-provider candidates declare their provider keys, OAuth tokens and credential selectors. Pi-family session markers and session-directory overrides do not leak into a child. These declarations do not qualify native account isolation.

Gemini's ACP candidate should be evaluated after these registrations; this change adds no unproven launch registration for it. Hermes remains hooks and transcript observation only until a supported structured protocol is qualified. Claude Desktop Chat remains integration-only. For agents exposing only a TUI, retain read-only transcript/hooks integration; TUI scraping requires a separate owner decision and is not implemented here.

## Runtime trait

M12.9/M12.10 apply the [specq agent-runtime rule](https://github.com/taskqorg/specq/blob/main/engineering/agent-runtimes.md). The public surface is `agent::Runtime`, `agent::Descriptor`, `agent::Policy`, `agent::Launch`, `agent::Run` and the runtime-neutral `Event`, `Window`, `Report`, `Usage`, `Snapshot`, `Accounts`, `Context` and `history` types. `agent::runtimes()` lists every adapter; `agent::runtime(name)` finds one by descriptor name (alias `agy` selects `antigravity`). Callers treat names and session IDs as opaque data.

Each adapter explicitly implements every required operation. `account_limits_with_executable` has no default that discards a caller executable; consumer-defined `Runtime` implementations must implement the operation when adopting this contract. The sections below remain the binding detail of what each adapter does natively.

| Operation | Contract in this runtime |
|---|---|
| `preflight(&Launch, &Repair)` | Ordered readiness checks and explicitly authorized preparation, with no model call ([Worker preflight](#worker-preflight)). |
| `launch(&Launch)` | Check `quota` when `Launch.accounts` is set (none free: error `quota-exhausted`), map `Policy` (refusal names the part), retain the compatible no-op optimization report, start one turn and return a `Run`. |
| `resume(session, &Launch)` | The same for an existing opaque session ID. |
| `input(run, parts)` | Send more user input to a running session as ordered `Part`s: `Text` or `Image` (an absolute path to a local file the caller already verified). A protected app-server run starts its next turn with exactly these parts, once; a later turn never repeats earlier parts. Refusals are typed `Failure::Refused(cause)` before any native write: `input-empty`; `image-unsupported` without capability `image-input`; `image-invalid` for a relative path or a missing file; `input-unsupported` for a one-process CLI, background or queue run, whose next turn is a `resume`. A native or transport error is `Failure::Failed` and is never retried. |
| `account(run)` | Read-only login identity of a session run (capability `account`): `Login` `subscription`, `api-key` or `none`, and the native plan name when reported. It never refreshes, writes or exports a credential, token or email. Other runs return `Failure::Refused` cause `account-unsupported`; an unrecognized login reads as `Failure::Failed`, never as a guess. |
| `catalogue(home)`, `catalogue_with_executable(home, executable)` | Models the login in `home` can use (capability `catalogue`): Codex uses the supplied executable when present (otherwise native lookup), then one bounded `model/list` over its own short app-server session, with no run. Other runtimes refuse with `catalogue unsupported by <name>`; a caller then offers descriptor `models`. |
| `cancel(run)` | Request the stop; returns whether exit is already confirmed. `poll` confirms it later. It never proves server-side stop (see Arguments). |
| `poll(run)` | `running`, `finished`, `failed` or `unknown` from the existing end detection. |
| `report(run)` | The `Report` of a finished run; a running one returns cause `running`. |
| `log_events(log)` | Read complete JSONL attempt-log records into ordered typed events without a run, process, transcript lookup or write. Non-JSON diagnostic lines are ignored; native parsing stays in the adapter. Text deltas are included. |
| `log_records(log)` | The same read as `log_events`, one `LogRecord` per source record, so a caller keeps source order and native time without native parsing ([log records](#log-records)). |
| `account_identity(home)` | Read-only account identity without a run: Codex `account/read` with `refreshToken: false` returns email, else `chatgptAccountId`; Antigravity `loadCodeAssist` returns `cloudaicompanionProject`. Unknown or unsupported reads return `None`. No token leaves the adapter; no credential is refreshed. |
| `progress(run)` | Typed `Event`s since the previous call, in native order, each returned at most once ([progress events](#progress-events)). |
| `launcher_output(run)` | The raw `background` launcher exchange of this run, once (Caller launch data). |
| `session_id(run)` | The native session once named, else none. |
| `session_url(run)` | The URL or cause of `observe` (Session observation), without polling. |
| `observe(run)` | One runtime-neutral `Observation`: `poll` state (a failed poll reads `unknown` with its error), native session, and session URL or absence cause with its native detail (Session observation). |
| `transcript(run)` | The caller log the run appends to. |
| `ready(executable, home)` | `install` (the given executable, else the CLI on `PATH`), `version` (`--version` within 5 s; `outdated` below the descriptor `minimum`) and `login` in `home` (none: the client's own). Login runs the native status command within 10 s: Claude `auth status --json` reads `ok` only for `loggedIn` with `authMethod` `claude.ai` and `apiProvider` `firstParty`; Codex `-c forced_login_method="chatgpt" login status` reads `ok` for `Logged in using ChatGPT`. Another login reads `api-key`, none reads `missing`, and a failed or unreadable status reads `unknown`. Antigravity has no status command: its credential file is absent and reads `unknown`. Each check carries a reason. `ready_for` and preflight use the launch policy: subscription-only Codex probes force ChatGPT; other probes report the selected authentication mode. |
| `quota(&Accounts, now)` | The account a launch takes, whether every account is disabled, and the hold end when every enabled account is limited. |
| `running()` | Runs of this adapter in this process that have not reached a terminal poll and were not dropped. |
| `protected_paths(home)` | Native home folders and files a run must not overwrite: Claude home and `.claude.json`, Codex home, Antigravity home and its sibling `config`. |
| `home_env(tool, label)` | The home variable pointing at `<base home>/<tool>-accounts/<label>`; none for `default`, which keeps the caller's own home. |
| `checkpoint(run)`, `recover(saved, &Launch)` | Private recovery value of a CLI turn, background or queue run, and the run rebuilt from it without a native call (Turn recovery and process setup, Compatibility sessions). An `agentapi` submission returns cause `no-checkpoint`. |
| `log_limits(log, session, home)` | Every quota `Window` in caller-held log text, else in the session's native transcript in `home`, each with its native event ID when reported. No run, no write ([limit snapshot](#limit-snapshot)). |
| `account_limits(home, now)`, `account_limits_with_executable(home, executable, now)` | Every quota `Window` of the login in `home`, from its native read-only source (capability `account-limits`): Codex `account/rateLimits/read` using the supplied executable when present (otherwise native lookup), Claude OAuth usage, Antigravity code-assist. A credential file changed during the read makes the result unknown. An absent Codex `auth.json` does not block its native app-server read (native credential stores are supported); unreadable existing files and changes during the read remain unknown. Never refreshes or exports a credential. |
| `classify(log)` | The `Report` of caller-held log text without a run: the same `Report::read` as a finished turn, with no exit code, deadline or rollout. `detail` holds the native reason of `auth-rejected` or `model-capacity`, the Claude terminal result subtype, or the Codex `turn.failed.error.message`; absent native detail stays unknown. A terminal result detail takes precedence over an earlier assistant failure detail while preserving its typed error classification. One event classifies as its own one-line log. |
| `executable(path)` | The client executable resolved on `path` (none: the process `PATH`), as in Arguments; Codex resolves its native executor. Unresolved names come back bare; nothing is spawned. |
| `home()` | The client's own home (Arguments, `protected_paths`). |
| `session_transcript(session, home)` | The native transcript file of a session in `home` (none: the client's own): the Codex rollout. Other runtimes and an unknown session read none. |
| `session_link(&NativeSpec, job)` | Capability `session-url`: the session URL of caller-built session `spec.session` (background `job`, if any) in `spec.home`, or its absence cause with native detail (Session observation). `--remote-control` in `spec.args` marks Remote Control requested. Other runtimes read `no-session-url`. |
| `prepare(intent, product, &mut NativeSpec, install)` | Compatible no-op optimization report and independent launcher attribution for a caller-built spec. |
| `native_arguments(&NativeSpec)`, `turn(base, &NativeSpec, log, input, &Process)` | Native argument inspection (Arguments), and one owned `Turn`: `id`, `poll`, `kill`, `wait`, `report`. For a caller that adds native arguments of its own; `launch` stays the default. |
| `background_launch`, `jobs`, `job`, `stop_job` | Capability `background`: the launcher exchange, the job list (`filtered` adds the project filter), one validated `BackgroundJob` row and its stop request (Compatibility sessions). Other runtimes refuse with `background unsupported by <name>`. |
| `launch_refusal(text)` | The named cause of a launch failure, from its stderr or `launch`/`resume` error text: `workspace-untrusted` (Claude `Workspace not trusted`), `session-missing` (Claude `No conversation found with session ID`, Codex `app-server thread/resume refused`), `login-required` (Claude `Not logged in` or `/login`; Codex `401`, `login` or `log in`, any case); otherwise none. Callers match the cause, never the text. |
| `app_queue`, `app_server_arguments` | Capabilities `app-queue` and `app-server`: the desktop-thread queue exchange and read-only app-server argument inspection. Submission uses the owned runtime or protected-host path. Other runtimes refuse with `<capability> unsupported by <name>`. |

`NativeSpec` is the caller's view of one native session for these operations: `root`, `session`, `resume`, `args` (native client arguments in the Arguments position), `home` and `env`; the adapter adds its own client. Callers gate each optional operation on `descriptor().has(capability)`.

Descriptor data, the only runtime metadata a caller reads. `models` are the model aliases a caller may offer when it has no catalogue read; `minimum` is the oldest supported CLI version (empty: none).

| `name` | `label` | `brand` | `family` | `models` | `minimum` | `capabilities` |
|---|---|---|---|---|---|---|
| `claude` | Claude Code | Anthropic | `claude` | `opus`, `sonnet`, `haiku` | 2.1.0 | `turn`, `resume`, `caller-session-id`, `compact-hook`, `session-url`, `quota-snapshot`, `account-limits`, `background`, `title`, `mcp-config`, `instructions-file`, `turn-parts`, `turn-deltas`, `settings-env` |
| `codex` | Codex CLI | OpenAI | `gpt` | | 0.162.0 | `turn`, `resume`, `quota-snapshot`, `account-limits`, `catalogue`, `app-queue`, `app-server`, `effort`, `image-input`, `account`, `host-args`, `settings-env`, `mcp-config`, `instructions-file`, `turn-parts`, `turn-deltas` |
| `antigravity` | Antigravity | Google | `gemini` | | | `turn`, `resume`, `title`, `account-limits` |

### Session observation

`observe(run)`, `session_url(run)` and `session_link(&NativeSpec, job)` name the session link or exactly one cause. They read native records only; no launch, consumer log parsing or rewrite is involved, and a recovered run reads the same records.

| Cause | Meaning |
|---|---|
| `no-session-url` | Codex, Antigravity and app-server runs have no session URL. |
| `session-unknown` | No native session is named yet. |
| `not-requested` | Claude: the launch had no `title`, so no `--remote-control`, and no native record shows Remote Control. |
| `pending` | Claude: Remote Control was requested and the debug log records neither a URL nor a failure yet. |
| `unlogged` | Claude: Remote Control was requested, no URL is recorded and no debug log exists, so registration state is unknown (P2). |
| `disabled` | Claude: the debug log records a policy, plan or login refusal; detail holds the native text. |
| `failed` | Claude: the debug log records another registration or connection failure; detail holds the native text. |
| `evidence-unreadable` | Claude: a job state or debug log exists but cannot be read or parsed; detail holds the error. |

Claude sources, each in the client home (`CLAUDE_CONFIG_DIR` or `~/.claude`, the same layout on macOS and Windows), latest evidence first:

1. A `system` `bridge_status` record with an `https://claude.ai/code/session_*` URL in the run's log or native transcript.
2. A background job's `jobs/<job>/state.json`: `bridgeSessionId` `cse_<id>` gives `https://claude.ai/code/session_<id>`; `respawnFlags` containing `--remote-control` marks it requested.
3. The debug log `debug/<session>.txt` (written when Claude debug logging is on), last 16 MiB, lines whose marker follows only the logger's `<time> [<LEVEL>] ` prefix: the last of `[bridge:repl] Hook initialized, session=cse_<id>` or `Mirror initialized` (URL), `[bridge:repl] Init failed: <detail>`, `notifyBridgeFailed detail="<detail>"` and `Org policy now refuses the connected bridge` decides. A detail naming a policy, plan, subscription, login or `disabled` reads `disabled`, any other `failed`.

The runtime never enables Claude debug logging; turning it on for workers is a separate launch decision with its own native qualification.

### Policy map

`Policy` has `read` and `write` scope (`native`, `none`, `project`, `any`), `network` (`native`, `on`, `off`), `tools` (`native` or an allowlist), `approval` (`native`, `plan`, `edits`, `auto`, `deny`, `bypass`), `roots` (`native`, `exact` list or `add` list of extra absolute writable folders), `unelevated` (the runtime's own Windows sandbox runs without elevation), `builtin` (`native`, `on`, `off`: the runtime's own shell and web tools), `isolated` (no user settings, plugins, slash commands or extra MCP servers) and `subscription` (subscription login only, never an API key). `native` keeps the runtime's own default. Every value without a row below refuses launch with `policy <part> unsupported by <name>`.

| Part | Claude | Codex | Antigravity |
|---|---|---|---|
| `read` | `native` | `native`, `any` | `native` |
| `write` | `native`; `any` with `approval` `bypass` (no flag enforces `none`: MCP servers and project hooks still write) | `none` → `--sandbox read-only`, `project` → `--sandbox workspace-write`, `any` → `--sandbox danger-full-access` | `native` |
| `network` | `native` | `on` with `project` → `-c sandbox_workspace_write.network_access=true`; `off` with `project` → `-c sandbox_workspace_write.network_access=false`; `off` with `none`; `native` | `native` |
| `tools` | `native`; allowlist → `--allowedTools <tool>..` | `native` | `native` |
| `approval` | `plan`, `edits` (`acceptEdits`), `auto`, `deny` (`dontAsk`), `bypass` (`bypassPermissions`) → `--permission-mode <mode>` | `deny` → `-c approval_policy=never` after the `write` sandbox; `auto` with `write` `project` → `--full-auto` instead of `--sandbox`; `bypass` with `write` `any` → `--dangerously-bypass-approvals-and-sandbox` instead of `--sandbox` | `bypass` → `--dangerously-skip-permissions` |
| `roots` | `native` | `exact` → `-c sandbox_workspace_write.writable_roots=[]` then `--add-dir <root>` per absolute root, so config-defined roots never widen it; `add` → only `--add-dir <root>` per absolute root, added to config-defined roots | `native` |
| `unelevated` | `false` | `true` → `-c windows.sandbox=unelevated` | `false` |
| `builtin` | `native`, `on` (Claude's default tool set); `off` → `--tools ""` | `on` → `--enable shell_tool -c web_search="live"`; `off` → `--disable shell_tool -c web_search="disabled"` | `native` |
| `isolated` | `true` → `--strict-mcp-config --setting-sources project --disable-slash-commands --system-prompt-snapshot off`; user hooks are excluded | `true` → `--disable` `apps`, `plugins`, `browser_use`, `computer_use`, `multi_agent` and `image_generation` | `false` |
| `subscription` | `true` → child `ANTHROPIC_API_KEY` and `ANTHROPIC_AUTH_TOKEN` empty | `true` → `-c forced_login_method="chatgpt"` | `false` |

Codex keeps its native project trust check; an untrusted root refuses rather than adding `--skip-git-repo-check`.

Codex `builtin`, `isolated` and `subscription` flags follow the `exec` or `app-server` subcommand, so they hold in app-server mode too.

`Launch.model` maps to Claude `--model <m>`, Codex `--model <m>` and Antigravity `--model=<m>`. `Launch.effort` (capability `effort`, a plain word) maps to Codex `-c model_reasoning_effort=<e>`; other runtimes refuse it. `Launch.compact_hook` maps to Claude `--settings` with a `SessionStart` `compact` hook; other runtimes lack capability `compact-hook` and refuse it. `Launch.home` is the login home of the run (the client's home variable; none keeps the caller's own). A turn also takes caller relay data, each behind its capability; another runtime refuses it with `<capability> unsupported by <name>`:

- `Launch.mcp` (`mcp-config`): an MCP config file → Claude `--mcp-config <file>`, and every server named in its `mcpServers` joins the `tools` allowlist as `mcp__<server>`; Codex thread `config.mcp_servers.<server>` with `command`, `args`, `env` and `default_tools_approval_mode: "approve"`. A resumed turn may name other servers. Env values never reach an argument.
- `Launch.instructions` (`instructions-file`): a file appended to the system prompt → Claude `--append-system-prompt-file <file>`; Codex thread `developerInstructions`.
- `Launch.parts` (`turn-parts`): ordered `Part`s as the turn input instead of `input` → Claude `--input-format stream-json` and one stdin user message, images as base64 blocks (`png`, `webp`, `gif`, else `jpeg`); Codex `turn/start` `text` and `localImage` items in the caller's order. An unreadable image fails the launch before spawn.
- `Launch.deltas` (`turn-deltas`): `Text` events in `progress` → Claude `--include-partial-messages`; Codex `item/agentMessage/delta`.

### One run path

The caller never selects a transport. The Codex adapter runs a launch with plain text input as `exec --json` (Arguments). A launch with `mcp`, `instructions`, `parts` or `deltas` runs as one `app-server` session owned by that turn:

1. Start `app-server` with the mapped flags and exchange `initialize`/`initialized`. The adapter requests no experimental API.
2. Send `thread/start`, or `thread/resume` with `excludeTurns: true`, with the mapped policy, model and relay data.
3. Send `turn/start` with the input and map notifications to events.
4. Answer every server request with JSON-RPC error `-32601`, since the policy allows no approval and no caller tool protocol exists. Stop the session on turn end.

The returned `Run` behaves the same in both forms: `poll`, `report`, `progress`, `cancel` (turn interrupt, else stop), `session_id` and one `agent.turn` record. `input` on such a run returns `input-unsupported`; its next turn is a `resume`. The stdio session ends with its owner, so a recovered app-server turn rebuilds observation only and never resubmits; a caller that must survive its own restart launches plain text.

### Progress events

| `Event` | Meaning | Claude | Codex app-server | Codex `exec` |
|---|---|---|---|---|
| `Text { item, delta }` | Live text, only with `Launch.deltas` | `text_delta` partial record | `item/agentMessage/delta` | absent |
| `Message { item, text }` | One complete assistant text block or native message | `assistant` text block, in native block order | `item/completed` `agentMessage` | `item.completed` `agent_message` |
| `ToolStart { id, kind, name, detail, timeout_ms }` | A tool began | `assistant` `tool_use` | `item/started` tool item | `item.started` tool item, rollout `function_call` or `custom_tool_call` |
| `ToolEnd { id, status }` | A tool ended | `user` `tool_result` | `item/completed` tool item | `item.completed` tool item, rollout `function_call_output` or `custom_tool_call_output` |
| `Quota(Vec<Window>)` | Windows just reported | `rate_limit_event` | `account/rateLimits/updated` | rollout `token_count` |
| `Context { used, window }` | Context use of the last model call | `assistant` usage and session window | `thread/tokenUsage/updated` | rollout `token_count` |
| `TurnStart` | A turn boundary: pending tools and earlier progress no longer apply | `system` `init` | `turn/started` | `turn.started` |
| `Progress` | `log_records` only: a supported record proved work advanced without a payload to report. A live stream keeps `Activity` for these records | nonempty string `user` text block, repeated `assistant` text | `item/completed` known non-tool item | `item.completed` known non-tool item, rollout `message` or `reasoning` |
| `Unsupported { reason }` | A record the adapter recognizes but cannot read; the observation stays unknown | none | `turn/completed` with an unknown status | none |
| `TurnEnd { outcome }` | Native terminal evidence (`ok`, `failed`, `cancelled`, `refused` or `unknown`); never process or descendant completion | `result`; `system` `turn_duration` reads `unknown` | `turn/completed` | `turn.completed`, `turn.failed`, `turn.cancelled`, rollout `task_complete` and `turn_aborted` |
| `Activity` | Any other native record, so inactivity watches stay correct; it proves no progress | every other record | every other notification | every other record |

Antigravity maps `init` to `TurnStart`, a known non-tool `step_update` (`user_input`, `agent_response`, `system_message` or `checkpoint`) in state `DONE` to `Progress` in `log_records`, and the other `step_update` and `result` records to the events they name. A tool step in an unknown state or a `result` with an unknown status reads `Unsupported`; everything else reads `Activity`. Native terminal records also emit `TurnEnd`; `poll` and `report` remain the run-completion authority. Antigravity sidecar tool-bearing planner records do not end a turn. Live app-server terminal events require the run's accepted native thread and turn IDs; a run-free reader describes the supplied records only. An unknown tool `kind` reads `Other`; known kinds are `Shell`, `Mcp` and `Web`.

`ToolStart.timeout_ms` reads `timeout_ms`, `timeoutMs`, `TimeoutMs` or `timeout` in milliseconds and `timeout_seconds` in seconds. A rollout call reads them from its `arguments` JSON and exports nothing else of it. Zero or an unreadable value is absent.

`ToolStart.detail` is an optional local audit target, never telemetry. Adapters select only command or file fields, never tool input objects, output, patches or dynamic-tool arguments. Claude maps Bash `input.command` and Read/Write/Edit/NotebookEdit/MultiEdit `input.file_path`; Codex maps command and changed paths on both transports. Simple filesystem commands retain their command and file operands; known non-file commands retain only the executable with redacted arguments; unknown command forms are withheld. Shell expressions, environment assignments, URLs, control characters and credential-like operands are withheld. File details contain paths only, with the same scrubber. Antigravity reads only known native command/path fields; unsupported tool schemas keep detail absent. Missing or withheld detail stays absent; callers keep their existing secret scrubber before storage or display.

Consumers matching every `ToolStart` field must add `timeout_ms` or use `..`; exhaustive event matches must handle `TurnEnd`. The optional timeout is milliseconds: Claude Bash `input.timeout`, Codex item `timeout_ms` / `timeoutMs`, and explicit Antigravity tool parameter `timeout_ms` / `TimeoutMs`. Async yield delays are not timeouts; absent or malformed timeouts stay `None`. `catalogue(home)` keeps native lookup; a caller with a configured executable uses `catalogue_with_executable(home, Some(path))`. Run checkpoints retain their event cursor schema; ownership is read from the run, never inferred from a notification.

App-server context notifications require a non-empty thread ID matching the run's owned native thread. Foreign or missing thread IDs yield activity only, without changing or invalidating context. Missing owned-thread measurements remain unknown. Exec rollout context comes only from the run's owned session file.

Codex emits tool events only for native tool items. Compaction, non-tool items and unknown future item types remain `Activity` in live events, on both exec and app-server paths. In `log_records`, completed `agentMessage`/`agent_message`, `userMessage`/`user_message`, `reasoning`, `plan`, `todo_list` and `contextCompaction`/`context_compaction` items can prove progress. Missing or unknown item types prove none.

Antigravity sidecar tool calls and responses map to `ToolStart` and `ToolEnd`. Neither Antigravity source reports context capacity or quota events; these stay absent. Requesting live text through `Launch.deltas` is unsupported on Antigravity. Native `agy` text deltas without that request remain activity. No synthetic context or quota is emitted.

Antigravity event qualification requires a native tool completion with status `DONE`; a successful final model response does not prove tool execution. A denied, cancelled or unknown tool result fails the qualification probe, which reports every `ToolEnd` status and retains the native log. Native macOS event evidence includes a pre-tool-hook denial: message and error events were observed, but successful tool execution remains unqualified ([native evidence](../evidence/runtime-246-antigravity.md)). Hooks and native authority are never bypassed to qualify this path.

`Launch.command` is a caller sandbox command that replaces the native executable; `Launch.mode` selects a compatibility mode (Compatibility sessions). The adapter places these mapped arguments where caller `<args>` stood in the Arguments table, so the native argument order is unchanged.

### Log records

`LogRecord { line, time, events, assistant_failure, lifecycle, permission_mode }` is one source record: its zero-based `line` in the supplied text, its `RecordTime` and the events it produced. Records keep source order; events inside a record keep native order. A line that does not start with `{` is a diagnostic and yields no record.

`assistant_failure` is an optional `AssistantFailure { code, error, detail }`. A Claude
assistant record with `isApiErrorMessage: true` or a string `error` is a failure;
`code` retains the native error string when reported, `error` is `auth-rejected`
for `authentication_failed` and `client-error` otherwise, and `detail` retains
its text blocks in order without rewriting their strings. Missing code or text
stays null. Successful assistant prose and user-supplied error-shaped content do
not become failures. `classify` also recognizes these records: the latest result
or assistant failure determines the turn outcome, so a later successful result
supersedes an earlier failed request. A failed Claude result retains the cause
of its most recent assistant record, bounded by the previous result or system
init/turn-duration boundary. It does not inherit an earlier failure across a
successful assistant record or a turn boundary. An assistant failure alone is not a new
native process-completion or cleanup receipt; live turn-end detection is unchanged.

`lifecycle` is an optional display label, independent of turn acceptance or
completion. Native lifecycle names replace `task_` with `turn_` and underscores,
dots and slashes with spaces: for example `turn started`, `turn complete`,
`system compact boundary` and `session started` (native `init`). Quota-only
records have no label. Unknown shapes have no inferred lifecycle.

`permission_mode` retains a nonempty Claude `permissionMode` string on that
record, otherwise null. It is saved observation, never a permission grant or a
fallback to requested or earlier permissions. Callers validate it against their
accepted resume baseline; absent, unsupported or changed modes stay unknown or
refuse. A later record without a mode does not assert the previous mode.

These three fields are also top-level projections on each `history` event.
Consumers need neither `raw` nor a block's native JSON for these decisions.
They remain private local content and are never exported as telemetry.
Adding fields requires consumers constructing or exhaustively destructuring
`LogRecord` to update their patterns; existing event variants and checkpoints stay unchanged.

A line that starts with `{` and does not parse yields a record with time `Malformed` and `Unsupported`. A caller that holds only complete lines treats it as unknown; a caller that may hold a partial last line waits for its newline.

| `RecordTime` | Native `timestamp` | `observed(now)` |
|---|---|---|
| `Absent` | field missing | `Ok(None)`: the caller chooses its own lower bound |
| `Null` | JSON null | `Err(TimeFault::Null)` |
| `Malformed` | another JSON type, or a string that is not a real UTC `YYYY-MM-DDThh:mm:ss[.fff]Z` time after the epoch | `Err(TimeFault::Malformed)` |
| `At(unix_ms)` | a valid time | `Ok(Some(unix_s))`, or `Err(TimeFault::Future)` when after the caller's `now` in Unix seconds |

The caller supplies the clock, so a future check samples time after its read. A faulted record proves neither work nor a boundary; the caller decides how long the fault stays. `TurnStart` and `TurnEnd` are the only events that clear pending tools. `Activity` never counts as supported progress, so a log of only unknown records stays unknown. A caller record, such as a progress or rule marker the caller wrote into the log, reads `Activity`; the caller finds it by `line`.

### Caller launch data

M12.12 lets a caller launch through the trait when its host configuration holds native data. A launch that sets none of these keeps its native arguments unchanged.

- `Launch.host_args` (`host-args`, Codex turns only): operator-configured arguments, passed verbatim after the mapped arguments. They refuse a non-default `Policy` with `host-args require the native policy on <name>`, because they could override any part of it. Another runtime or mode refuses with `host-args unsupported by <name>`.
- `Launch.settings_env` (`settings-env`, CLI turn and background modes): environment of the session, as text.
  - Claude: one `--settings` JSON `env`, joined with `Launch.compact_hook` and the effective child's `TASKQ_TASK` and `AGENTQ_LAUNCHER` (including inherited values), which win over a caller value. No optimization value is injected or resolved from installation state. Subscription-only policy clears `ANTHROPIC_API_KEY` and `ANTHROPIC_AUTH_TOKEN` in both child and settings after caller values. A background session runs under the Claude daemon, whose environment is not the launcher child's; settings `env` reaches it and survives resume. Other settings values do not join the child environment, so `Launch.env` stays the place for child variables.
  - Codex: child environment after `Launch.env`, so a settings value wins.
  - The values of a Claude launch enter argv, so a caller never puts secrets here. Secrets stay in `Launch.env`.
  - Antigravity, queue and protected app-server launches refuse with `settings-env unsupported by <name>`; an ordinary Codex turn forwards this environment to its child in either native form.
- `Runtime::launcher_output(run)` (`background`): the raw `Result<Output, Failure>` of the launcher exchange that `background_launch` returns, handed over once so the caller can record it and tell a refusal from an uncertain response with `launch_refusal`. A run without a launcher reads cause `no-launcher`. A second call and a recovered run read cause `launcher-output-unavailable`, because the output is not part of the checkpoint. A foreign run reads `foreign-run`. The run's own state does not change: an unobserved job stays `unknown` and never resubmits.

### Turn recovery and process setup

A consumer service that re-executes itself reattaches its live CLI turn children through `checkpoint` and `recover`:

- `checkpoint` of a CLI turn holds schema `agentq-run/1`, kind `turn`, the child's native process identity (`coreq::process::identity`: PID plus creation time), the log path, the turn's start offset and start time, the resumed session, the cancel/timeout flags and the launch scope hash (arguments, environment and home). It returns cause `turn-ended` once the exit was observed, `identity-unknown` when the native identity cannot be read, and `no-checkpoint` for an `agentapi` submission, whose receipt pipe cannot outlive its parent.
- `recover(saved, &Launch)` with the same launch fields rebuilds an observation-only run: no process starts and no input is sent. A changed scope, runtime or log refuses. `poll` rereads the log from the saved start offset for the turn-end event and checks the identity; a different or missing incarnation means the child exited. On Unix a child that is still the caller's own (a re-executed parent keeps its PID) is reaped with `waitpid` and its exit code read. An exit it did not observe reads `exit` unknown, so a turn without an end event reads `unknown`, never `failed` or `ok`.
- `cancel`, a deadline and a lingering child after its turn-end event stop the recovered child only through `coreq::process::terminate` with the saved identity, so a reused PID is never signalled. `id` stays the child's PID; `progress`, `session_id`, `report` and `record` work as for the original run.
- The original child keeps running when its parent exits: its stdout and stderr append to the log file, not to a pipe of the parent.

`Launch.process` configures the child process of a CLI turn and of an app-server session:

- `configure(&mut Command)` runs last, after the adapter's working directory, environment and `quiet` (Windows `CREATE_NO_WINDOW`) setup, and directly before spawn. Windows creation flags set there replace the adapter's, so a caller that keeps a hidden window includes `CREATE_NO_WINDOW` itself. Unix `process_group`, `pre_exec` and similar settings apply unchanged.
- `spawned(&Child)` runs after spawn and before any input is written, for example to assign a Windows Job Object through the child handle. An error kills and reaps the child and fails the launch with that error.
- Background, queue, job list and stop launchers are bounded exchanges owned and reaped by `coreq::process`; they get only `quiet` and no caller hooks, because the native background job they start is not their child.

## Live worker control

`Runtime::respond`, `steer`, `compact` and `set_option` are typed native worker operations. `Launch.controls` declares the need for a live control channel; the adapter selects the transport. It is false by default. Each operation requires its descriptor capability and a compatible owned live transport; unsupported runtime, transport, option or state refuses with a named cause before submission. Unknown capabilities grant no authority. An ordinary launch gains no terminal prompt or implicit approval handler.

| Operation | Native boundary |
|---|---|
| `respond` | Answer the original pending approval or question with an offered choice, typed question answers or rejection. It cannot submit a new turn or change a grant. |
| `steer` | Submit ordered input to the known active native turn, preserving its session and expected turn identity. It never interrupts or creates a replacement turn. A native queue is reported as queued; an unavailable steering path refuses. |
| `compact` | Request native compaction only when the native session permits it. An acknowledgment is not a completed compaction or a restored context. |
| `set_option` | Request a supported model or native session option. Native validation, the original account and permission policy remain binding. An acknowledgment is not proof that a later model call used the requested model. |

Codex control uses its owned app-server, including `turn/steer` with the exact active turn and `thread/compact/start`. Claude selects `--input-format stream-json --permission-prompt-tool stdio` before spawn. Its model operation uses `set_model`; permission-mode changes are not an added grant path. Idle compaction submits the native `/compact` command and still requires native result/context evidence. Claude steering remains unsupported until its injection can be qualified against the original active turn; an unfenced user message is not submitted as a steer. Claude control uses its native stream-JSON control channel and user-message injection only where the native protocol supports the requested operation. ACP reuses its existing request, offered-choice and model/mode methods; unproven steering, compaction and provider-specific operations refuse. Background jobs, desktop queue submissions and one-shot CLI transports have no added reply channel. The runtime never silently changes transport or restarts a live worker to make an operation available.

`Event::Request` retains its opaque `id`, typed `kind`, optional `prompt` and offered `choices`, and adds `questions`. Each `Question` has an opaque `id`, display `prompt`, offered `choices`, and explicit `multiple`/`free_text` flags. `Control::Reply` keeps `id` and an offered `option` (none rejects/cancels); `Control::Answers` maps the same opaque request ID to question IDs and ordered typed `Answer::Choice` or `Answer::Text` values. Choice identifiers are returned unchanged; text is accepted only for a question that permits it. The adapter binds native identifiers, or runtime-issued identifiers where the native protocol has none, without asking the product to parse native fields. ACP retains its offered-choice reply; structured answers refuse until its native protocol supplies that shape.

The runtime-authored control journal has `type: agentq_control`, `session_id`, original `run_id`, numeric `generation: u64`, original JSON `native_id`, nullable `turn_id`, a full serialized `Event::Request` in `event`, `state` and original JSON in `native`. Generation identifies one occurrence and stays unchanged across revisions; it is never defaulted from missing or malformed data. States are exactly `pending`, `reply-pending`, `reply-sent`, `cancelled` and `unknown`. They distinguish reply intent from successful pipe receipt; neither claims native application. The journal is private session content in the caller log, returned by `transcript` for controlled sessions. Its physical record is source evidence; an unproven link to a native physical record remains unknown.

Approvals and questions use the shared neutral request and choice data in progress and history. A native request ID is observation data, not reply authority: the live controller binds each occurrence to its original run, generation, session and native turn. Reuse of the same native ID cannot make a stale response valid. Foreign, duplicate, cancelled, late and uncertain-delivery responses refuse. A response is validated against the exact pending request and the original launch policy; no absent reply, unknown choice or unreadable request means approval. Native resolution, cancellation and turn completion retire the associated pending reply authority. A resolution must match the original live registry, native session and request occurrence; malformed or foreign notifications grant no retirement authority. Pending requests remain visible as waiting for their owner or end under the launch policy.

The controller records the request observation before exposing its reply handle and records the exact native reply as intent before a native write. The unchanged journal `native` field retains the original native request on observation and the native response on reply-intent/delivery revisions. It distinguishes intent, successful pipe delivery, native acceptance and observed application. An ambiguous journal or transport write fences the original occurrence; neither automatic retry nor recovery resubmits it. Recovery retains observation only and cannot reconstruct a native permission callback. Controlled Claude recovery reads the original caller log at its saved turn and progress boundaries; it retains the durable writer fence and has no native input, reply, model, compaction or cancellation transport. Without a native result, recovered Claude execution reads unknown. Live progress honors the requested text deltas, resets message deduplication at each new turn, and checkpoints only the cursor of events already delivered; recovery never replays delivered text or suppresses another turn because native message IDs repeat. Native results and process absence remain distinct. Control data and answers are private session content, never telemetry.

A controlled session retains its mandatory SessionStore owner while it can write native input, including between turns. The durable controlled-writer binding is established before the first turn; known turn completion retains a distinct controlled-writer phase; it does not make that live writer idle for adoption. Recovery cannot release it from a prior turn report; explicit adoption still requires original native process/scope absence. Every mutation, including an automatic callback rejection, rechecks the original durable authority and native request scope. The retained owner validates the exact current durable row within its existing mutex/file-lock mutation boundary before binding, writer retention, input admission, native identity/scope updates or terminal/stop saves. Cached state never repairs or replaces a foreign, missing, corrupt or changed owner row. Prepared-owner rollback/removal and partially written binding destinations obey that same authority boundary. Pending-session adoption rereads and validates its selected durable row after acquiring that row's existing lock, before scope, unresolved-stop or native-writer admission decisions; a pre-lock sample grants no admission. A positively associated native interrupted-turn terminal settles that turn while retaining the live controlled writer; an unknown signal or termination does not. Native settlement, process lifetime, retained scope cleanup and stop identity remain distinct. A protected-host session continues to use its existing host verification and no-replay guards; ordinary control methods cannot send through that host's transport or acquire its authority.

No new runtime timeout, cap, retry, queue bound or compaction budget is added. Scripted protocols qualify adapter behavior only. Versioned native approval, steer, model and compaction evidence is separate; missing native or platform evidence remains unqualified.

### Selected app-server archive source

The runtime retains complete original inbound line bytes with the native app-server frame. Its existing session/turn observer alone selects capture records, including early notifications only after matching native acceptance. The selected artifact uses `codex-app-server-selected/1` and representation `native-jsonl-inbound-selection/1`; its physical ranges describe that artifact, not full stdout. Foreign, old-turn, global, unbound, malformed and unsupported frames and stderr remain private diagnostics and are never fallback upload sources. Value-only, protected and recovered observations cannot manufacture original bytes. Capture failures degrade only optional archive evidence; protocol, control, ownership, report and Job behavior stay unchanged. The caller-only history format grants no native discovery, resume or reply authority.

## Worker preflight

Binding detail of M12.15–M12.17 and PRODUCT R14. `ready` and `quota` remain available as individual read-only probes. `ready_for`, `catalogue_for` and `account_limits_for` bind reads to the same `Launch` executable, wrapper, home and environment.

`Runtime::preflight(&Launch, Repair)` returns one `Readiness`: a list of checks in the order below, each with state, cause, native detail and next step. `launch` and `resume` run it first, without repair. `agentq agent preflight` exposes it to non-Rust callers. Resume also verifies the session's account, home and runtime against the saved run.

| Order | Check | Passes when | Blocked cause |
|---|---|---|---|
| 1 | Client | Executable resolves and version meets the descriptor `minimum`; ACP releases with known workspace-write defects also require a verified safe version | `client-missing`, `client-outdated`, `client-version-refused`, `client-version-unverified` |
| 2 | Account | The requested home exists and belongs to this account; native login status reads ok. There is never a fallback to another account | `account-home-missing`, `login-required` |
| 3 | Capacity | `quota` picks an account, or the caller set no accounts | `quota-exhausted`, with the hold end |
| 4 | Workspace | Root is an existing folder; writable when `Policy` needs writes; native effective trust covers the requested path, including its ancestor and worktree rules | `workspace-missing`, `workspace-unwritable`, `workspace-untrusted` |
| 5 | Permissions | Every `Policy` part maps to this runtime; the native sandbox starts on this host when one is required | `policy <part> unsupported by <name>`, `sandbox-unavailable` |
| 6 | Inputs | Model is in the catalogue when the catalogue is readable; MCP config, instructions and images are readable | `model-unavailable`, `input-invalid` |
| 7 | Storage | Caller log path and CoreQ store are writable | `storage-unavailable` |
| 8 | Evidence | AgentQ hooks are registered and trusted; telemetry store and archive shipper are reachable. Optional: this check only ever reads `degraded` | none |

### Check states

| State | Meaning | Launch |
|---|---|---|
| `ok` | Verified by a native read in this preflight | Continues |
| `repaired` | Failed, then fixed by an M12.16 repair and verified again | Continues |
| `degraded` | Work is possible; some evidence or optional behavior is lost | Continues; visible in the report |
| `unknown` | The check could not read an answer, for example a probe timeout or a client with no status command | Continues by default; refuses when the caller set `Launch.readiness` to `strict` |
| `blocked` | The worker cannot do this work | Refuses before spawn |

A `Readiness` is part of the run's report, and its states and cause codes join `agent.turn` as metadata. Native detail stays local (M6.2). `ok` is never inferred from a missing check.

`Launch.readiness` is `Continue` by default or `Strict`. `Launch.store`, when supplied, joins the caller log storage check. The selected command, wrapper arguments, account home and child environment are used for the probes; no version result is cached. A missing explicit home or a conflicting home/environment selection refuses rather than falling back.

`Run::readiness`, its final report and its checkpoint retain local check details. The frozen `agent.turn` carries fixed `counts` keys `preflight.<check>.<state>[.<cause>]`, each with value 1; native errors, paths and next-step text stay local. `Observation.phase` reads `starting` until native evidence, then `started`; the existing poll state remains compatible. `starting_ms` is the run's observed age.

Inherited session markers are removed. With an explicit account home, inherited `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `CLAUDE_CODE_OAUTH_TOKEN`, `CLAUDE_CODE_OAUTH_REFRESH_TOKEN`, `OPENAI_API_KEY`, `CODEX_API_KEY`, `GOOGLE_API_KEY`, `GEMINI_API_KEY` and `GOOGLE_APPLICATION_CREDENTIALS` are removed across provider families; explicit caller values survive unless subscription policy clears them. Child Git clears inherited askpass programs and disables terminal and credential-manager prompts by default. Explicit caller environment values remain authoritative. SSH defaults to `ssh -oBatchMode=yes` unless the caller supplied `GIT_SSH_COMMAND`. Recognizable secrets in native arguments and hook-trust bypass flags refuse before spawn.

Codex probes native sandbox startup by running a no-op command through `codex sandbox -P :read-only --include-managed-config`, with no model call or writes. This proves host sandbox startup; policy mapping is checked separately. Windows elevated setup is never run by preflight; the caller must select unelevated mode for a read-only probe. A mapped sandbox policy whose host startup cannot be natively probed remains `unknown` (`sandbox-unverified`). A failed catalogue probe remains unknown. These are not native qualification claims.

### Cost

Preflight makes no model call and costs no tokens. It adds no new timeout, cache or budget: each check runs once per launch, and native probes keep their existing limits (`--version` 5 s, login 10 s). A probe that ends without an answer reads `unknown`; the mandatory ACP safe-version guard maps an unverified version to a blocked client check before spawn. A check is read-only unless `Repair` allows it. Limits and caches join only when telemetry shows a real problem.

### Repair

`Repair` lists what the caller authorizes; the default is none. Allowed repairs:

- Exact-path client trust: Claude `hasTrustDialogAccepted`, Codex project `trust_level`. Written under the client's own lock, keeping unrelated keys. Never for a home folder, an ancestor of the OS or selected account home, or a filesystem root.
- AgentQ's own hook trust: the hash comes from the client's own hook listing; a computed hash is only a fallback. Foreign hooks are never trusted. `--dangerously-bypass-hook-trust` is used only for an AgentQ-owned isolated home.

`Repair { workspace, hooks, journal }` grants preparation only; both flags default to false. A write requires the caller's existing ownership journal. A caller sharing an installation journal holds its lifecycle lock; the CLI and onboarding do so. Claude uses its literal `.claude.json.lock` directory (or legacy `.config.json.lock`), including the native 10-second mtime staleness rule and preserves foreign JSON fields, file permissions, symlink targets and the native custom-OAuth config selection. Codex uses `config/batchWrite` with the user-layer revision, under its native configuration lock. Journaled native values restore only when the current value still matches; changed values remain conflicts.

On Windows, Codex hooks use a bare executable path when safe, or a PowerShell encoded command when the path needs quoting. Onboarding migrates only an exact unchanged journal-owned hook value and preserves foreign hooks.

Codex hook hashes come only from `hooks/list`. Trust writes require an exact registered command and an ownership-journal receipt for that hook source and event; a command suffix alone grants no authority. An explicitly disabled hook stays disabled. Onboarding requests only owned hook trust. Passive profile/status/service reads compare native configuration with unchanged journal-owned hook bytes and the recorded native hash, without starting the client; unknown, changed or untrusted evidence carries a next step. Explicit preflight and repair use `hooks/list`. Explicit repair also records an unchanged native trust grant for an owned hook when needed for passive inspection; restoring that receipt changes nothing.

Each repair is journaled before the write, verified by reading back, and restored by uninstall (§ 9). Login, account choice, model, permissions and provider are outside repair. Login belongs to the explicit owner-started flow of [#286](https://github.com/taskqorg/agentq/issues/286).

### No silent wait

- The child gets no prompt path: no terminal on stdin unless the launch owns one, a non-interactive Git credential guard, and approvals that follow `Policy`. A request that needs an answer reaches the caller as a typed step, never a hidden wait.
- The child environment drops inherited auth and session variables that do not belong to its account; secrets go in `Launch.env`, never in argv. Session variables for all registered runtimes are removed. With an explicitly selected account home, inherited Anthropic, OpenAI and Google API/OAuth environment credentials are removed before explicit caller values are applied. Default-account authentication remains native. Git uses `GIT_TERMINAL_PROMPT=0`, `GCM_INTERACTIVE=never` and `SSH_ASKPASS_REQUIRE=never`.
- A run reads `starting` until native proof of a session: a session ID, or the first native record. Then it reads `started`. A native refusal ends it with its `launch_refusal` cause. `starting` and its age stay visible, and the caller may cancel through identity-safe stop.
- After input is written, nothing resubmits it. Inactivity after start is the stall signal of [#287](https://github.com/taskqorg/agentq/issues/287).

### Native evidence

A fake client proves only the adapter. Each check needs one real failing case and one real passing case per runtime and OS, with client versions recorded (VERIFICATION A7). Antigravity has no login status command, so its account check reads `unknown` until a native read exists; this is reported, never `ok`.

## Session ownership and stall observation

Every launch and resume uses the caller-owned `Launch.session_store`; there is no compatibility bypass. `Launch::new` places it beside the caller log by default, and callers sharing native sessions must select the same store. The store binds the native session to runtime, canonical project and account home, a run token, the controller's native creation identity and the observed native worker identity. It is private durable caller data, never telemetry or native configuration.

ACP and JSONL RPC acquire this same owner before spawn, bind the native session returned by new/load or get_state before prompt input, and retain the handle on the run; checked native settlement alone permits finish. Structured session IDs remain opaque, including path-shaped and leading-hyphen IDs, in hashed store keys. Ownership of an opaque ID does not authorize raw CLI use: every raw `NativeSpec` argument or process seam separately requires ASCII letters, digits or hyphens with no leading hyphen, before preflight or spawn; the single `-` sentinel is allowed only for a new start, never a resume. Terminal history resume applies the same CLI ID guard before preflight; structured history reads retain opaque IDs. Persisted stop IDs retain their existing validation. Structured process hooks compose with the owner hook and Windows Job Object. ACP and RPC do not declare `process-lifetime`; `StopWithOwner` refuses before preflight/spawn. Their stateful server ownership and existing Job Object cleanup do not qualify the canonical recoverable or owner-death policies.

A writer holds an OS advisory lock and journals admission before input. Unknown native session IDs hold a private pending record and OS lock until native evidence binds the ID; unrelated new sessions remain independently admissible. Resume of an untracked session refuses with `session-owner-unknown`; a second writer refuses with `session-busy`. Unknown process identity refuses with `session-owner-unknown`. A reused PID is distinct from the recorded native process and never authorizes a signal. An exited process without a known native completion leaves ownership unknown until explicit adoption; launcher exit or a caller cancellation alone never proves that no native writer remains. Recovery checks the original token and binding and acquires exclusive control without resubmitting input; it cannot coexist with the original controller. Existing protected-host permits and no-replay checks remain required.

`Runtime::adopt(session, launch, known_native)` is explicit import authority. It refuses an existing live writer and checks each supplied native process creation identity; a live matching process refuses. When native writers cannot be enumerated, adoption records `native-writer-unverifiable` and returns that uncertainty to the caller, rather than claiming native absence. The store then arbitrates cooperating runtime writers. The #282 resume command adopts before its first resume. Non-Rust callers use `agentq agent adopt` with the launch binding, `session` and optional `known_native` process identities; it returns the same owner observation and never starts a client. `agent turn` accepts the same `session_store` path. Native submission seams take the same explicit `SessionOwner` handle and validate its runtime, project, account and session before input. Public inspection returns native argument data, never a spawnable command. A discovered app-server thread binds before input; explicit adoption refuses while a matching pending admission has a live or unverifiable local writer. Failed ownership writes and unknown completion retain the fence, including repeated calls through the same handle. Recovery validates and retires a pending alias only while holding admission, target and alias locks for the same binding and token.

`Launch.inactivity`, when set by the caller, is an observation window with no default. It adds no automatic timeout. `Observation.activity` is `active`, `stalled`, `unknown` or `disabled`, with a reason and observed idle age; a transition to stalled emits one `Event::Stalled`. Native activity resets the signal. A tool with a native declared timeout is exempt until that timeout; an active tool with no timeout reads unknown. No native activity evidence also reads unknown. The default only reports; `Launch.stop_on_stall` explicitly requests existing identity-safe cancellation. Observation uses a separate native-record cursor, so observing does not consume caller events. Checkpoints retain ownership and activity state. Malformed complete records and non-object JSON make activity unknown until a native turn boundary; blank separators are valid. Explicit stall cancellation keeps ownership unknown and releases no capacity or permit on the request. Managed process termination uses the original owned group or Job Object; caller-owned and external scopes retain their existing cancellation path and remain unverified. Native app-server turn interruption remains distinct from process termination. A separate durable unresolved-stop fence survives finish, drop, recovery and binding aliases; generic adoption refuses even after the direct parent disappears. Only reconciliation of the original authorized cleanup scope may release that fence. Original-scope cleanup follows the process lifetime policy below.

## Account sign-in

M12.23 applies only to an explicit owner-started native agent account sign-in. It does not authorize GitHub authentication, unattended sign-in, account rotation by a scheduler, or login during launch/preflight/repair. A native sign-in may clear or revoke the selected account's prior credentials before publishing its link; the caller must present that native account operation to its owner. AgentQ never restores, copies or deletes credentials to undo it.

`Runtime::login_start(&LoginOptions)` returns an opaque `LoginSession`; `login_status`, `login_code` and `login_cancel` operate only on that runtime's session. `LoginOptions` selects the native executable, child environment, optional account home and `LoginFlow` (`Browser` or `Device`). Windows sign-in runs a native executable directly: the known Claude npm shim resolves only to its installed native package entrypoint; other command wrappers refuse and require an explicit native executable. The descriptor names supported sign-in flows. An unsupported flow refuses before spawn. This account operation cannot submit worker input or replace a worker owner, grant, admission, session or home.

The selected home must already exist and be owned by the current OS account. Missing, foreign, changed or unreadable home identity refuses; no other account is tried. Explicit homes keep their literal native spelling, including the client's Keychain identity. With no explicit home override the native default is not pinned into a home environment variable. Inherited provider credentials and enclosing session markers are removed; sign-in never accepts tokens through argv or treats another provider's environment as this account's login. Each home has one runtime sign-in writer, with durable admission before native sign-in. Its guard survives a dropped handle until the original native process and transport cleanup finish; an unfinished claim after controller loss or uncertain termination refuses another sign-in, without automatic takeover.

`LoginStatus` separates starting, waiting for the owner, code submission, authenticated, cancelled, failed and unknown. A prompt contains only the native provider's validated sign-in URL, its device code when reported, and whether a pasted code is required. It is private relay data, redacted from debug output and absent from telemetry and logs. Raw CLI output, pasted codes and credential bytes never leave the adapter. stdout and stderr are scanned separately, including split native output; a partial link is never published. A submitted pasted code crosses stdin once, never argv, and is never replayed or automatically retried. `CodeSubmitted` proves the pipe write, not native acceptance. A device code is entered at the native URL, not sent to a CLI that has no code-input protocol.

Claude uses its native `auth login --claudeai` pipe flow and printed pasted-code link. Codex uses its native app-server browser-login exchange (no browser launch on the execution host) or explicit CLI device-auth flow. Codex browser sign-in shares the native registered callback address: an occupied address refuses before login submission, and `LoginOptions.callback_authority` selects an existing current-user-owned authority directory shared by cooperating runtime processes across accounts/projects. Browser sign-in requires it and journals the actual native callback resource before spawn; other modes do not read it. Its OS lock and original claim survive a dropped handle until the native process and transport finish. A claim left unfinished after controller loss or uncertain cleanup refuses another flow, without timeout or automatic takeover. A later noncooperating external CLI claimant remains an unavoidable check-to-native-bind race, not cleanup authority. Opening a browser on the execution host is suppressed; the caller chooses where to present the link. Antigravity refuses until its native login protocol is proven. Provider redirects, callbacks and completed authentication remain native responsibilities. A link, a code, changed bytes or process exit alone is not authenticated proof: success requires the native completion result and a matching native account-status read in the same home. Unsupported or unreadable evidence stays unknown. An unreadable initial credential snapshot refuses the native sign-in before it can change credentials. Codex reads its effective credential-store configuration; only the native file store currently has a verified cancellation guard. Keyring, auto, ephemeral and unreadable stores refuse sign-in without changing the backend or trying a fallback.

Cancel rechecks the original home and native credential evidence before requesting termination of the original owned login process. If credential bytes changed, credentials disappeared during rotation, the native client reported completed sign-in, or the evidence is unreadable, cancellation refuses with its reason; it cannot roll back an authenticated home. A stop request is not exit proof. A credential change or unreadable evidence during an accepted stop reads unknown, rather than cancelled. `LoginStatus.exited` records the native wait result; `finished` requires the native process and pipe/helper cleanup to finish. Status distinguishes a requested stop from an observed native process exit and makes no server-side revocation or subtree-completion claim. Dropping a handle uses that same guard and does not release an unfinished sign-in's writer authority.

No default timeout, polling deadline, retry, cache or credential-rotation grace is added. Native expiry/denial/error states are named; a caller can explicitly cancel while the guard permits it. Existing workers keep their original home and ownership. Changed or uncertain account evidence reports restart required or unknown, rather than moving a running worker to another account; the caller owns any restart. A credential briefly missing remains unknown until a later explicit read can prove its state.

Qualification uses temporary native homes only, offline scripted pipes and isolated native URL/cancel checks. Real account sign-in and completed-login qualification require the owner; a captured URL and cancelled process do not qualify completed sign-in.

## Process lifetime and orphan inventory

An unfinished pending admission checks its original managed scope before import, even when its native leader exited before session discovery. Unknown scope absence refuses adoption. A known native terminal report settles session admission separately from process scope cleanup. Before a later admission or adoption replaces a completed run's ownership row, its token-bound native scope and original cleanup authority are durably retained in the orphan inventory; a live or unknown descendant scope is never silently retired. Recovery and cancellation retain the original owner handle and journal a stop before any managed signal.

`Launch.lifetime` is `Recoverable` by default: loss of the launching OS process does not stop the native worker, and existing same-child recovery remains observation-only. `StopWithOwner` explicitly stops the owned process group when that OS process dies. Dropping a `Run` is not owner death. The original launching process remains the lifetime owner after recovery; a different recovery policy refuses. The selected policy and native scope identities are durable ownership data, separate from execution health.

Managed CLI and app-server children enter a private process scope before input. Raw caller-built Commands and caller Process hooks retain their own setup under Recoverable and expose an unverified scope; StopWithOwner refuses those combinations before spawn rather than replacing caller flags, groups or Job Objects. On macOS and Linux a small native guardian pins the process group and watches the original owner's kernel process identity and the native leader's exit. Its control/report descriptors are closed in the executed client and its descendants. `StopWithOwner` stops the group on owner death or leader exit; `Recoverable` leaves native work running on owner death. Windows uses a Job Object assigned while the child is suspended; `StopWithOwner` uses kill-on-job-close, with no inherited owner handle. Unsupported native background/desktop or protected-host scopes refuse `StopWithOwner` before submission; existing independent protected-host authority is unchanged. There is no new timeout or termination grace.

`Runtime::orphans(launch)` inventories only that caller's SessionStore and matching runtime, canonical project and account binding. It reports policy, original controller and native creation identities, native scope and their evidence separately. A missing controller is an orphan only when native creation identity still matches; unreadable identities, PID reuse, an unproven group or surviving descendants without their original leader remain unknown. `Runtime::stop_orphan(entry, launch)` rechecks the durable token, binding, controller absence and native scope before signaling. It refuses a live or reused controller identity and a changed/unknown native scope. Adoption of an unfinished managed run also requires absence of its original scope. POSIX termination is requested through the identity-bound guardian, which signals its own pinned group; Windows termination uses the verified native Job Object. A successful stop request is not absence proof: the next inventory supplies readback. A missing Windows Job name alone is unknown because its namespace entry can disappear while descendants live. Explicit Job cleanup retains the original handle through its active-process-zero notification and durable token/scope-bound absence evidence; a crash before that evidence remains fenced and unknown. Runtime::reconcile_stop rechecks the original scope and releases its unresolved-stop fence only after native scope absence is proven; it leaves the native turn outcome unknown and never adopts or submits input. Managed cancellation uses that same owned scope; direct-process-only or unreadable scopes keep their fence. Old ownership records without lifetime evidence refuse; no compatibility path is added. No inventory operation adopts, replays input or changes native credentials.

## Session model

- One native session belongs to one conversation or one task attempt.
- A new start passes a caller-chosen session ID to Claude (`agent::new_session` gives a UUID v4). Codex names its thread in the first `thread.started` event and Antigravity names its conversation in `conversation.started` (or `response.newConversation.conversationId`); `new_session` gives `-` until then.
- A CLI seam owns one client process per turn. ACP uses native `session/new`/`session/load` and the prompt response marks the turn complete independently of process exit. The retained low-level agentapi comparison seam receives a prompt argument and tails `<Gemini root>/antigravity-cli/brain/<session>/.system_generated/logs/transcript.jsonl`. The next turn resumes the same native session ID.
- The client compacts its own context. For Claude, `Launch.compact_hook` becomes `--settings` with a `SessionStart` hook on `compact` that runs the caller's command, which prints the rules again.
- Output of every turn appends to one caller log per session. A turn reads only the bytes it appended.

## Arguments

The caller builds the base command: the client executable from `agent::executable`, or a sandbox wrapper around it. `agent::command` appends the arguments, sets the project root as working directory, removes `CLAUDE_CODE_SESSION_ID`, `CODEX_THREAD_ID` and `ANTIGRAVITY_CONVERSATION_ID`, adds the caller's environment and sets the account home.

| Client | Start | Resume |
|---|---|---|
| Claude | `-p --output-format stream-json --verbose <args> --session-id <id>` | `-p --output-format stream-json --verbose <args> --resume <id>` |
| Codex | `exec --json <args> -C <root> -` | `exec --json <args> resume <thread> -` |
| Antigravity ACP | `agy acp`, then `session/new` and `session/prompt` | `agy acp`, then `session/load` and `session/prompt` |
| Antigravity CLI comparison seam (`agy`) | `--input-format stream-json --output-format stream-json <args>` | `--input-format stream-json --output-format stream-json <args> --conversation <session>` |
| Antigravity sidecar comparison seam (`agentapi`) | `new-conversation <args> <prompt>` | `send-message <args> <session> <prompt>` |

- `<args>` are the caller's client arguments in this position: TaskQ passes `--permission-mode <mode> --settings <json>` to Claude, sandbox arguments to Codex, and native flags only to the retained Antigravity comparison seams; high-level ACP uses model/mode methods and refuses `Launch.title`.
- The adapter resolves a name on `PATH`; on Windows it also tries `<name>.exe` and the npm shim `<name>.cmd`. `user_bin_path` appends `~/.local/bin` when `PATH` lacks it.
- The Codex adapter replaces an npm `codex.js` launcher with its native executor on macOS and Linux, so a kill reaches the client. It never scans other installations.
- Antigravity executable lookup selects `agy` on PATH, also checking the user bin directory; it never selects agentapi or `ANTIGRAVITY_AGENTAPI_EXE`. A caller-selected command is authoritative and must speak ACP. Explicit low-level stream/agentapi seams remain solely for comparison proof and never act as a launch fallback.
- Native `agentapi` submission exit is not turn completion. The runtime tails complete new records from `<home>/brain/<session>/.system_generated/logs/transcript.jsonl`; a resumed turn captures its boundary before submission. Partial records wait, malformed records, file replacement, truncation and changed consumed prefixes fail observation. Each poll reads at most 8 MiB of new bytes from each transcript; partial records and full-history lookup retain progress across polls, including histories or records larger than that budget. Truncated records wait for the corresponding complete `transcript_full.jsonl` step. Session IDs used as locators contain only ASCII letters, digits and hyphens.
- `agy` stream `init` identifies the session, `step_update` supplies per-step usage, and terminal `result` supplies status and final text (`SUCCESS`, `ERROR`, `CANCELED`, `INTERRUPTED`, `INVALID`). `WAITING`, `RUNNING` and unrecognized statuses never prove completion. [Headless protocol](https://antigravity.google/docs/cli/headless). Result usage is cumulative and is never summed as per-turn usage. Missing counters and model remain unknown.
- An existing `agentapi` server does not inherit submission environment. Optimization intent is a compatible no-op for both sidecar and CLI launches; it neither refuses submission nor edits server configuration.
- `kill` and a deadline stop the owned submission process and observation. They do not prove that server-side execution stopped; callers retain native cancellation and capacity ownership until their own readback. A submission without a session or terminal remains unknown. Native platform/account qualification remains open until a real caller-owned turn is observed.


## End of a turn

- A turn ends at a Claude `result` event, a Codex `turn.completed` or `turn.failed` event, or an Antigravity `PLANNER_RESPONSE` step with `status: "DONE"` and no `tool_calls` (or `status: "ERROR"` / `"CANCELLED"`).
- A process that is still running after its turn-end event is stopped: Codex on Windows kept running after its turn ended.
- `Launch.deadline` stops the process at the deadline; the turn then reads `cancelled` with error `timeout`.
- `poll` serves a caller loop that does not block.

## Outcome

`report` classifies the turn in this order; `Report::read` alone has no deadline or exit code:

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

A rejected Claude `rate_limit_event` before work also returns `Report::reached`, preserving its reset. A reached limit returns `Report::reached`: the reported reset in unix seconds, or none. Claude reports it on the first `assistant` event (`apiError: usage_limit_reached` or `quotaLimits.status: rejected`, reset `quotaLimits.resetsAt` or `apiErrorParams.rate_limit_info.resetsAt`). Readable `apiErrorParams.rate_limit_info` name and reset fields fill missing or unreadable `quotaLimits` fields; a fallback name preserves readable data in its matching `unifiedWindows` entry, including the pre-work reset. Codex and Antigravity report it as `error` or `turn.failed` / step error text containing `usage limit` or `quota exceeded`, without a reset. A limit after work started is not a start refusal. Typed `TurnEnd` events keep that same pre-work refusal context across incremental reads and cursor recovery.

`Report.detail` retains a Claude terminal `result.subtype` or Codex failed-turn native `error.message`, including the app-server failure report. These fields do not change `outcome`, `error` or `reached`; runtimes without native detail retain unknown.

`Report.reason` retains the short classified Codex authentication reason (`401 Unauthorized` or `refresh token rejected`) on both exec and app-server paths. `error` remains `auth-rejected`; `detail` keeps the full native failed-turn message. Other reports leave `reason` absent. Products consume these fields without parsing native messages.

`Report.termination` is a typed native-process observation, independent of turn outcome, `exit` and `StopEvent`: `Observed` means AgentQ reaped the actual worker child; `Unknown { reason }` means termination was not observed. `Runtime::report` and the JSON report expose the same value (`state: observed` or `state: unknown` with `reason`). Log-only classification and native background/app-server session reports remain unknown because a terminal record is not process-termination evidence.

On Unix, a child wait or owned-child `waitpid` confirms termination even when a signal supplies no numeric exit code. On Windows, a completed child wait confirms termination. AgentQ cleanup after a native turn end also confirms termination but leaves `Report.exit` absent and keeps the native turn outcome; it creates no stopped-turn failure. A recovered missing or replaced process identity stays unknown with reason `recovered-process-exit-not-observed`, even when the old incarnation is gone. A sidecar submitter exit never proves termination of its separately owned native server or turn, and stays unknown with reason `sidecar-process-not-observed`. Neither observation authorizes signalling a process or proves descendant/server-side cleanup. Consumers constructing `Report` add `reason` and `termination`; these observations do not change run checkpoints or `stop_id`.

For an unfinished Claude turn, products read the latest ordered `Event::Message` from `log_records` for last-assistant evidence; `Report.text` remains final-result text. A message proves neither a terminal boundary nor success. Antigravity tool input evidence requires the native `step_update.state` (`ACTIVE` gives `ToolStart`; `DONE` gives `ToolEnd`); missing state gives `Unsupported`, never acceptance. `Activity` also proves no acceptance.


## Usage

- Claude: each `assistant` message ID counts once, because one message spans several lines. Uncached input is `input_tokens`, cache read `cache_read_input_tokens`, cache write `cache_creation_input_tokens`, output `output_tokens`. The model is the one of most calls.
- Codex: each `turn.completed` counts. Its `input_tokens` include `cached_input_tokens`, so uncached input is the difference. Codex reports no cache writes (0), no call count and no model in `exec --json` output.
- Antigravity CLI: per-step `usage` of completed stream `step_update` records counts once by `step_index`; terminal result counters are cumulative and are ignored. No model-call count is inferred. Antigravity sidecar: each completed `PLANNER_RESPONSE` step counts once, using the latest record for its `step_index`. Uncached input is `input_tokens`, cache read `cache_read_tokens`, cache write `cache_write_tokens`, output `output_tokens`, and model is `model`. Missing counters stay unknown; no usage or model is inferred from the requested model. Native counter formats need platform qualification.
- A counted call without a value makes that sum unknown, never smaller. No counted call: every count is null with reason `no-usage`.
- A model ID is a code of at most 64 characters from `[A-Za-z0-9._:-]`; any other value is not reported.

`Report.live_background` lists, oldest first, one label per job that Claude log evidence leaves live; `Report.background_job` states that the list is not empty. Other runtimes report an empty list and false because these Claude job records are absent. This is log evidence only, never process, stop or cleanup authority.

- **Start.** A tool-use record alone is unconfirmed. A Bash `run_in_background` or Monitor job needs a non-error result that names its native task, in `backgroundTaskId`, `taskId`, `task_id` or `shellId`, or in the text `with ID: <id>` or `(task <id>`. A foreground Bash call becomes a job only through a structured `backgroundTaskId`, so its output cannot start one. An error result means the job never ran.
- **End.** A terminal notification ends the job matching its task ID or tool-use ID, read from `queue-operation` content, `queued_command` attachments, `system` `task_notification` records and user text. Terminal statuses are `completed`, `failed`, `killed`, `stopped`, `cancelled`, `error`, `timeout` and `expired`. Notification-like text in tool output, prompt snapshots or request records closes nothing.
- **Stop.** A TaskStop, KillShell or KillBash call naming a job takes it out of the live list at once, because a stopping job gives a caller nothing to continue. A failed stop result restores it; a successful one ends it.
- **Turn boundary.** A `system` `init` record clears earlier jobs, because the list describes what the last turn left running. A caller that marks its own turns in the log passes only the records after its last marker.
- **Label.** Each job uses the fixed tool name `Bash` or `Monitor` (A1). Descriptions are untrusted and may copy private argument fragments, so they stay inside the adapter with commands, arguments and task IDs. Equal labels still occupy separate list entries.

## Limit snapshot

| Client | Source | Used percent | Reset | Window |
|---|---|---|---|---|
| Claude | last `rate_limit_event` of the turn | `rate_limit_info.utilization` (0-1) × 100 | `rate_limit_info.resetsAt` | `rate_limit_info.rateLimitType` (`five_hour`, `seven_day`) |
| Codex | last `rate_limits` of the turn output, else the last `token_count` in the thread rollout `<home>/sessions/<yyyy>/<mm>/<dd>/rollout-*-<thread>.jsonl` | `primary`/`secondary` `used_percent` (0-100) of the window with higher use | that window's `resets_at` (or now + `resets_in_seconds`) | that window's `window_minutes` as `<n>m` |

- A `Window` holds `window` (length as `<n>m`, or the native type), `used` (fraction), `resets_at`, `event_id`, `exhausted`, `rejected` and `unknown`; `rejected` independently records native admission status (true for `rejected`, false for `allowed`/`allowed_warning`, otherwise null), never inferred from utilization. In Claude `unifiedWindows`, the named top-level rejection identifies the binding window and marks siblings false unless they explicitly report their own status. Without that binding evidence sibling rejection stays unknown. Existing serialized windows without `rejected` read null; `exhausted` is true on explicit rejection or utilization at least one, false on an explicit allowed status or lower utilization, and otherwise null. Claude rejected `rate_limit_event`, `quotaLimits` and `apiErrorParams.rate_limit_info` retain the native `rateLimitType` and reset without inventing utilization. Missing fields stay null with a reason. Claude independent window updates retain the last reading of each window. Codex retains the latest snapshot of each native `limit_id` / `limitId` group in logs, rollout fallback and app-server state, including recovery. Equal window lengths in distinct groups stay separate; a partial update replaces that group's old values with unknowns. Terminal app-server reports stay frozen while later updates feed the next turn.
- Account reads use a bounded native session for Codex; Claude OAuth `api/oauth/usage`, including structured `limits` and model scopes; and Antigravity Code Assist `loadCodeAssist` then `retrieveUserQuotaSummary`, retaining every model bucket. Consumer accounts use the daily endpoint; GCP terms select the production endpoint.
- Claude credentials come from its native Keychain item on macOS, otherwise `.credentials.json` in its home. Antigravity uses the native `gemini` Keychain item or its home token file, with the default-home OAuth file fallback. An explicit home never falls back to another account or to Antigravity's default Keychain identity. A Windows login held only in the native credential store reads unknown without a token file. Reads are guarded before and after; expired, denied, changed or missing credentials return unknown. Keychain reads that would prompt stop and latch for this process. Tokens travel only on curl stdin. Neither API refreshes credentials.
- Native API shape failures preserve readable sibling windows and mark unreadable windows unknown; they never become zero. Null Claude legacy windows and null scope dimensions are omitted; malformed non-null windows or scope dimensions remain unknown. Claude extra usage credits are not quota windows.
- `Report.limits` holds every window: Claude `unifiedWindows`, Codex `primary` and `secondary`. `Report.limit` stays the binding window for `agent.turn`: the Claude top-level window, the Codex window with higher use.
- Codex `exec --json` output carries no limit snapshot today; the rollout of the account home does. A rollout snapshot is the latest one of the thread, which can be from an earlier turn.
- Percent is rounded to a whole number.
- Reasons: `not-reported` (no snapshot), `no-rollout` (Codex rollout not found), `partial-snapshot` (percent or reset missing).

## Observation

`Run::record` appends the records of a finished run in one call: one `agent.turn` and one `agent.call` per model request ([specq telemetry contract](https://github.com/taskqorg/specq/blob/main/engineering/profiling.md#telemetry-contract)). A second call after a successful append is refused; a call after a failed append retries the same records. Every record is the shared CoreQ `coreq-observation/1` shape, so a shared field not collected reads null with reason `not-collected`.

- `agent.turn` keys: `product`, `version`, `operation` and `kind` = `agent.turn`, `started_ms`, `duration_us`, `outcome`, `error`, `project`, `host`, `runtime`, `account`, `task`, `attempt`, `route`, `model`, `session_id`, `exit_code`, `user_cpu_us`, `system_cpu_us`, `peak_rss_bytes`, `input_tokens`, `cache_read_tokens`, `cache_write_tokens`, `output_tokens`, `reasoning_tokens`, `limit_used_percent`, `limit_resets_at`, `limit_window`, `unknown`.
- `agent.call` keys: the identity keys above with `operation` and `kind` = `agent.call`, `model`, the five token buckets, `started_ms` (the record's native `timestamp`, else null with `not-reported`), `duration_us` (null with `not-reported`), `outcome`, `error` and `session_id`. `agent.turn` totals and `agent.call` records describe the same tokens: sum one kind, never both.
- `trace` and `span` are null with reason `not-traced`. A task turn has `route` null with reason `task-turn`; a route turn has `task` and `attempt` null with reason `no-task`.
- Caller context ([CoreQ R55](https://github.com/taskqorg/coreq/blob/v0.23.0/coreq.md)): `agent.turn` and `agent.call` read `project`, `task`, `attempt`, `actor` and `entry` from the current CoreQ `TraceContext`, so a product copies no field. An explicit `Context::project` or `Context::task` wins; a `Context::project` of `other` yields to a caller project. `Context::route` stays independent. A value missing from both reads null: `task` and `attempt` with `no-task` (`attempt` with `no-attempt` when a task is known), `actor` and `entry` with `not-collected`. No caller means the record is unchanged, and an unrelated thread or span never lends its context.
- Runtime children: a CLI turn, an app-server session and each bounded native exchange (background, queue, job list, stop, readiness) receive the current context in `COREQ_TRACE_CONTEXT`, and a stale value is replaced or removed. `agentq agent turn` runs no collector, so it treats its own valid `COREQ_TRACE_CONTEXT` as the caller for its records and child; an invalid value stays unknown and never changes the turn.
- The protected host carries CoreQ-validated `COREQ_TRACE_CONTEXT` through its launchd job and clean native environment. A current span wins over inherited context; without a collector, each hop decodes and re-encodes its process context. Missing or invalid context is omitted, and legacy trace-only context remains supported. Caller metadata grants no authority and changes no permit, native identity, input gate or recovery record.
- A failed append is kept in `Report::recorded`; it never changes the turn's result. A repeated record keeps the first payload of each immutable id, so recovery never counts twice and never fails on a value only the first process could measure.
- Every `agent.turn` has an immutable id: `agent.turn` under the CLI turn's key, `agent.native` or `agent.app` under the native turn's key.

### Deferred persistence

A caller that appends outside its supervision path, for example after a CoreQ budget lock wait, keeps the observation of the finished run and appends it later:

```rust
let state = runtime.poll(&mut run)?;      // Takes no store lock. A terminal poll freezes the CLI report.
let frozen = run.snapshot(&context)?;     // `running` before the terminal observation. No store, lock or native call.
let saved = frozen.save();                // Optional durable JSON for a retry after a restart.
frozen.append(&store)?;                   // May wait for the store. An error leaves `frozen` and the run unchanged.
```

- The terminal poll freezes the CLI report once: tokens, calls, outcome, session, rollout values, duration, exit and resource use. `Runtime::report`, `Run::snapshot` and `Run::record` read that value. Later bytes in a shared log or rollout, queue delay, a late cancel and a passing deadline never change it. An unreadable log at that moment freezes nothing: `poll` returns the error and the next poll retries.
- Background, queue and app-server runs freeze the same way from their native terminal report; a snapshot makes no second native read, so a later turn with the same input cannot refuse it.
- The first `snapshot` fixes the records and the caller context of its thread. Take it in the caller's span. A repeated call returns the same value, whatever context it is given; `input` on an app-server run starts a new turn, whose snapshot is new.
- `Frozen` holds only immutable IDs and CoreQ-validated metadata, never a prompt, result text or path. `save` writes schema `agentq-frozen/1`; `restore` refuses any other value as a whole. A `Run` checkpoint carries a taken snapshot.
- Save the snapshot before starting a continuation on the same log. A CLI turn cannot checkpoint after its exit, so a recovered run that first observes the terminal state after a continuation cannot tell its own bytes from later ones. Its `agent.turn` id stays stable; the saved snapshot is what keeps later calls out.
- A failed snapshot or append stays unknown to the caller and is no completion signal: it releases no capacity, permit or native ownership, and the caller retries with the same value.

### Model requests

| Runtime and mode | One `agent.call` per | Reasoning tokens |
|---|---|---|
| Claude turn, background | `assistant` message id | null `not-reported` |
| Codex turn | `token_count` of the turn in the thread rollout, with a distinct cumulative total | `reasoning_output_tokens` |
| Codex queue, app-server | attributable `last` increment with a distinct cumulative total | `reasoning_output_tokens` |
| Antigravity `agy`, sidecar | completed step index | null `not-reported` |

- A Codex `exec` turn without a rollout has only its `turn.completed` totals. It writes one `agent.call` with `source` `turn-aggregate`, so that record may cover several requests; the rollout is the source of per-request records.
- A call or bucket the client did not report reads null with its reason. No model request observed writes no `agent.call`, and the turn's tokens read null with `no-usage`.

### Process fields

| Run | `exit_code` | CPU and `peak_rss_bytes` |
|---|---|---|
| CLI turn | native exit code; Windows reads it as its unsigned DWORD; a signalled child is null `signalled` | sampled |
| Recovered CLI turn | as observed, else null `exit-unknown` | null `recovered` |
| Plain app-server turn | null `session-process` | sampled, per turn inside the session process |
| Protected or recovered app-server turn | null `session-process` | null `not-owned` |
| Antigravity sidecar | submission exit code | null `sidecar-server` |
| Background, queue | null `not-owned` | null `not-owned` |

- One thread per owned process samples its live tree at start and each second, on Linux (`/proc`), macOS (`proc_pidinfo`) and Windows (Toolhelp, `GetProcessTimes`, `K32GetProcessMemoryInfo`), and ends with the root process. No host tool runs. No successful sample reads null `not-sampled`; another platform reads null `unsupported-platform`.
- Peak RSS is the largest sum of one sample. CPU sums the last sampled time of each process that was in the tree.
- `ponytail:` both are sampled, so a process that lives between samples and the CPU after the last sample are missing, and a process reparented out of the tree stops counting. Exit accounting per process replaces this when the measured gap matters.

## Accounts

- `Accounts { list, limits }` holds one client's ordered accounts and their recorded limits. The caller stores both.
- An empty list is one enabled `default` account. A label is 1-32 of `[a-z0-9-]`, not starting with `-`.
- A limit ends at its reported reset, else `PROBE` (1800 s) after it was seen.
- `pick`: the first enabled account whose limit has ended, else the first enabled account; none when all are disabled. `spare`: the account a limit moves to, only with more than one account. `hold`: the earliest live limit end of an enabled account and whether it was reported.
- Homes: `default` keeps the caller's own home (`CLAUDE_CONFIG_DIR` / `CODEX_HOME` / `GEMINI_HOME`, else `~/.claude` / `~/.codex` / `~/.gemini`; a Windows service uses `USERPROFILE`). Any other account is `<home>/<tool>-accounts/<label>`, with `<tool>` the consumer's `Tool.name`. `command` sets the home variable for the child.

## Compatibility sessions

Native CLI turns keep the v0.17 arguments and behavior inside the adapters. `Client`, `Spec`, `Turn`, `NativeTurn`, `BackgroundJob`, `AppSession`, `AppServer`, the background, queue and app-server helpers, the output parsers, `which`, `codex_native`, `claude_settings` and `rollout` are private. `Launch.mode` selects a compatibility mode. Each mode needs the descriptor capability of the same name; otherwise launch refuses with `mode <mode> unsupported by <name>`.

| Mode | Capability | Runtimes | Behavior |
|---|---|---|---|
| `Turn` (default) | `turn` | all | One CLI turn, as in the sections above. |
| `Background` | `background` | Claude | `--bg` launch; `Launch.input` is the caller's protected file reference. `poll` discovers the job from the launcher's job ID (first line `backgrounded · <job>`; or, on resume, the session), then binds the native transcript `<home>/projects/<canonical root without the Windows verbatim prefix, each non-alphanumeric character as ->/<session>.jsonl`. `cancel` sends the stop request for the verified job. |
| Protected app-server | `app-server` | Codex | `ProtectedAppServer::open` only: opens the thread with the caller's `thread` parameters plus the mapped policy (`write` as `sandbox`, `bypass` with write `any` and `deny` as `approvalPolicy: never`; `roots`, `unelevated` and `effort` refuse) and `model`. `input` starts the next turn with native `text` and `localImage` (path) items in the caller's order. `account` sends one `account/read` with `refreshToken: false` and maps `chatgpt` to `subscription` (with `planType`), `apiKey` to `api-key` and no account to `none`. `progress` returns the same events as [one run path](#one-run-path); the host owns server requests. `cancel` interrupts the active turn, else stops the server. `transcript` returns cause `no-transcript` (`Launch.log` holds stderr only). Recovery rebuilds observation only. |
| `Queue { transcript }` | `app-queue` | Codex | `resume` only: queues `Launch.input` (a reference) into the desktop thread and observes `transcript`. `cancel` returns cause `cancel-unsupported`: desktop cancellation stays a caller UI operation. |

In both modes `launch` returns an error only before submission. A launcher or queue response that fails after submission keeps the `Run`; its state reads `unknown` until native evidence arrives, and nothing resubmits. `checkpoint(run)` returns the private recovery value (cause `no-checkpoint` for an `agentapi` submission; CLI turns: Turn recovery and process setup); `recover(saved, &Launch)` rebuilds the run from it and the same launch fields, without a native call. `session_url` returns the observed Remote Control URL or a cause (Session observation). `progress` and `transcript` read the native transcript. `Launch.title` maps to Claude `--name <t> --remote-control <t>`; Codex and Antigravity ACP refuse it. `Launch.deadline` stops a CLI turn at that instant; the run then reads `cancelled` with error `timeout`. A cancelled CLI turn reads `cancelled` with error `interrupted`. Progress cursors and the event mapper state are private checkpoint fields. A resumed CLI turn captures its existing rollout boundary before spawn; recovery preserves it. A legacy resumed checkpoint without that boundary emits no rollout quota/context events, rather than attributing earlier turns to the recovered turn.

`ProtectedAppServer::open(session, &Launch)` opens a verified protected frontend as the same `AppServer` mode run; every write repeats the host authority check. These modes never switch Claude to `-p` or replace an app session with `exec`.

- `background_command` passes `--bg <caller args>` on start and only `--bg --resume <session>` on resume. Start arguments include the caller's settings, name, Remote Control name and permission mode in their original order. The native client assigns the start's session and job IDs. Input is the caller's file reference, not private task text: Claude forwards it to child argv. The caller durably binds and retains that input before submission.
- `background_launch` uses bounded stdin exchange and returns launcher output or failure without interpreting either as completed work. A timeout, lost reply or malformed reply is uncertain; retain the original input and recover without resubmitting. `background_jobs` uses `agents --json --all --cwd <root>` in the same account; a job missing there (Claude's filter misses roots under `.claude/worktrees`) is looked up in the unfiltered `agents --json --all` listing. `BackgroundJob::read` verifies exact canonical root, job and session; readiness means a known native state, not accepted input or completion. Remote Control identity comes only from that session's native `bridge_status` URL.
- `app_queue` submits the caller's file reference with `queue --thread <session> --message <reference>` in the same account. New desktop sessions use the existing normal CLI handshake to obtain a thread before queueing; handshake observations belong to that turn, never to the queued turn. The caller opens/focuses the app and verifies its presence. A failed queue response is uncertain and must not cause another queue call.
- `NativeTurn` observes a caller-selected transcript from a byte boundary captured before submission. Claude requires its exact user input then `system/turn_duration`; Codex desktop requires session metadata for the exact root/thread, `task_started`, the exact user input and matching `task_complete` or `turn_aborted`. Duplicate matching inputs, conflicting terminal results, foreign identities, truncation, replacement, invalid or partial records stay unknown. Each bounded poll continues reading without resubmission.
- `NativeTurn::checkpoint` and `recover` preserve the original input boundary, account, session, timing and observation ID. They contain private locators and input; callers protect and durably store them outside telemetry before submission. A new background launch uses a caller alias and an absent transcript locator; `attach_background` binds the verified job and its native transcript after discovery, retaining the original input. Recovery validates file identity and the read prefix and rebuilds observation only; it grants no stop authority. Each poll reads at most 8 MiB of new records, each record at most 8 MiB; prefix verification has a 5-second bound and failure retains ownership. `record` commits the terminal observation using the saved ID so an acknowledgment lost across restart cannot duplicate it.
- `app_server_arguments` returns inspection data for `app-server <caller args>`. It returns no spawnable command; managed app-server submission uses the owned runtime or protected-host path. `AppSession` supplies initialize/initialized, thread start/resume, turn start, interrupt and raw notification observation. All JSON parameters, including instructions, tools, model, effort, provider and grants, are caller inputs and are preserved. A server request remains with the caller and is never auto-approved. A missing thread cannot silently start another one. A request-correlated native turn ID is required before a matching `turn/completed` ends the turn without ending the server; process/transport exit alone remains unknown. Before that reply, up to 256 notifications / 8 MiB can be retained for correlation; exceeding either bound preserves the uncertain original submission.
- `AppServer` owns the bounded stdio transport (8 MiB per record, 16 queued records). The bound counts bytes and is checked before decoding. A record over the bound and a stream that ends inside a record fail with distinct messages; neither reveals record content. An ordinary turn handles its own protocol, rejects server requests, closes stdin at the terminal event and reaps its child; timeout or EOF without native completion stays unknown. A protected host retains its transport and authority. `ProtectedAppServer::requests(run)` takes its native server requests, and `ProtectedAppServer::reply(run, reply, until)` accepts only replies and repeats the existing authority verification. Typed `progress` carries no server requests or account replies. The protected host alone handles dynamic tools and approvals; no ordinary product protocol API remains.
- Stop requests do not prove exit or completion. `background_stop` targets only a verified job and returns the request response; the caller reads jobs and its owned process identity back before releasing capacity. App-server interrupt targets the exact accepted thread/turn. Desktop native cancellation remains a caller-owned UI/control operation using the native turn ID of the run's `progress` records; the runtime observes its matching abort. No process tree or grant is inferred from these identities.

Compatibility reports reuse `Report`, `Usage`, `Snapshot`, `Context`, `agent.turn` and `agent.call`. Claude usage counts assistant IDs within the matched input's turn. Codex usage adds attributable model-call increments (`last_token_usage` / `last`) once per distinct cumulative snapshot (`total_token_usage` / `total`); it never reports the thread total as turn usage. Missing counters or inconsistent increments make the affected sums unknown. App-server usage/limits normalize native field names; absent values remain null with reasons. Native formats without attributable usage return `no-usage`; unreported duration is null with `not-reported` in the observation. Readiness and launcher/queue responses emit no completed-turn observation. Protocol references: [Codex app-server](https://learn.chatgpt.com/docs/app-server), [OpenAI token accounting](https://github.com/openai/symphony/blob/main/elixir/docs/token_accounting.md).

Fake clients verify these boundaries on Linux, macOS and Windows. Native consumer qualification and UI behavior remain with each consumer (TaskQ #2094, PilotQ #265).

## Protected app-server

M12.8 owns an independent protected host. `ProtectedHost::signed` checks the operator-pinned executable digest and Ed25519 signature before every dispatch, on macOS and Windows. The signature message is UTF-8 `agentq.protected-adapter/1\0` followed by the lowercase hexadecimal SHA-256. Public keys (32 bytes) and signatures (64 bytes) are lowercase hex. Keys from task text, environment or receipts are not trust anchors. This signature does not provide macOS signature checks or TCC grants.

Protected host dispatch runs in the exact checked executable, including the stable launcher; it never redirects through `current`.

The independent command is `agentq agent host --action prepare|launch|observe|cancel|reconcile --request JSON`. It uses `agentq.app-server/1`, `agentq.app-server-preparation/1` and `agentq.native-completion/1`. A request contains `schema`, `permit`, `policy_revision`, `account` and `preparation`; preparation takes only `schema` and `permit`. The permit binds the actual caller incarnation, immutable caller/operation, canonical project, Codex runtime, boot and first-admission expiry. A Pipe-owned issuer signs its binding and private key hash; the operator's signed policy pins that issuer. Capacity remains Pipe's; this host never admits or releases it.

The host's fixed private root is the OS user's `.codex/agentq-host`. Ambient homes and project files cannot select authority. `trust.json` pins the operator's public key and signed executable digest. `policies/<sha256 of canonical project UTF-8 bytes>.json` is an Ed25519 envelope over the exact policy payload, with domain `agentq.protected-policy/1\0`. It carries existing project Workspace grant provenance/revision, account home, native image/hash, original profile, native arguments and the caller permit issuer. Provisioning this document imports existing authority only; no command creates grants, changes accounts/models/capacity or edits client configuration. Missing, revoked or changed policy refuses launch/input.

The prepared authority adds write protection for the private host root, native and adapter executable directories and rejects a project overlapping those controls. The saved profile remains the source of read/network and runtime/cache permissions; only the existing explicit macOS Codex Workspace boundary is supported. Other native platforms refuse launch. Windows verifies artifact/policy/permit signatures but is not qualified for containment. An unrestricted same-user operator remains outside this boundary.

Preparation is non-launching and authenticates the actual parent PID/creation/boot against the signed permit. It saves independent grant, profile, account/image and approver/revision evidence before returning its digest. The caller durably retains the original permit, independent checkpoint and a one-shot dispatch intent before launch. The host consumes durable intent before launchd registration. Every input revalidates the original preparation, current signed policy, grant/profile/artifacts and exact native/supervisor/frontend incarnations. Missing or changed proof refuses effects. A timed-out input permanently fences the transport; no path retries it.

One launchd generation, sealed input, kernel coalition accounting, durable descendant certificate and acknowledged exact-generation cleanup retain the existing lifetime guarantees. Root/frontend/caller exit, EOF, cancellation and terminal model events never prove descendants or external tool effects ended. Unknown cleanup retains occupancy. The caller alone releases its original permit after both host completion and its external effects finish. Recovery never launches or resends input.

Native stdio uses the existing bounded transport. Only the scope listeners are non-blocking; each accepted stream blocks under its deadlines ([task 266](../evidence/runtime-266.md)). The run's private `AppSession` still owns thread/turn correlation, interruption, checkpoints and its one idempotent `agent.turn` and `agent.call` records. Server requests and native account/model/permission parameters remain caller-owned. The adapter preserves the host's model/account and approved outer boundary; caller protocol parameters cannot widen it. Optimization intent is a compatible no-op (`off`, `optimization_retired`); no optimization environment/header is injected. Launcher attribution remains.

Previously retained TaskQ-shaped checkpoints and receipts are recovery-only evidence, never a launch format for the independent host. Operator-authenticated import retains their exact bytes and operation identities; reconciliation records independent cleanup evidence separately. It does not invoke TaskQ or replace an old adapter identity with the new one. Missing original proof remains unknown and occupied; a copied receipt alone provides no authority. One durable bootout fence and one durable cancellation fence share an operation-wide native-incarnation binding, never request or receipt bytes: a reformatted request or another signed snapshot, older or newer, shares the fence and never repeats a stop. Either action's existing journal record binds both actions; conflicting records refuse without rewriting them. A snapshot carrying original cleanup intent consumes the fence, and a changed incarnation refuses.

The explicit legacy `ProtectedHost::new` API stays available to consumers pinned to the old protocol; the signed independent route never falls back to it. Historical qualification and consumer packets remain in [task 186](../evidence/runtime-186.md) and [task 199](../evidence/runtime-199.md); they do not qualify the independent host. Publication pins, signature/digest and adversarial checks: [independent host evidence](../evidence/runtime-241.md).

## Live run data

Unassociated legacy/request observations remain separate raw evidence with `accounting_unknown: unassociated-usage-overlap`, `countable:false` and null derived counters/cost. Aggregate call count and usage stay unknown until native scope resolves the overlap. A later overlap observation revises already-delivered usage authoritatively, with both origins; it does not retire another turn or invent a request. Consumers must not sum non-countable observations.

All identified-request and legacy accounting identities include the owning native turn while preserving the raw native request ID as a separate fact; no snapshot retirement occurs without positive same-turn association. Native queue usage without an explicit turn ID inherits only its verified current accepted task boundary. LiveData prefers the registered native context measurement, preserving its unknown fields; an input estimate is used only when native context is absent.

Every step exposes neutral `presentation` fields: model, effort, cli_version, sidechain
and meta. Native model/effort/version context carries forward; native per-record flags
do not. Absent fields remain null with reasons; no installed CLI version is substituted
for the version that wrote a transcript. Clients need no native parsing for these facts. Claude attachments are metadata unless
a typed hook, read-truncation, diagnostics or queued-prompt record applies. Hook
steps retain success, blocking errors, additional context and stop-hook summaries
(including hookErrors and preventedContinuation). Queue steps retain native operation
and prompt text; removal never creates a user message. PR links are link steps.
Server tool calls are web steps and tool-search results are tool steps.

A session checkpoint carries its resolved source and parent/child associations.
Recovery validates the same source binding and native prefix, without walking all
homes or rereading the parent to resolve each page. This is cursor state, not a cache.

History includes context, windows, usage, plan, todo and file-rollup steps, with the
same shape for live and finished runs. `Session::live_data` derives the latest context,
all reported windows, model calls with token buckets and cost source, the latest plan
and todos, and changed files grouped by native turn. Missing fields are null with a
reason; no plan, cost, token total or file edit is invented from assistant prose.

Native request IDs and each registered runtime call parser deduplicate usage;
history never chooses accounting or limit parsers by runtime-name prefixes. Rollout
token_count uses per-call last usage, never accumulated totals added to those calls.
Usage is selected with the complete native turn context available in the caller's
read window. Identified Codex requests supersede that turn's legacy snapshots. A
legacy step already delivered on a previous page receives a higher-revision Usage
step with state `superseded`, null counters and `active: false`; clients upsert it
and exclude it from accounting. If both forms first appear in one page, only the
identified request is delivered. A later Usage revision replaces all reported
fields authoritatively, including nulls and unknown reasons; it never inherits a
missing old counter, cost, time or model. App-server notifications and native queue
freeze use the same per-native usage normalization and request revision selection.
Reasoning tokens are a reported subset of output. Cost is explicitly native-reported
or unknown; no pricing table is guessed. Live result cost uses the latest native
result only: missing, null, malformed or negative cost stays unknown instead of
borrowing an earlier result. Explicit native zero remains measured zero.
File rollups retain the native changes and source revisions; repeated delivery replaces a change instead of counting it again.
Claude uses native promptId when emitted. Without a native turn ID, an explicit
user/turn-start record supplies a deterministic derived grouping key, while
`Turn.native_id` remains null with its reason. File rollups never merge edits across
these turn boundaries. Missing native IDs never become completion evidence.

`agentq agent statusline --output <absolute-path>` accepts Claude's native JSON on
stdin, appends it to a caller-owned private JSONL feed and emits no terminal text.
It neither installs itself nor edits a user statusline. A caller may explicitly use
it in per-launch settings or forward an existing feed. `Session::live_data` optionally
joins that feed by exact session ID; mismatched sessions refuse. No usage HTTP endpoint
is polled. Context-window and five-hour/seven-day limit fields are projected only
when native input supplies them; stale/missing data keeps its source time or unknown.

`agent.turn` and `agent.call` share session and native request identity metadata.
Reported numeric cost carries its source, with unknown reasons otherwise. History
content, plan/todo text, file paths, diffs and statusline payloads never enter telemetry.
This adds no token-producing event path and preserves M6.9 and immutable call IDs.
Telemetry joins call.parent to turn.span using a hash of the existing immutable turn
key. Native-reported USD is converted to integer `counts.cost_native_nanousd`, with
`cost_source_native=1`; missing cost carries `cost_source_unknown=1`. No prices are inferred.

## Stopped run observation

`Runtime::stop_event(run, bytes)` polls through the existing runtime authority and
returns a typed `StopEvent`, also convertible to a history `StepKind::Stop` step.
Causes are `exit_without_turn_end`, `signal`, `lost_owner` and `turn_end_process_live`.
An unobserved exit has null exit_code, unknown outcome and an explicit reason;
it never becomes a failed run merely because ownership was lost.

The event includes the last complete native activity in the caller-selected tail
byte window, or a reason it cannot be read. Partial records never become activity.
Its persisted run identity permits repeated delivery and recovery without duplicate display,
while later runs in the same native session remain distinct. A sidecar submitter exit
is not a stopped turn while its native transport still reports running. Native modules
register the stream and transcript formats used for stop activity; core code never
selects them by runtime name.
Full activity output remains available by the original source byte range. No extra
timeout or output ceiling applies. `poll` and `report` retain completion authority;
the event does not release a permit, retry work, send messages or synthesize success.
The same interface works after run recovery, retaining unknown exit provenance. A
runtime-requested cleanup after native end keeps the native exit unknown and is not
a lost-owner event. Other Runtime implementations may explicitly refuse stop observation.

## Universal step history

`Runtime::history` reads live and finished sessions using the same `Turn`, `Step` and
`StepKind` data. Kinds are message, thinking, shell, file_change, tool, mcp, web,
subagent, image, question, approval, compaction, usage and unknown. Each step keeps
its raw record, source byte range and digest; normalization happens at read time.
Nothing writes a native transcript or stores a second canonical history.

Approval and question history projects the same `Event::Request`, `Choice` and
`Question` data emitted by live control ([shared proposal](https://github.com/taskqorg/specq/issues/132)).
Runtime-authored `agentq_control` records retain full request snapshots. The source,
session, original run identity, request generation and opaque request ID bind one
logical step; later physical records replace that observation authoritatively,
including null or missing content. Every revision retains its exact physical origin.
Native request IDs reused by another run or generation remain separate steps.
`Step.id` is the derived upsert identity. The preserved `data.event.Request.id`
is the original opaque control ID, validated only by the original live owned run;
neither `Step.id` nor `native_id` is a replacement reply handle.

Request states are `pending`, `reply-pending`, `reply-sent`, `cancelled` and
`unknown`, distinguishing an ask, reply intent, reply pipe receipt, cancellation
and uncertain delivery. A pipe receipt is never provider acceptance or application.
Unknown states and malformed scope remain visible as unknown. Native-only controls
without verified run/generation use physical occurrence identity and explicitly
unknown reply scope. History never creates a reply handle, restores a callback or
grants respond authority; replies require the original live owned run and its
pending registry. A historical request alone cannot authorize input. Native raw
records remain retained separately; a missing physical link to the original native
record stays unknown rather than creating an invented origin. Native payload fields
named `unknown` remain native data; they cannot supply runtime unknown annotations.
The selected runtime validates its registered source formats at both initial open
and checkpoint recovery; a journal session identity cannot select another parser.

Native item/call IDs determine stable step IDs within a source/session. Revisions
carry the source offset and are self-contained: command, cwd, kind and completed
output survive later result records. Output deltas accumulate into output; clients
upsert by ID and accept the greatest revision. Derivation replays raw source at read
time, including before a recovered page boundary; no second history or cache is stored.
A page coalesces updates per ID and orders them by the last source revision; `origins`
retains every contributing record's exact range and hash for raw reads.
The record block index preserves order within a source record; derived facts follow its content blocks.
Claude replay/metadata records stay raw unknown steps and never replay embedded tools.
Records without a native identity retain their physical identity. Parent call,
turn and session IDs are projected only when native data reports them.
Shell data contains command, cwd, exit_code, duration, output, stdout, stderr and
interrupted; absent fields are null with a per-field reason. Claude stdout and
stderr stay separate. Unknown records are steps with exact raw bytes.

`History::steps(path, bytes)` consumes complete JSONL records up to the caller's
chosen byte window, including a complete record crossing that window. No record
size ceiling is imposed. A partial last record waits; validation failures leave
the checkpoint unchanged. Complete whitespace-only transport separators are consumed
without producing steps or events; malformed nonempty records still refuse.
`read_bytes(path, start, length)` returns the exact source
range after the same identity/prefix checks. Clients choose display/output sizes.
Legacy `poll` and `poll_before` keep their existing paging contract below.

`agentq agent history` accepts JSON on stdin: runtime, session, optional home or
source, checkpoint, and bytes. It returns JSON steps, more, pending_bytes and the
next checkpoint. A source is selected inside the adapter; callers never parse paths.
`Runtime::history_sessions` discovers session IDs and sources read-only, including
Claude subagent files. A Claude subagent is identified by agentId, distinct from its
parent sessionId; discovery exposes its parent session. Opening the child resolves
the parent native call ID from that parent's transcript, when reported, without
reading other sessions' content. Message-chain parentUuid is not
a task-parent link. Spawn results and native SubAgentActivity project child session IDs.
Claude child IDs are scoped by their parent session. Each discovered source also has
an opaque `id`; pass it to `Runtime::history` when a native ID has multiple physical
copies or locations. Parent/child links use these IDs when resolved. Clients never
choose or parse native paths to disambiguate sources.
`Descriptor::history_resume(session, launch, known_native)` and
`Session::resume_command(launch, known_native)` explicitly adopt and acquire the session
before returning an owned `ResumeCommand`. Caller-reported process state is not writer
proof. `arguments()` exposes the native arguments for inspection; `spawn()` uses the
selected launch command, account environment and process hooks, recording native process
identity before returning the child to the caller. Dropping the command before spawn releases its unused
admission. After spawn, the durable native identity continues to refuse another writer
while that process lives, even if the controller exits. The terminal command requires
an empty input and no relay-only options or worker observation windows; it never submits a worker turn. Source handles
resolve to native IDs, and native subagent terminal resume remains refused.

Codex app-server `item/commandExecution/outputDelta` is supported as observed deltas;
they do not prove completion. Claude and Antigravity live console deltas are unsupported
without native proof; their emitted completed output remains readable.
Codex code-mode `exec` is a composite tool step, because its JavaScript can call
several tools or compute arguments. It retains the complete code and console output;
native CommandExecution child records become shell steps when emitted. Native shell
fields absent from code-mode results remain null with reasons; reading JavaScript
text never claims a shell command executed. Native token_usage_record projects its
per-request usage and ID, separately from cumulative thread/turn totals.
Codex completed item PascalCase variants and Claude thinking, toolUseResult and
attachment records are mapped. Fork metadata inside a rollout is provenance, not a
new session receipt; such metadata retains caller binding and a separate
provenance flag. Other records use native/caller binding unless the caller explicitly
binds a Claude copied-session alias as described below. The first metadata record
binds the source. Foreign event
thread IDs still refuse. Antigravity retains its transport limitations below.

## Worker capabilities and bounded history

`agent::history` is the shared read-only observation API for TaskQ and other callers.
It does not submit input, change sessions, grant authority or migrate live workers.
`Runtime` runs (with their private `Turn`, `NativeTurn` and `AppSession`) remain the execution and receipt APIs.

### Required functions and ownership

| Function | Runtime boundary and receipt |
|---|---|
| Launch, resume, session ID | `Runtime::launch`/`resume`; native session receipt, never a generated ID treated as acceptance |
| Input, files, images | Caller retains input and file/image references; CLI passes caller arguments; app-server passes typed input unchanged |
| Ask, reply, result | Native tool/request records are observations; the adapter refuses server requests of its own app-server turn and a protected host replies with the original request ID; CLI has no added interactive reply channel |
| Tools and outputs | History blocks retain native calls/results and IDs; caller tools arrive over MCP (`Launch.mcp`) |
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
| Codex app-server | S one-turn session with MCP/instructions/parts/deltas/cancel; L protected host authority, restart recovery observes only | S caller-recorded JSONL notifications; L unrecognized notifications raw-only | `codex-cli 0.159.3` / macOS arm64: initialize/thread-start capture only; model turns T; protected qualification remains M12.8 |
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

`History::new(format, source, session)` resolves the format through the native registry and binds an immutable caller source label and session. The capability matrix separately enumerates documented transport support; registration alone adds no native qualification claim.
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
`validity` separates native envelope validity from normalization support. It contains
`status` (`valid`, `invalid` or `unknown`) and `reason` (null only when valid).
For Claude native and stream records, a non-empty string `type` is a valid envelope,
including unrecognized types such as `file-history-snapshot`; missing, empty or
non-string discriminators are invalid with reason `missing-or-invalid-record-type`.
Other transports report unknown with reason `envelope-validation-unsupported`.
This validates only the envelope, not message content, completion or authority.
Unknown types keep their raw record, unknown author/kind and normalization limit;
consumers use `validity` to skip unsupported metadata or refuse invalid envelopes
without parsing native JSON. Forward and backward pages expose the same validity.
An emitted native session ID must match the bound session; otherwise the page fails without advancing. `History::with_native_session_alias(format, source, session, native_session)` explicitly binds a caller-validated copied Claude job to its original record session ID. It accepts only Claude formats and a non-empty alias distinct from the caller session, before reading. Records may name only that alias or the bound session; any third ID still fails. The alias is saved and validated in the checkpoint; file identity and consumed-prefix checks remain mandatory. The caller must establish the copy relationship outside this API; a filename mismatch alone never authorizes it.
`session_binding` is `native` on a record carrying the bound ID, `alias` on one carrying the explicit alias, otherwise `caller`, never identity proof. Aliased records expose `native_session` and retain the caller `session`; raw content is unchanged. Existing non-aliased event shapes and IDs stay unchanged. Later Codex copied-parent metadata retains `provenance: true` with caller binding. Claude subagent identity and its separate native parent remain validated; an explicit copied-session alias does not relax the parent check.
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

`agentq agent preflight [--repair]` reads one JSON request from stdin: `client`, absolute `root` and `log`, optional `home`, `store`, `command` array, string-valued `env`, `model`, `mcp`, `instructions`, abstract `policy`, and `readiness` (`continue` or `strict`). It prints ordered `readiness.checks` and `allowed`, exiting 1 when blocked. `--repair` authorizes exact-path workspace and owned hook trust through the installation's ownership journal; it never signs in. The existing native-argument `agent turn` JSON interface requires its default policy; abstract turn policies use `Runtime::launch`.

`agentq agent turn` reads one JSON request on stdin, runs the turn with `agent::run` (or `Turn` without a store) and prints one JSON report. It uses the same `Spec`, `Report` and compatible no-op `Intent` as the library. A request error prints `Error:`/`Next:` and exits 1; a failed turn is a report and exits 0.

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
| `store` | Absolute CoreQ telemetry store path for the `agent.turn` and `agent.call` records; null records nothing |

The report has `schema` 1, `session`, `outcome`, `error`, `text`, `exit`, `started_ms`, `duration_us`, `usage` (`calls`, `input`, `cache_read`, `cache_write`, `output`, `reasoning`, `model`, `unknown`), `process` (`user_cpu_us`, `system_cpu_us`, `peak_rss_bytes`, `unknown`), `limit` (`used_percent`, `resets_at`, `window`, `unknown`), `limits` (every quota `Window`), `background_job` (boolean), `live_background` (job labels), `reached` (`{"reset": n|null}` or null), `recorded` (`ok`, `not-requested` or the append error) and `optimization`. Background, app and app-server modes stay library APIs: they need caller-owned checkpoints and transports that one stdin request cannot carry.

## Optimization intent

`Intent::{Inherit, Off, On}` remains source-compatible for pinned consumers, including TaskQ, PilotQ and Pipe. Every intent is a no-op and reports `off` with reason `optimization_retired`. It reads no optimizer state and adds no child environment or provider-header overrides. The launcher attribution marker remains independent of optimization.

## Adoption and rollback

Consumers of the runtime and CoreQ on their default branches when this page changed. This repository and its runtime use the CoreQ tag in `runtime/Cargo.toml`:

| Consumer | Current use | Adoption |
|---|---|---|
| PilotQ | `coreq` v0.19.1, `agentq-runtime` at `97abcebb305447800fec5875e3cbaf24c8e7f22e` | Pinned; a newer revision follows its own contract and qualification |
| TaskQ | `coreq` v0.19.2, `agentq-runtime` at `2a530cbb846f3ea6d0a3e4c9e8d94e2869130272` | Pinned; a newer revision follows its own contract and qualification |
| DashQ, ServerQ | `coreq` v0.11.0, no `agent` use | None |
| AssetQ | `coreq` v0.19.2, `agentq-runtime` tag `runtime-v0.2.0` (which resolves its own `coreq` v0.18.0) | Pinned; a newer revision follows its own contract and qualification |

- The runtime crate version is in `runtime/Cargo.toml`; consumer changes are listed in [the runtime changelog](../../runtime/CHANGELOG.md). The public surface uses catalogue/limit reads and typed events instead of native `Runtime::send` and raw `progress`; explicit protected request/reply remains under `ProtectedAppServer`. Example source pin: `33cef656ba86536e4e7f970ed5fc637b81622b06`, CoreQ v0.19.2 in its lockfile; the example runs through `agent::Runtime`. Protected API source pin: `aab1a190ab76f1e932578520d7a8787ef1d4b660`; [consumer recipe and publication evidence](../evidence/runtime-186.md).
- Pin updates: `.github/workflows/pin-update.yml` with `scripts/pin_update.py` runs hourly in each repository that calls it. AgentQ moves to the newest CoreQ tag and bumps the runtime patch version, which publishes a runtime tag. A consumer moves to the newest runtime tag and the CoreQ tag it resolves in one PR, so it keeps one CoreQ source. The PR merges through the queue once green unless the caller passes `auto_merge: false`. This moves the pin only; adopting a new API and qualifying native paths stay separate.
- Publication: publish the runtime source commit and pin its full immutable Git SHA (`rev`) in `examples/runtime-consumer`; its checked lockfile must resolve that same remote SHA, without a local path patch. The example builds and runs both clients from that pin on each platform. Publication does not authorize consumer adoption or live installation. After a runtime version change merges to main and that exact source passes the `Test` workflow on Linux, macOS and Windows, `.github/workflows/runtime-tag.yml` invokes `scripts/tag_runtime.py`. The script checks that exact run and SHA, creates the missing tag `runtime-v<runtime/Cargo.toml version>` and reads it back. It never replaces an existing tag and rejects an existing tag at any other SHA. Consumers pin `agentq-runtime = { git = "https://github.com/taskqorg/agentq", tag = "runtime-v<version>" }`.
- One CoreQ source: a consumer that uses CoreQ directly spells the runtime's exact CoreQ dependency from `runtime/Cargo.toml`. Cargo treats `tag` and `rev` at the same commit as two sources, and then two `Store` types. `tests/runtime_publication.py` resolves such a consumer and refuses more than one CoreQ source.
- `examples/runtime-consumer` is the minimal consumer outside the workspace; the runtime contract tests build and run it on each platform.
- Refactor adoption: no live workers run during this refactor. Consumers adopt its completed API at the end; no consumer compatibility paths or opt-outs are added.
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

## Runtime operation and adoption

- It derives from CoreQ v0.18.0 `agent` with its contract tests; TaskQ's argument order and parsers stay unchanged inside the adapters, and Antigravity adds headless `agy` stream-json plus sidecar `new-conversation` / `send-message` and `transcript.jsonl` step observation. Optimization intent is a no-op for existing sidecar servers too.
- `observe(run)` returns one runtime-neutral observation: progress state, native session, and the session URL or its named absence cause with the native evidence text.
- Claude reads only its own native records (transcript `bridge_status`, background job state, debug log); it never launches or rewrites anything for it, and an unreadable record reads `evidence-unreadable`. Causes and sources: [RUNTIME.md](RUNTIME.md#session-observation).
- Read-only worker capability and bounded transcript/event observation uses `agent::history`; source identity, raw content, ordering, limits and per-transport support are defined in [RUNTIME.md](RUNTIME.md#worker-capabilities-and-bounded-history). It grants no submission, replay, stop or cleanup authority.
- Native `agy` lookup appends `~/.local/bin` through `user_bin_path` when PATH lacks it, and never falls back to `agentapi`. Explicit `GEMINI_HOME` wins; otherwise use the Gemini root `~/.gemini`.
- Worker benchmark runs, viewer data and live scratch workspaces stay outside the repository (`--output`, default `<OS temp>/agentq-benchmarks`); live fixtures pass `--dangerously-skip-permissions` to headless agy turns only.
- Every optimization intent reports `off`, reason `optimization_retired`, with no environment or provider override. Historical benchmark evidence remains historical; it does not qualify a current optimizer.
- During adoption the CoreQ `agent` module stays frozen and compatible. Each consumer adopts an immutable runtime revision through its own contract and qualifies its real native paths; CoreQ retires its module only after every inventoried consumer adopted ([adoption](RUNTIME.md#adoption-and-rollback)).

ACP and JSONL RPC candidates use the same unlimited stream transport and canonical history steps. Explicit-source history is available through the runtime trait; native source discovery for unqualified candidates reads unsupported rather than guessing a directory layout.

## Selected inbound app-server history

`codex-app-server-selected/1` identifies the read-only representation
`native-jsonl-inbound-selection/1`: original complete inbound lines selected by the
owned runtime's accepted session/turn correlation. It is not full stdout, native
home history, a runtime-authored control journal or a selection proof by itself.
Only the runtime-owned descriptor can substantiate selection; archive never parses
native fields or falls back to private stdout/stderr diagnostics.

`History::caller` validates Codex registration and expected native identity for this
format. Checkpoints, events, IDs and archive source/wire metadata retain the exact
selected format. Parser interpretation shares the existing Codex app-server decoder;
original bytes, hashes and offsets refer only to the selected physical artifact.
Changed format creates a distinct archive source generation. An absent native ID
stays unknown; it is not inserted into original lines from descriptor metadata.

Direct `History::new`, native Session creation/recovery/discovery, native-session
aliases, resume/adopt/reply/launch and generated-directory authority do not admit
this caller-only representation. Capabilities expose read-only history and retained
tool/media/delta facts, never execution or reply support. The source format grants
no additional path or content-export authority. Native server requests in selected
records retain observation-only unknown reply scope and physical occurrence IDs.

## History media

Caller-owned physical logs use `History::caller(runtime, format, source, binding,
native_session)` with an explicit caller binding and optional expected native ID.
Missing native identity stays null across steps, media origins and recovery; a
native ID appearing without that binding, or contradicting it, refuses. Registered
runtime/format binding is checked before reading and at recovery. This reader
never creates a resolved native Session, resume/adopt/reply authority or a native
session identity from its caller label.

`History::media(path)` reads only positively typed media references in that explicit
physical source through the shared catalog and byte/hash/range checks. It never
scans session-wide generated-artifact directories, even when the caller label
resembles a native ID or an expected native ID is supplied. `Session::media()`
delegates to the same implementation with its separately validated native scope;
only that scope retains existing native generated-artifact discovery. Arbitrary
paths and quoted prose confer no additional file authority.

`Session::media()` derives a `MediaCatalog` from the same complete native records as
steps. Each descriptor has kind, MIME, name, size, SHA-256 and a source (inline,
local path, generated asset or URL), native step/origin links and explicit unknown
or missing reasons. Claude image/document and nested tool-result blocks, Codex
input_image/local_image/ImageView/generated assets and Antigravity brain artifact
references are supported. Codex generated_images/<native-session> and the selected
Antigravity brain directory are additional native session sources. Structured tool/file-change path references are inspected;
Only recognized native media blocks and structured tool/file-result fields authorize references; JSON in assistant/user prose or arbitrary nested objects does not. Quoted conversations and text/prose containers revoke inherited tool-result authority. Sidecar media/tool-call containers are recognized only at the root of a validated Antigravity sidecar record, never from fields in another runtime or nested content. Native event, step and media image spellings share the same native classifier. Each physical source revision is inspected and linked to its logical step, so completion cannot discard earlier references. Implicit generated-asset discovery rejects symlinked roots and ancestors, and rechecks the anchored session directory on reads. Assistant prose and filename extensions never establish media. Video and audio
classification requires a real file signature. URLs are retained without fetching.

The catalog decodes inline base64 once per read, deduplicates backing bytes by hash,
and offers `read(key, start, length)` by descriptor ID, SHA-256 or an explicitly
referenced path. Only references found in that session and native Antigravity brain
artifacts enter the catalog. A requested arbitrary path refuses. File reads validate
the content hash before returning a caller-selected byte range; changed, vanished,
invalid or inaccessible sources remain explicit. No output ceiling, timeout or
persistent media/steps cache is introduced. Clients choose display and stream ranges.
Media catalog descriptors can be emitted as media steps without losing original raw
source ranges. Archive #283 can copy catalog bytes once by hash while available;
this API does not claim durable retention after files vanish, and does not write an
archive or a viewer. Viewers treat media as untrusted content.

Runtime `Event::Image` retains the native item identity, whether it is generated,
its saved path and the full native image item. Codex imageGeneration/image_generation
and savedPath/saved_path survive the trait boundary, including generation failure
with no saved path; consumers need no native JSON parsing for those facts.

## History sources

Qualification record: [runtime-history-290.md](../evidence/runtime-history-290.md).

The Runtime trait also resolves history-only adapters. These adapters advertise only
history; launch, resume, input, control, account and process operations explicitly
refuse as history-only. The existing execution-runtime enumeration remains unchanged;
`history::sources::runtimes()` enumerates all history descriptors. No caller branches
on agent names. Each source has its own small Rust module, adapted from Orca's MIT
scanner/decoder contracts and fixtures (NOTICE). Search remains outside the runtime.

Initial open and checkpoint recovery validate the exact registered format for the
selected runtime. History-only adapters bind their SourceSpec format; execution
runtimes bind their native protocol formats. Another registered source or an
unregistered format prefix never supplies an exception.

Native JSONL records preserve exact ranges. Whole JSON documents and SQLite stores
are read as source snapshots: each derived row carries its native JSON pointer or
SQL table/key, physical file identity/hash and byte-range references. Reconstructed
rows are labeled as projections, never original file bytes. `source_bytes` reads only
these native source parts. Snapshot changes produce a new monotonic generation; every page exposes that generation, and the first page of a changed snapshot carries `reset: true` so clients replace the previous view before applying its steps. Stable native row keys keep step identities across rewrites. Cursor revisions remain monotonic;
client-selected page bytes choose the next rows, including a complete crossing row.
Unknown native records remain unknown with exact source access; no record cap applies. Every session-bearing record, including recognized nested envelopes and metadata companions, must match the selected native session before any snapshot page is published. Companion bytes participate in the manifest, identity and generation. Identical-byte physical file replacement resets the view.

SQLite native DB/WAL bytes are opened only as files for reading, copied into a private
temporary directory, checked again for concurrent content or file-identity changes, and queried there. WAL header/frame salts, checksums and complete framing are validated before SQLite can ignore a damaged tail; an invalid committed WAL refuses with an explicit error, never a successful stale projection. A stable raw WAL-index header, when present, binds the committed frame count/checksum and permits excluding a previous generation's physical tail. The private query copy uses only that validated committed prefix; native DB/WAL/index bytes remain unchanged and bound in the source manifest. Native
-shm files are neither opened for writes nor created. A changing source refuses rather
than retrying or claiming a consistent snapshot. Temporary query files are removed
when the read ends. No native checkpoint, vacuum, migration or schema change occurs.
Derived SQL rows are session-scoped, but raw DB/WAL/index source parts can contain other
sessions. This local reader grants no archive or sharing authority over those bytes;
a selected session alone never authorizes sending its entire database off-host.

Explicit home roots, runtime home variables and caller-supplied WSL/UNC homes share
the same readers. Callers list each explicit home through the same generic request. Sources
are chosen by opaque IDs; shared databases scope every row to the selected session.
An absent native proof means fixture-only, even if a CLI happens to be installed.
Native Mac qualification never claims native Windows or WSL qualification.

| Agent | Default native source under user home | Format | Support target / reason |
| --- | --- | --- | --- |
| claude | `.claude/projects` (`CLAUDE_CONFIG_DIR` override) | native | limited: native Mac read proof; unknown shapes retained |
| codex | `.codex/sessions` (`CODEX_HOME` override) | native | limited: native Mac read proof; unknown shapes retained |
| antigravity | `.gemini/antigravity-cli/brain` (`ANTIGRAVITY_HOME` override) | native | limited: native Mac read proof; unknown shapes retained |
| hermes | `.hermes/sessions; .hermes/state.db` (`HERMES_HOME` override) | json/sqlite | limited: native Mac read proof; unknown shapes retained |
| gemini | `.gemini/tmp` (`GEMINI_CLI_HOME` override) | mixed | fixture-only until native read proof |
| cursor | `.cursor/projects` (`CURSOR_HOME` override) | jsonl | fixture-only until native read proof |
| copilot | `.copilot/session-state` (`COPILOT_HOME` override) | jsonl | fixture-only until native read proof |
| cline | `.cline/data/sessions` (`CLINE_SESSION_DATA_DIR` override) | json | fixture-only until native read proof |
| codebuddy | `.codebuddy/projects` (`CODEBUDDY_HOME` override) | jsonl | fixture-only until native read proof |
| devin | `.local/share/devin/cli/transcripts; .local/share/devin/cli/agent_logs` (`DEVIN_HOME` override) | json | fixture-only until native read proof |
| droid | `.factory/sessions; .factory/projects` (`FACTORY_HOME` override) | jsonl | fixture-only until native read proof |
| grok | `.grok/sessions` (`GROK_HOME` override) | mixed | fixture-only until native read proof |
| jcode | `.jcode/sessions` (`JCODE_HOME` override) | json | fixture-only until native read proof |
| kimi | `.kimi-code/sessions` (`KIMI_CODE_HOME` override) | mixed | fixture-only until native read proof |
| kiro | `.kiro/sessions/cli` (`KIRO_HOME` override) | mixed | fixture-only until native read proof |
| muse | `.local/share/muse/sessions` (`XDG_DATA_HOME` override) | jsonl | fixture-only until native read proof |
| omp | `.omp/agent/sessions` (`OMP_CODING_AGENT_DIR` override) | jsonl | fixture-only until native read proof |
| openclaw | `.openclaw/agents` (`OPENCLAW_STATE_DIR` override) | jsonl | fixture-only until native read proof |
| opencode | `.local/share/opencode/.` (`XDG_DATA_HOME` / `OPENCODE_DB` override) | mixed | limited: native Mac read proof; unknown shapes retained |
| opencode2 | `.local/share/opencode/.` (`XDG_DATA_HOME` / `OPENCODE_DB` override) | sqlite | fixture-only until native read proof |
| pi | `.pi/agent/sessions` (`PI_CODING_AGENT_DIR` override) | jsonl | fixture-only until native read proof |
| qoder | `.qoder/projects` (`QODER_HOME` override) | jsonl | fixture-only until native read proof |
| zcode | `.zcode/cli/db` (`ZCODE_HOME` override) | sqlite | fixture-only until native read proof |
| rovo | `.rovodev/sessions` (`ROVODEV_HOME` override) | json | fixture-only until native read proof |

Source-specific companions: Cline <id>.messages.json; Kiro <uuid>.jsonl beside its
metadata; Grok chat_history.jsonl beside summary.json; Rovo session_context.json;
Kimi agents/*/wire.jsonl beneath session_* with state.json agent identity/parent lineage; OpenCode session/message/part JSON stores
or session/message/part SQLite tables; OpenCode 2 session_v2/session_message tables;
ZCode its separate db.sqlite. OMP/Pi/OpenClaw message envelopes retain tool results,
thinking and compactions. Kiro toolUseId joins call/result revisions, and OpenCode file parts preserve MIME-appropriate media kinds. Gemini thoughts/toolCalls, Copilot data envelopes, Devin
ATIF observations, and OpenAI/Anthropic block variants retain their native fields.

History-only native accounting retains reported token buckets and cost per message; absent buckets stay unknown. Session/model aggregates never become extra model calls. Snapshot pages preserve native parent-session fields independently of message chains. Resolved parent/child links require an unambiguous native source family; Kimi links stay within the same physical session directory and multiple root main agents refuse. Selecting an opaque source ID never authorizes linking a copied workspace's parent or child. Devin also resolves its native APPDATA location on Windows; Pi/OMP overrides accept the native sessions root.
