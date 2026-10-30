# agentq contract

agentq installs, routes and observes context optimization for Codex, Claude Code and Hermes on one host. It is one static binary on [coreq](https://github.com/taskqorg/coreq) with no server, relay or dashboard of its own. This file holds every agentq decision; its linked detail files hold binding detail.

It follows the [specq release contract](https://github.com/taskqorg/specq/blob/main/engineering/release-contract.md) and [taskq.md](https://github.com/taskqorg/taskq/blob/main/taskq.md) § 4 and § 8 as their reference consumer. Product outcomes: [docs/spec/PRODUCT.md](docs/spec/PRODUCT.md) revision 2; § 11 maps each requirement here. Principles `P1`–`P5` and section rules `M<section>.<n>` are mandatory; text after them in a section is binding detail.

## Principles
- **P1 Never break the agent.** Any agentq failure keeps the original tool output and the client's own login, model, approvals and sandbox.
- **P2 Unverified means unknown.** Configured is not verified. Stale or missing values say so with a reason, never zero or success.
- **P3 Spec first.** A change that alters a decision edits this file or its linked detail first; code, README and INSTALL follow. Only the owner accepts a decision change. AGENTS.md points here.
- **P4 One static binary.** `agentq`, built from an empty Rust codebase (PRODUCT R11). No Node, npm or toolchain on the host. Python only for the headroom engine (§ 3).
- **P5 Client only.** The client holds no relay, GitHub App, gateway server, serviceq panel or dashboard. Server duties belong to [dashq](https://github.com/taskqorg/dashq) (§ 6, § 8).

## 1. Commands
- **M1.1** Output: one line per outcome; errors `Error: <problem>` then `Next: <action>`, at most 2 lines. `--json` prints the same data as JSON.
- **M1.2** `agentq hook` never fails the client's hook: 5 s budget; a failure returns `{}` and exits 0. It runs in the launcher process itself, so a release mid-update or a failed start cannot fail it.
- **M1.3** `agentq doctor` is read-only and exits 1 on a problem. Absent optional Langfuse keys alone do not fail local health.
- **M1.4** `agentq check compression` makes paid client calls and runs only when explicitly invoked; offline `selftest` never starts them.

| Command | Does |
|---|---|
| `agentq onboard [agent..]` | Read-only discovery: ready Codex, Claude Code, Claude Desktop and Hermes profiles per runtime; missing input; legacy install (§ 10); other optimizers. No writes. |
| `agentq onboard --yes [agent..]` | Install the release and engines, apply § 4 for the selected agents, then (re)start the service unit so its first pass starts the proxy on each fixed port. Wait up to 60 s for each routed proxy; a silent one gets one `agentq service restart` and another 60 s, else the line names it. Then run `doctor` and print what to verify by a real turn. Repeat is safe. |
| `agentq enroll <https url> [--headless]` | DashQ provider-backed browser approval (headless: link and code only); protected observation grant and projected snapshot receipt (§ 6). Released before qualification #95. |
| `agentq connection status [--json]`, `agentq disconnect`, `agentq langfuse` | Connection, projection, export and revocation state with the next action; disconnect stops sending at once and retries the revoke. Released before qualification #95. `langfuse`: standalone mode, reads Langfuse host and keys from hidden stdin into protected storage (§ 6). |
| `agentq run -- <cmd> [args..]` | Run a native command through RTK when qualified, else unchanged (§ 3). Exit code and stderr preserved. |
| `agentq compress <agent>`, `agentq stats [--json]`, `agentq stats --accounting [--from <unix>] [--to <unix>] [--bucket <s>] [--events <path>]` | Compression core entry, its local stats, and the read-only ACCOUNTING O1–O3 aggregate of local stats or of exported `agentq.compress` events at `<path>` (§ 3). |
| `agentq mcp <agent>`, `agentq hook <agent> <event>`, `agentq model codex` | Stdio MCP server `agentq` (§ 2); hook handler for § 4 events, JSON on stdin and stdout (M1.2); loopback provider entry for one finished Codex model call, JSON on stdin, queues `agentq.model` (§ 6). |
| `agentq profile [--json]`, `agentq profile trace on [<minutes>]\|off` | Ranked profile of measured operations against their budgets; detailed trace switch (§ 6). |
| `agentq direct [agent..]` | Restore direct provider access: remove routing edits only, keep MCP and telemetry; works when the service is down (PRODUCT R4). Records the agents in `config.json` `direct`, so direct is intended; `onboard --yes` clears that and routes again. |
| `agentq optimization on\|off [codex\|claude\|hermes] [--dry-run]` | Change the host compression switch or one agent's preference without editing native client files or restarting clients (§ 4). |
| `agentq disable\|enable [codex\|claude\|hermes\|claude-desktop] [--dry-run]` | Reversibly detach selected owned integrations or restore the saved supported fields; data and installation stay. No target selects the installation; enable without a saved disable is unchanged (§ 4). |
| `agentq status [--json]` | Fresh local integration, compression and service state, including intentional disable, pending reload/drain, aliases, conflicts and observation time; works with the service stopped. |
| `agentq configure [<profile> <field>=<value>.. [--yes]]` | Profile projection, typed plan, or guarded apply with host readback under the lifecycle lock (§ 4). Released before qualification #95. |
| `agentq check compression [codex\|claude\|both] [--codex PATH] [--claude PATH] [--fixture cargo\|pytest] [--output DIR]` | Live native CLI qualification with the embedded checker, Python and the chosen installed runner, default cargo (§ 3). |
| `agentq doctor`, `agentq telemetry status\|verify [agent]\|export`, `agentq incidents [list\|show]` | Health of install, host, service, engines, agents and legacy state (M1.3); queue status, synthetic read-back check and local JSONL export (§ 6); local incident reports and what would be sent (§ 7). |
| `agentq update [--check]`, `agentq selftest`, `agentq service install\|run\|pause\|resume\|status\|restart\|stop\|uninstall\|list` | coreq update steps and service (§ 5); selftest checks the binary, built-in fixtures and engine pins, and on Windows retires a locked legacy fixture in the OS temp dir (§ 10). |
| `agentq uninstall [--dry-run]`, `agentq uninstall --legacy [--dry-run]` | § 9; § 10 removes the legacy install; none left: `already removed`. |

- Each `hook` run stamps the event time as configuration evidence. A run that cannot read stdin or write stdout records `data/evidence/<agent>.<event>.failed` with its time and error. A successful run moves it to `<agent>.<event>.last_failure`, so `.failed` exists exactly while the latest run failed; the last failure stays for the heartbeat (§ 6).
- `doctor` checks install, host tools (§ 5), service, engines, owned edits count, local SQLite integrity, telemetry queue and optional keys, legacy install, running legacy pids and files left in a retired legacy folder. Each recorded hook failure fails, an unreadable record included.
- `doctor` reports per agent its routing and whether that is intended (§ 5): an onboarded agent intended routed but not routed fails with `run agentq onboard --yes`; an unknown intent fails. Ports and routing drift join when the service owns them; `onboard --yes` reapplies owned edits.
- The Node CLI's `install`, `rollback`, `set-key`, `provider`, `headroom`, `gateway`, `monitor`, `proxy`, `retrieval`, `probe`, `plugin-path`, `verify`, `connect`, `set-langfuse`, `lifecycle`, `native-hook`, `tools` and `incidents report|enable|disable` do not exist; their duties live above or are dropped in § 11.

## 2. MCP tools
- **M2.1** Server name `agentq`; tools `index_document`, `search_document`, `get_original`. Tool names and input schemas stay stable (PRODUCT R2). Clients reload registrations normally.
- `index_document` takes a UTF-8 file or text of at most 1 MiB, stores the original and splits it into chunks. `search_document` ranks term matches over chunks of one index and returns passages with their index ref. `get_original` returns the retained bytes for a ref unchanged; an expired ref says `expired` and names the source.
- `ponytail:` plain term scoring in the binary; swap in an engine behind the same tools when a paired task test shows a gain. Limits: 100 originals per profile, 7 days, 256 MiB; oldest go first. context-mode is not an engine (§ 11).

## 3. Managed engines
- **M3.1** Engine pins (version, per-platform URL, sha256) are built into the binary; no config changes them. No install script fetches an engine. A failed engine never fails the command: its line names the reason and agents keep working without it.
- **M3.2** Code, complete JSON, dense constraints and short results pass through (PRODUCT R3).
- **M3.3** One core in the binary decides, compresses, retains and counts; adapters only translate. A new agent needs an adapter, never a new core path.
- **M3.4** Only the built-in qualified list of an adapter selects an engine; there is no generic post-filter. A result that is not smaller, an error or 3 s without an answer returns the original text.
- **M3.5** A native command executes once. RTK sees only its captured output and never reruns it.
### Engines and pins
- An engine downloads into `<install>/engines/<name>-<ver>/` on install and on an update that changes its pin, and is checked by sha256 and `--version`. Engines install in `service install` before the unit loads and on each `service run` start, so a release with a new pin installs it; an engine already checked is reused.
- **RTK:** prebuilt binary from `rtk-ai/rtk` releases, pin 0.51.0. `agentq run` passes only the qualified list (`pytest`, `cargo test|build|check`, `go test`) to `rtk`; others run natively. Native RTK execution keeps command semantics. Read-only sandbox without writable storage or no checked RTK: run natively, no compression.
- Raw output of each RTK run is retained in `data/runs/<id>/` for `get_original` with ref `rtk:<id>` and the hash of RTK's recovery hint (500 runs, 7 days, 512 MiB).
- **headroom:** loopback model proxy for routing and usage observation; its request compression stays off; its supported extension sends qualified new tool results to the core. It never injects tools, never rewrites transmitted prefixes and never switches provider.
- headroom pin `headroom-ai[proxy]` 0.40.0 installs with `pip --require-hashes --only-binary :all:` from the built-in lock `agentq/headroom.lock` into a private venv `<install>/engines/headroom-<ver>/`, checked by `headroom --version`.
- headroom uses Python 3.11–3.13 found on the host (`AGENTQ_PYTHON` overrides). The pin's wheels need macOS 14+ on arm64 and do not exist for macOS x64. No Python or no wheels: agents run unrouted, `doctor` says why.
### Compression core
- Accepted results target: [ACCOUNTING O1–O4](docs/spec/ACCOUNTING.md) defines outcome populations, paired byte measurements, coverage and whole-task evidence for PRODUCT R9/R10. This adds no optimizer, capture permission or universal savings claim.
- Core input: `agent`, `adapter` (`run`, `hook`, `provider`), `tool`, `command` (shell tools only; matched to a runner, never stored), `text`, native `exit_code` for captured results, native IDs when known.
- Config off for the agent, under `min_bytes`, code or complete JSON: pass through. Anything off the adapter's list passes with reason `not_qualified`. Every call writes one stats record.
- A compressed result keeps the original under a `get_original` ref (`<engine>:<id>`, store and limits of `data/runs/`) and ends with one line naming that ref. Unqualified, disabled or unavailable-engine runs stay native without capturing output; their byte measurements are unknown.
- The run adapter captures eligible native stdout/stderr once before RTK filtering, keeps stderr and the native exit code, and returns native bytes whenever verification fails; its 3 s filter budget excludes native execution. Retained originals are stdout bytes; stderr is never compressed.
- Qualified lists are built in per adapter and change only by a release; no config adds a tool. The runner set is the RTK list. Every other tool of every adapter (file reads, search, web, MCP results, other commands) passes unchanged.
- `run` (Claude Code through its hook, any agent calling `agentq run`): a listed command executes natively once; RTK executes a private replay of that capture through its qualified recipe. `provider` (Codex `exec_command`, `shell`) and `hook` (Hermes `terminal`) reuse that bounded capture replay with the native exit code.
- Replay verifies the native arguments and exit code; for pinned RTK pytest it also expects only its injected display flags (`--tb=short`, `-q`, `-rxX`, each omitted when the native args already set that category). A mismatch returns the original. Recipes stay limited to the runner set, with whole-task checks and the protected-result guards.
- Headroom keeps transport passthrough; its supported proxy extension calls this core, never its generic history optimizer. Entries, `compression` config, the stats record and accounting coverage: [COMPRESSION.md](docs/spec/COMPRESSION.md). `agentq check compression` pass rules: [COMPRESSION-BENCHMARK.md](docs/spec/COMPRESSION-BENCHMARK.md#native-client-check-rules).
### Adapters
- An adapter maps the native result to the core input, writes the returned text back in the native shape and keeps the original on any failure (P1). It never holds thresholds, engine choice, retention, a retrieval tool, stats, telemetry, config or network calls of its own.
- **Claude Code:** outside native auto mode, the § 4 `PreToolUse` rewrite to `agentq run` is the adapter. In auto mode, the original command stays for native permission classification and only successful Bash stdout is compressed through `PostToolUse.updatedToolOutput`; stderr and the response envelope stay unchanged.
- Claude failed, interrupted and image results remain native because the failure hook cannot replace output. No hook grants permission or alters the client's mode.
- **Codex:** no compression hook. The loopback provider sends each new `function_call_output` or `custom_tool_call_output` (string, typed or JSON-wrapped text) of a listed tool through the core once per call ID and replays the returned bytes on later requests, so the transmitted prefix stays byte-identical. Other items pass unchanged.
- Codex sessions key on native `thread-id` (then `session-id`) for HTTP/SSE and WebSocket Responses. Code mode qualifies only one literal `const|let name = await tools.exec_command({...}); text(name);` carrier; arbitrary JavaScript and multiple calls pass unchanged.
- Codex replay is durable in `data/provider/<session hash>.json`: 512 results / 16 MiB per session, 64 MiB total, no eviction of transmitted bytes. Corrupt, unwritable or full replay storage preserves the original.
- **Hermes:** the shipped plugin `agentq` registers `transform_tool_result` and `post_api_request` (§ 6). `transform_tool_result` runs `agentq compress hermes` for `terminal`, the only listed Hermes tool, and unwraps its `output` field; other tools return before any call.
- The Hermes plugin has no proxy URL, own retrieval tool, lifecycle call or query memory: retrieval is MCP `get_original`, as for every agent.

## 4. Hooks and config edits per runtime
- **M4.1** Each edit is recorded in the ownership journal (§ 9) before it is written.
- **M4.2** Existing foreign optimizers or custom providers stop the edit for that file with a named conflict; nothing is replaced by guess.
- **M4.3** Loopback port per agent and runtime is fixed at install and kept across updates.
### Edits per runtime
| Runtime | Files and edits |
|---|---|
| Codex CLI and Desktop | `config.toml`: `model_provider = "agentq"`, `[model_providers.agentq]` loopback `/v1`, `wire_api = "responses"`, websockets; `[mcp_servers.agentq]` runs `<install>/agentq mcp codex`, `enabled_tools` the three tools, `index_document` `approval_mode = "approve"` unless the user set one. |
| Codex hooks and telemetry | `hooks.json`: `SessionStart` and `Stop` run `agentq hook codex <event>`. Observability plugin config points its exporter at the loopback OTLP port. |
| Claude Code CLI and Desktop Code | `settings.json`: `env.ANTHROPIC_BASE_URL` loopback, telemetry env pointing at the loopback OTLP port, `SessionStart`, `PreToolUse`, `PostToolUse` and `Stop` hooks; `Stop` reads model calls (§ 6). Outside auto mode, PreToolUse wraps qualified Bash commands with `agentq run`. `~/.claude.json`: `mcpServers.agentq`. |
| Claude Desktop Chat | `claude_desktop_config.json`: `mcpServers.agentq` only; no routing (PRODUCT R1). |
| Hermes | `config.yaml`: `mcp_servers.agentq`, `plugins.enabled` `agentq` with the shipped plugin files; Langfuse plugin tags. Hermes keeps its own Langfuse exporter configuration (§ 6). |
| Instructions | Global `AGENTS.md`/`CLAUDE.md` block between `<!-- agentq:start -->` and `<!-- agentq:end -->`, naming `agentq run` and `agentq doctor`. |

- Legacy entries `supercompress`, `headroom`, `context_mode` MCP servers and launcher-owned hooks are removed only when the journal or the legacy uninstall (§ 10) owns them. Desktop clients that override `ANTHROPIC_BASE_URL` bypass routing; the `SessionStart` hook tells the model so and `doctor` reports it.
- Config roots: the client's own env (`CODEX_HOME`, `CLAUDE_CONFIG_DIR`, `HERMES_HOME`), else the default home folder; project-level overrides are reported, never edited. Native Windows and WSL are separate installs.
- A JSON config that starts with a UTF-8 BOM is parsed without it; the edit writes it without the BOM and its `edited:` line says `UTF-8 BOM stripped`. Uninstall restores the original bytes, BOM included.
- [CONFIGURATION A1–A5](docs/spec/CONFIGURATION.md) owns profile projection and separately authorized typed management with revision checks, journaled apply, host readback and real-client verification. Observation enrollment grants no configuration authority.
- Typed envelopes and host acceptance: `agentq/configure.rs` with fixtures `docs/spec/configuration-fixtures.json`. Local guarded apply, readback and hook evidence: `agentq configure`, `agentq/executor.rs` (CONFIGURATION "Host executor"). Remote operations are released before qualification #95.
### Temporary controls
- **M4.4** Compression and integration state are independent. Host optimization off writes `compression.enabled=false`, on writes `true`. Agent off writes `compression.agents.<agent>=false`, on removes that key; a host pause preserves agent preferences.
- **M4.5** All adapters and anonymous `agentq run` obey the host switch; per-agent isolation requires a known agent. Previously transmitted Codex replay bytes never change.
- **M4.6** Full disable persists intent before removing owned routing, MCP, hooks, Hermes plugin files/activation and journaled instruction blocks. It keeps the installation, release/engine assets, local data, originals, keys, grants and queued observations under their existing retention.
- **M4.7** Only explicit `enable` clears disable intent; onboarding, service install/restart and configure cannot reconnect a disabled profile (`integration_disabled`). Manual binary update remains available without reactivation. `direct` keeps its routing-only meaning.
- **M4.8** Corrupt or unreadable lifecycle state (`data/integration.json`) means unknown and blocks mutations and new compression rather than resetting state.
- **M4.9** Detaching configuration does not unload a running client's cached provider, hooks or MCP. agentq suppresses new transformations, preserves active work and exposes `pending_reload`/`pending_drain` until a native inventory proves the drain.
- Removing owned Codex routing (`disable`, `direct`, uninstall) restores the native default selection but keeps `agentq` as an exact, marked direct OpenAI definition without a base URL or AgentQ dependency, because Codex conversations persist their provider ID. Codex selects its native endpoint from the unchanged login/auth mode.
- That direct definition is not an attached optimization. Enable or onboarding may replace only the exact unedited compatibility block with the owned loopback definition; later edits remain conflicts. It stays after uninstall, and previously detached installations can be repaired with it when the name is absent.
- While a host disable is pending, the service runs only required transport and recovery: export, automatic update activation and remote configuration application are suspended. The enrolled DashQ snapshot keeps its 60 s cadence with lifecycle `pending_disable` (§ 5), so DashQ can name the host disabled.
- Resume is never repaired by rewriting conversation history, provider IDs in stored chats, authentication, model, approvals or sandbox. Native login, model, approvals, sandbox and foreign telemetry settings are outside disable and enable.
- New or restored Desktop Chat MCP registrations use `agentq mcp claude-desktop`, so its disable intent is independent of Claude Code. Existing shared `mcp claude` connections have no caller identity and stay pending reload; Claude Code is never disabled to guess their caller.
- Native qualification is reported per client and platform; code checks alone do not qualify working profiles. Rollout to the existing Mac, Windows and FRA installations is authorized for release 0.8.0 only, with the Desktop and reboot gaps recorded in the integration-control plan; those paths stay unqualified and this is not a stable-platform claim.
- Under disable, foreign entries and later edits remain and conflicts are explicit; shared roots and registrations switch together. Lifecycle state, transitions, locks, drain, service handling, status and revisions: [INTEGRATION-CONTROL.md](docs/spec/INTEGRATION-CONTROL.md).
### Namespace cutover
- **M4.10** The MCP server, Codex provider, managed instruction markers and Hermes plugin are named `agentq`. No old namespace alias is registered. Existing Codex chats are not migrated: histories stay intact, but chats bound to the retired provider must be replaced by new chats.
- `agentq migrate-names [--dry-run]` renames only unchanged owned registrations under the lifecycle lock; onboarding and service startup run it. `agentq migrate-names --rollback` restores recorded names only while ownership/control metadata still matches the cutover. Procedure: [LEGACY.md](docs/spec/LEGACY.md#namespace-cutover).

## 5. Install layout and service
- **M5.1** coreq owns layout, update, launcher, rollback, service units and `status.json`; agentq passes tool name `agentq`, install folder and release repository `taskqorg/agentq`.
- **M5.2** One OS unit per host and runtime runs the service; there is no other resident process. More than one `service run` is an error naming every pid.
- **M5.3** Release trust is HTTPS plus who can push tags; installers and self-update check only the sha256. A failing selftest publishes nothing.
- **M5.4** agentq has no administrator rights and installs only its engines. Data in `data/` changes only additively (compatibility N/N-1).
### Layout, release and service
- Install: `~/.local/share/agentq`, Windows `%LOCALAPPDATA%\agentq`: `releases/<ver>/agentq`, `current`, `previous`, launcher `<install>/agentq`, `engines/`, `data/` (§ 8), `status.json`.
- Onboarding and `service install` make the stable launcher available as `agentq` for the installing OS user; `agentq setup-path` repairs it without changing integrations or restarting services. Uninstall removes only this launcher's registration ([procedure](docs/spec/SERVICE.md#command-registration)).
- Release CI on a `v*` tag from `main` builds `agentq-<ver>-<os>-<arch>` for macOS arm64/x64, Windows x64, Linux x64/arm64 (static musl), plus `SHA256SUMS` and `latest.json`. Each binary and `SHA256SUMS` carry a GitHub build provenance attestation (`gh attestation verify <file> --repo taskqorg/agentq`).
- A checked Headroom venv is immutable; a failed version probe reports unavailable and never deletes it. Headroom checks, installation and proxy workers use one OpenBLAS/OMP thread.
- The unit (coreq project `host`: `com.agentq.host`, `AgentQ-host`, `agentq-host.service`) runs `<install>/agentq service run` in `<install>`. It owns the loopback ports, headroom workers, OTLP intake, the queue sender, update checks and `status.json`.
- A `service run` process is one whose command line runs a binary under `<install>/releases/`; the Windows launcher is not counted. `service install` copies the running binary only where the bytes differ ([Windows processes](docs/spec/SERVICE.md#windows-service-processes)).
- Update: coreq steps, hourly; each update keeps only `current` and `previous`; a dashq release notice may trigger an early check. A pin change runs at most two headroom workers. A service run restarts into a new `current` only with no open proxied connection, at most 15 min after the change ([detail](docs/spec/SERVICE.md#update-and-worker-swap)).
- `status.json` lives at `<install>/status.json`, written every 30 s; `service status` treats it as unknown after 90 s. It adds per-agent routing and intent, MCP, hooks, engines, queue, `model_backlog`, newest incident and per-profile `configuration`, without paths or file text ([fields](docs/spec/SERVICE.md#status-file-fields)).
### Host requirements
- agentq runs only the external tools in the [host tools table](docs/spec/SERVICE.md#host-tools) (specq [host requirements](https://github.com/taskqorg/specq/blob/main/engineering/release-contract.md#host-requirements)); RTK and headroom are § 3 engines. A change that starts using a new tool, flag or API raises its row in the same change.
- `doctor` checks each row: missing, unreadable or below its minimum fails as `host tool <name>: <found> below minimum <min>; <fix>`; OS components are checked for presence on PATH in one line; optional tools report without failing. `service install`, `onboard` and each `service run` start report every failing requirement with its fix step; `doctor` stays failed until the host provides it.
- Every host tool call goes through one wrapper. A failed start, and a failed exit where success is required, is logged as `` agentq: `<command>`: <error> `` to stderr (the service log for `service run`) and returned as an error, never as an empty result ([exceptions](docs/spec/SERVICE.md#host-requirement-notes)).
### DashQ release observation
- The additive `release` object in `status.json`, snapshot schema 1 and the heartbeat (`agentq.release`, JSON) is installation-scoped observation of `selected`, `running`, `coreq_version`, `health` and `lifecycle` ([fields](docs/spec/SERVICE.md#release-observation-fields)). Installation identity and source `at` bind it to this producer; it grants no control authority.
- `health` proves only that the running release wrote status, never routing, workload or client health, and never the selected version when it differs. No elapsed-time or socket-only detach proof is added. No paths, arbitrary errors or native configuration are exported.

## 6. Telemetry to Langfuse
- **M6.1** Writers never wait for the network: an outage, missing keys or an unwritable queue never blocks or fails agent work, and the queue drains on the next successful pass.
- **M6.2** Metadata only: no raw content, arguments, keys or paths. `agentq.compress` names only its tool, matched runner (`cargo test`, `go test`, `pytest`) and engine (`rtk`, `headroom` or `none`).
- **M6.3** Unknown is never zero: a missing value omits its attribute and names the reason. Estimates carry an `estimate` label and never count as observed usage.
- **M6.4** Secrets live in Keychain (`security`), Secret Service (`secret-tool`) or a 0600 file `data/langfuse` where neither works (Windows until a Credential Manager binding lands); never in config, argv, logs or telemetry. curl gets them through its stdin config.
- Target: Langfuse project `Agents` over OTLP HTTP JSON, `<host>/api/public/otel/v1/traces` with Basic auth. Native exporters (Codex observability plugin, Claude Code Langfuse env) send to the loopback OTLP port; agentq queues them with its own events. The Hermes Langfuse plugin sends directly; agentq never edits its `.env` and adds no intake path for it.
- Standalone mode: `agentq langfuse` reads host, public key and secret key (three lines, secret without echo); the queue posts straight to Langfuse. The host is `https://`, or `http://` on loopback only.
- Team mode: the accepted [connection contract](docs/spec/ENROLLMENT.md) amends §§ 1/5/6/8. Coreq enrollment uses DashQ's identity provider; the service sends allowlisted snapshots and eligible events. DashQ projection works without Langfuse; forwarding credentials stay server-side; standalone stays available.
- Events, one OTLP span each named after the event: `agentq.compress` (§ 3), `agentq.retrieval` (one MCP tool call), `agentq.installation` (heartbeat at service start and hourly), `agentq.incident` (§ 7), `agentq.resource`, `agentq.model`. dashq reads them for its Agents view.
- The start heartbeat waits until no agent reads `unknown`, at most 5 min, so a binding proxy is not exported as an hour of `unknown` routing. Routing, hook and backlog attributes: [TELEMETRY.md](docs/spec/TELEMETRY.md#heartbeat-attributes).
- Every event carries `agentq.schema` (1), `agentq.event_id` (also the trace ID, so a resend is idempotent), time, `agentq.stage`, `agentq.installation` (from `data/installation`), `host.name`, `agentq.runtime` (`macos`, `linux`, `wsl`, `windows`), `agentq.version`, engine versions, profile, agent and native IDs, else `agentq.ids_unknown` with a reason.
- Queue: one file per event `data/queue/<unix seconds>-<event id>.json`, capped at 64 MiB and 7 days on each write and send; overflow drops oldest and counts `dropped`. One service sender thread posts batches of at most 100 and deletes them after a standalone 2xx, or an explicit durable acceptance receipt when enrolled.
- `telemetry status` prints pending, oldest age, last delivery, dropped and the last error. `telemetry verify` queues one `agentq.verify`, sends now and reads it back for 60 s via `GET <host>/api/public/v2/observations?traceId=<id>`; a miss prints that query and its last answer (none: not ingested yet; 401: keys).
### Model calls
- **M6.5** One `agentq.model` span per native model call, stage `call`, schema 1, additive. Event ID = first 32 hex of SHA-256 of `<installation>:model:<call id>`, so a duplicate counts once and a later reading of the same call replaces it in the queue and in Langfuse.
- **M6.6** Model calls export only model name, IDs and integers; no prompt, output, tool input or path. agentq computes no price: only a client-reported cost sets `agentq.model.cost`, else `agentq.model.cost_unknown` is `not reported`. The span is a Langfuse `SPAN`, not a `GENERATION`, so Langfuse infers no price. A disabled integration reads and queues nothing.
- Producers ([TELEMETRY.md](docs/spec/TELEMETRY.md#model-calls)): Claude Code's `Stop` hook reads transcripts from a saved offset in bounded passes inside the hook budget and the service drains the rest; Hermes's `post_api_request` passes one allowlisted record per call; the Codex provider passes each finished response's usage in the background and keeps the stream byte-identical.
### Resource samples
- **M6.7** One sampler per installation, the service's sender (§ 5), on Linux, macOS, Windows and WSL; schema 1, additive. `windows` is the whole host and `wsl` the WSL 2 VM inside it; consumers never add a `wsl` installation to the `windows` one on the same `host.name`.
- **M6.8** No mount path, device or volume name, username, process or command output is exported; a filesystem `id` is a keyed hash. A failed, denied or missing probe omits its numbers and sets `agentq.resource.cpu_unknown`, `memory_unknown` or `disk_unknown` to a short reason.
- One sample per 300 s wall-clock bucket `floor(unix / 300)`, taken by the first sender pass (30 s pace) in a bucket; a start waits for the next bucket. Event ID = first 32 hex of SHA-256 of `<installation>:resource:<bucket>`, so a resend, second sampler or clock step replaces that trace, never adds one. CPU, memory and disk probes: [TELEMETRY.md](docs/spec/TELEMETRY.md#resource-samples).
### Profiling
- Profiling follows [specq profiling](https://github.com/taskqorg/specq/blob/main/engineering/profiling.md): it reuses the § 3 stats records and the queue `state.json`, adds no collector, process or network call, and stores operation names, numbers and class codes only. Operations, budgets, samples, trace switch and ranking: [TELEMETRY.md](docs/spec/TELEMETRY.md#profiling).

## 7. Incidents via coreq
- **M7.1** A report never changes agent work or an exit code.
- coreq incident reports, option A: an issue in `taskqorg/agentq`, label `incident`, through the user's existing github.com login; dedup by fingerprint; no relay and no GitHub App. Only built-in catalog faults report: engine start, update, routing, queue overflow, worker kill. Payload is coreq's allowlist JSON, at most 4 KiB, enumerated values only.
- No login, offline or unknown delivery: kept in `data/incidents/<fingerprint>.json` for manual submission. Opt-out: `incident_reports = false` in `config.json` or env `AGENTQ_INCIDENT_REPORTS=off`; setup discloses destination, visibility and account before any send.

## 8. State
- **M8.1** State other than local telemetry stays plain files with atomic rename and one lifecycle lock. Interrupted transactions resume or roll back from the journal on next start.
- `<install>/data/`: `owned.json` (journal), `config.json` (incident opt-out, mode, ports, `direct`, `compression`, `profile`), `installation`, `resource-key`, `langfuse` (fallback), `originals/`, `runs/`, `provider/`, `model/`, `stats/` and `traces/` (legacy history), `queue/` (with `state.json`: last delivery/error/dropped; old export samples kept), `incidents/`.
- Local telemetry uses `telemetry.sqlite3` (SQLite/WAL, FULL, 2 s busy timeout, 64 MiB including sidecars); stats/traces retain 7 days, exports 256 samples per operation. Preserve event IDs, atomic coverage, legacy history and optional export. OTLP events persist locally before optional delivery; compression uses its canonical stats row.
- `stats --records` exports retained metadata for local qualification. `telemetry export` emits versioned metadata-only JSONL with errors and unknown completeness; JSONL remains the legacy format.
- Dashboard duties (fleet view, versions, last seen, health, usage, incidents, access control) belong to dashq, which reads optional Langfuse events and authenticated snapshots under the connection contract. agentq ships no panel and no dashboard JSON.

## 9. Ownership and uninstall
- **M9.1** The journal records each created or reused component and each edit (file, key path, before value, after value) before mutation.
- **M9.2** `uninstall` restores owned values whose current value still equals the recorded after value, keeps later foreign edits and reports conflicts. `uninstall --dry-run` prints every restore and removal.
- `uninstall` stops and removes the service unit, engines, data and install folder. Logins, shared keys, reused components, exported history and the Codex direct definition (§ 4) stay. A second run says `already removed`.

## 10. Legacy Node install
- **M10.1** `uninstall --legacy` strips only legacy entries, after copying each listed file to `<legacy>/config-backup/`; a user edit made after the Node journal never blocks. Running legacy proxy pids are listed and left for the user.
- **M10.2** `onboard --yes` with a legacy install runs `uninstall --legacy` without a prompt (`--yes` is the consent), then applies § 4. A failure stops onboarding with its message.
- The Node package `@alexkirs/agent-context` is gone from this repository; installed copies keep working until this section removes them. `onboard` detects `~/.agent-context` and the global npm package; `onboard`, `doctor` and `uninstall --legacy` name running legacy pids.
- `uninstall --legacy` works without Node, unloads the `contextq-monitor` unit, removes the global npm package and retires `<legacy>`; when files are locked the removal still succeeds and `doctor` lists what is left. `npm-final/` holds the final package 0.5.0, which only prints the migration instruction and exits 1.
- Retired names remain recognized only for removal and the one-time namespace cutover (§ 4). Detection, removal procedure and retired repository parts: [LEGACY.md](docs/spec/LEGACY.md).

## 11. PRODUCT.md map
- Kept: R3 (§ 3; one compression core for every agent, only built-in qualified lists per adapter, others pass as `not_qualified`), R4 (§ 1 `direct`, § 3), R5 (§ 1 `onboard`, `enroll`, § 4; device enrollment served by dashq), R7 (§ 9, § 10), R9 (§ 6), R11 (Principles, § 5), R12 (Principles; this file is the approved spec revision).
- Mapped: R1 (§ 1, § 4, § 5; Desktop Chat: MCP only), R8 (§ 8, § 6; SQLite/WAL for local telemetry, other state stays files), R10 (§ 6, § 8; accounting kept, dashboard dropped from agentq, owned by dashq).
- Changed: R2 (§ 2, § 3; built-in search replaces context-mode, tool names unchanged); R6 (§ 5; coreq release trust, HTTPS and tags, replaces signatures; no staged rollout; worker swap kept for headroom).
- Not provided: relay and GitHub App (§ 7 posts through the user's login); gateway server, serviceq panel and dashboard (dashq owns them); SuperCompress, provider selection and Headroom compression (local tools replace them, no paid key); Ed25519 recipe signing and staged rollout (coreq trust, one release repo).

## Linked detail
- [docs/spec/PRODUCT.md](docs/spec/PRODUCT.md): product outcomes R1–R12.
- [docs/spec/VERIFICATION.md](docs/spec/VERIFICATION.md): minimal product acceptance and live check records.
- [docs/spec/ACCOUNTING.md](docs/spec/ACCOUNTING.md): optimization result definitions O1–O4.
- [docs/spec/COMPRESSION.md](docs/spec/COMPRESSION.md): compression entries, config, stats record and accounting coverage.
- [docs/spec/COMPRESSION-BENCHMARK.md](docs/spec/COMPRESSION-BENCHMARK.md): compression qualification checks, native client check rules and evidence limits.
- [docs/spec/CONFIGURATION.md](docs/spec/CONFIGURATION.md): profile projection and typed configuration A1–A5.
- [docs/spec/ENROLLMENT.md](docs/spec/ENROLLMENT.md): DashQ connection contract.
- [docs/spec/INTEGRATION-CONTROL.md](docs/spec/INTEGRATION-CONTROL.md): temporary control lifecycle, transitions, drain, status and revisions.
- [docs/spec/SERVICE.md](docs/spec/SERVICE.md): command registration, Windows service processes, update, status fields, host tools and release observation fields.
- [docs/spec/TELEMETRY.md](docs/spec/TELEMETRY.md): heartbeat attributes, model call mapping, resource probes and profiling.
- [docs/spec/LEGACY.md](docs/spec/LEGACY.md): namespace cutover procedure and legacy Node removal.
