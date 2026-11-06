# agentq contract

AgentQ is the single worker layer for native agents: prepare, launch, resume, observe, control, history and archive (§12). Products keep admission, grants, scheduling and UI. It ships one static binary and one runtime library on [coreq](https://github.com/taskqorg/coreq), with optional host recording and no server, relay or dashboard of its own.
This file holds every agentq decision; its linked detail files hold binding detail. Session archives are separate from metadata-only telemetry.

It follows the [specq release contract](https://github.com/taskqorg/specq/blob/main/engineering/release-contract.md) and [taskq.md](https://github.com/taskqorg/taskq/blob/main/taskq.md) § 4 and § 8 as their reference consumer. Product outcomes: [docs/spec/PRODUCT.md](docs/spec/PRODUCT.md) revision 6; § 11 maps each requirement here. Principles `P1`–`P5` and section rules `M<section>.<n>` are mandatory; text after them in a section is binding detail.

## Principles
- **P1 Never break the agent.** Any agentq failure keeps the original tool output and the client's own login, model, approvals and sandbox.
- **P2 Unverified means unknown.** Configured is not verified. Stale or missing values say so with a reason, never zero or success.
- **P3 Spec first.** A change that alters a decision edits this file or its linked detail first; code, README and INSTALL follow. Only the owner accepts a decision change. AGENTS.md points here.
- **P4 One static binary, one runtime library.** `agentq`, built from an empty Rust codebase (PRODUCT R11), and the Rust library `agentq-runtime` (§ 12) in this repository. No Node, npm or toolchain on the host. No Python engine or optimizer.
- **P5 Client only.** The client holds no relay, GitHub App, gateway server, serviceq panel or dashboard. Server duties belong to [dashq](https://github.com/taskqorg/dashq) (§ 6, § 8).

## 1. Commands
- **M1.1** Output: one line per outcome; errors `Error: <problem>` then `Next: <action>`, at most 2 lines. `--json` prints the same data as JSON.
- **M1.2** `agentq hook` never fails the client's hook: 5 s budget; a failure returns `{}` and exits 0. It runs in the launcher process itself, so a release mid-update or a failed start cannot fail it.
- **M1.3** `agentq doctor` is read-only and exits 1 on a problem. Absent optional Langfuse keys alone do not fail local health.
- **M1.4** Offline `selftest` never starts paid client calls.

| Command | Does |
|---|---|
| `agentq onboard [agent..]` | Read-only discovery: ready Codex, Claude Code, Claude Desktop, Hermes and Antigravity profiles per runtime; missing input; integration conflicts. No writes. |
| `agentq onboard --yes [agent..]` | Install the release, apply § 4 record hooks for the selected agents, then (re)start the service and run `doctor`. Verify recording by a real native turn. Repeat is safe. |
| `agentq enroll <https url> [--headless]` | DashQ provider-backed browser approval (headless: link and code only); protected observation grant and projected snapshot receipt (§ 6). Released before qualification #95. |
| `agentq connection status [--json]`, `agentq disconnect`, `agentq langfuse` | Connection, projection, export and revocation state with the next action; disconnect stops sending at once and retries the revoke. Released before qualification #95. `langfuse`: standalone mode, reads Langfuse host and keys from hidden stdin into protected storage (§ 6). |
| `agentq agent host --action <action> --request <json>` | Independent protected host prepare, launch, observe, cancel and reconcile under existing authority (§ 12); no admission service. |
| `agentq agent history` | Read native session steps as JSON pages; runtime and session on stdin, caller-selected byte window and checkpoint (§ 12). |
| `agentq agent turn` | Run one Codex, Claude or Antigravity turn through `agentq-runtime` from a JSON request on stdin; prints the JSON report (§ 12). Needs no service, engine, MCP server or install. |
| `agentq agent preflight` | Check one launch request from JSON on stdin and print its readiness report; `--repair` applies only M12.16 repairs. No model call (§ 12). Target: #284. |
| `agentq hook <agent> <event>`, `agentq model codex` | Hook handler for § 4 events, JSON on stdin and stdout (M1.2); record entry for one finished Codex model call, JSON on stdin, queues `agentq.model` (§ 6). |
| `agentq profile [--json]`, `agentq profile trace on [<minutes>]\|off` | Ranked profile of measured operations against their budgets; detailed trace switch (§ 6). |
| `agentq direct [agent..]` | Restore journal-owned legacy routing only; preserve record hooks, telemetry and the exact direct Codex definition for saved chats. Works with the service stopped (PRODUCT R4). Changed direct-provider receipts can be released explicitly; unresolved or unowned dead routes exit nonzero with exact manual edits. |
| `agentq disable\|enable [codex\|claude\|hermes\|claude-desktop\|antigravity] [--dry-run]` | Reversibly detach selected owned integrations or restore the saved supported fields; data and installation stay. No target selects the installation; enable without a saved disable is unchanged (§ 4). |
| `agentq status [--json]` | Fresh local integration and service state, including intentional disable, pending reload, aliases, conflicts and observation time; works with the service stopped. |
| `agentq configure [<profile> <field>=<value>.. [--yes]]` | Profile projection, typed plan, or guarded apply with host readback under the lifecycle lock (§ 4). Released before qualification #95. |
| `agentq doctor`, `agentq telemetry status\|verify [agent]\|export`, `agentq incidents [list\|show]` | Health of install, host, service and agents (M1.3); queue status, synthetic read-back check and local JSONL export (§ 6); local incident reports and what would be sent (§ 7). |
| `agentq update [--check]`, `agentq selftest`, `agentq service install\|run\|pause\|resume\|status\|restart\|stop\|uninstall\|list` | Transactional update and coreq service (§ 5); selftest checks the binary, built-in fixtures, a transactional install of its own bytes and local telemetry. |
| `agentq uninstall [--dry-run]` | § 9; none left: `already removed`. |

- Each `hook` run stamps the event time as configuration evidence. A run that cannot read stdin or write stdout records `data/evidence/<agent>.<event>.failed` with its time and error. A successful run moves it to `<agent>.<event>.last_failure`, so `.failed` exists exactly while the latest run failed; the last failure stays for the heartbeat (§ 6).
- `doctor` checks install, host tools (§ 5), service, owned edits count, local SQLite integrity, telemetry queue and optional keys. Each recorded hook failure fails, an unreadable record included.
- `doctor` and `status` report a selected legacy `agentq` loopback provider as `legacy_agentq_route`, or missing provider as a problem, with next step `agentq direct`; unreadable config means unknown. Cleanup runs at service start; a stopped or disabled host needs explicit `direct` cleanup. Service status and heartbeat carry the diagnosis too.

## 2. Retired retrieval tools
- **M2.1** AgentQ registers no MCP document tools. `get_original` retires with compression; local measured use of `index_document` and `search_document` is recorded in [OPTIMIZATION-RETIRED.md](docs/spec/OPTIMIZATION-RETIRED.md).

## 3. Retired optimization
- **M3.1** No compression core, RTK wrapper, Headroom, provider adapter, proxy, routing port, drain or Python host requirement remains. Native commands and results stay native.
- Measurements and the sole return condition: [OPTIMIZATION-RETIRED.md](docs/spec/OPTIMIZATION-RETIRED.md).

## 4. Hooks and config edits per runtime
- **M4.1** Each edit is recorded in the ownership journal (§ 9) before it is written.
- **M4.2** Existing foreign optimizers or custom providers stop the edit for that file with a named conflict; nothing is replaced by guess.
- **M4.3** New integrations never route model traffic or register optimizer tools. Each service start retires journal-owned legacy routing, MCP, wrapping hooks and instruction blocks before export or configuration reconciliation; unchanged Hermes plugin files follow the record-only release.
- Retirement is idempotent, includes pre-v0.8 receipts, preserves foreign edits and never re-enables disabled profiles.
### Edits per runtime
| Runtime | Files and edits |
|---|---|
| Codex CLI and Desktop | `hooks.json`: `SessionStart` and `Stop` run `agentq hook codex <event>`; native authentication and provider stay unchanged. |
| Claude Code CLI and Desktop Code | `settings.json`: `SessionStart` and `Stop` record hooks only; no `ANTHROPIC_BASE_URL`, wrapping hooks or MCP edits. |
| Claude Desktop Chat | No integration is installed. |
| Hermes | Shipped plugin records `post_api_request` and `on_session_end`; no tool-result transformation or MCP. |
| Antigravity | `config/hooks.json`: named `agentq` group with a flat `Stop` handler only; foreign groups and later edits stay. |

- Config roots: the client's own env (`CODEX_HOME`, `CLAUDE_CONFIG_DIR`, `HERMES_HOME`), else the default home folder; project-level overrides are reported, never edited. Native Windows and WSL are separate installs.
- A JSON config that starts with a UTF-8 BOM is parsed without it; the edit writes it without the BOM and its `edited:` line says `UTF-8 BOM stripped`. Uninstall restores the original bytes, BOM included.
- [CONFIGURATION A1–A5](docs/spec/CONFIGURATION.md) owns profile projection and separately authorized typed management with revision checks, journaled apply, host readback and real-client verification. Observation enrollment grants no configuration authority.
- Typed envelopes and host acceptance: `agentq/configure.rs` with fixtures `docs/spec/configuration-fixtures.json`. Local guarded apply, readback and hook evidence: `agentq configure`, `agentq/executor.rs` (CONFIGURATION "Host executor"). Remote operations are released before qualification #95.
### Temporary controls
- **M4.4** Integration enable/disable controls record hooks independently of native agent execution.
- **M4.5** Retained optimizer data is not rewritten or deleted by this retirement; uninstall retains its explicit removal scope.
- **M4.6** Full disable persists intent before removing owned routing, MCP, hooks, Hermes plugin files/activation and journaled instruction blocks. It keeps the installation, release assets, local data, originals, keys, grants and queued observations under their existing retention.
- **M4.7** Only explicit `enable` clears disable intent; onboarding, service install/restart and configure cannot reconnect a disabled profile (`integration_disabled`). Manual binary update remains available without reactivation. `direct` keeps its routing-only meaning.
- **M4.8** Corrupt or unreadable lifecycle state (`data/integration.json`) means unknown and blocks mutations and integration edits rather than resetting state.
- **M4.9** Detaching configuration does not unload a running client's cached provider, hooks or MCP. agentq preserves active work and reports reload evidence separately.
- Removing owned Codex routing (`disable`, `direct`, uninstall) restores the native default selection but keeps `agentq` as an exact, marked direct OpenAI definition without a base URL or AgentQ dependency, because Codex conversations persist their provider ID. Codex selects its native endpoint from the unchanged login/auth mode.
- That direct definition is not an attached optimization. The compatibility block is never replaced with loopback routing; later edits remain conflicts. It stays after uninstall, and previously detached installations can be repaired with it when the name is absent.
- While a host disable is pending, the service runs only required transport and recovery: export, automatic update activation and remote configuration application are suspended. The enrolled DashQ snapshot keeps its 60 s cadence with lifecycle `pending_disable` (§ 5), so DashQ can name the host disabled.
- Resume is never repaired by rewriting conversation history, provider IDs in stored chats, authentication, model, approvals or sandbox. Native login, model, approvals, sandbox and foreign telemetry settings are outside disable and enable.
- Native qualification is per client and platform; code checks alone do not qualify working profiles. Earlier rollout records in the historical control plan apply only to their cited releases; unrun Desktop and reboot checks remain unknown.
- Under disable, foreign entries and later edits remain and conflicts are explicit; shared roots and registrations switch together. Lifecycle state, transitions, locks, service handling, status and revisions: [INTEGRATION-CONTROL.md](docs/spec/INTEGRATION-CONTROL.md).
### Namespace
- **M4.10** The retained Codex provider and Hermes record plugin are named `agentq`. No MCP server or old namespace alias is registered. Saved chats retain their provider ID and resume through the exact direct definition; histories stay intact.

## 5. Install layout and service
- **M5.1** coreq owns layout, update, launcher, rollback, service units and `status.json`; agentq passes tool name `agentq`, install folder and release repository `taskqorg/agentq`.
- **M5.2** One OS unit per host and runtime runs the service; there is no other resident process. More than one `service run` is an error naming every pid.
- **M5.3** Release trust is HTTPS plus who can push tags; installers and self-update check only the sha256. A failing selftest publishes nothing.
- **M5.4** agentq has no administrator rights and installs only its own release. Data in `data/` changes only additively (compatibility N/N-1).
### Layout, release and service
- Binding [layout, release artifacts, service ownership, installation, update and status](docs/spec/SERVICE.md#installation-and-service-layout).
### Host requirements
- agentq runs only the external tools in the [host tools table](docs/spec/SERVICE.md#host-tools) (specq [host requirements](https://github.com/taskqorg/specq/blob/main/engineering/release-contract.md#host-requirements)); There are no managed optimization engines. A change that starts using a new tool, flag or API raises its row in the same change.
- `doctor` checks each row: missing, unreadable or below its minimum fails as `host tool <name>: <found> below minimum <min>; <fix>`; OS components are checked for presence on PATH in one line; optional tools report without failing. `service install`, `onboard` and each `service run` start report every failing requirement with its fix step; `doctor` stays failed until the host provides it.
- Every host tool call goes through one wrapper. A failed start, and a failed exit where success is required, is logged as `` agentq: `<command>`: <error> `` to stderr (the service log for `service run`) and returned as an error, never as an empty result ([exceptions](docs/spec/SERVICE.md#host-requirement-notes)).
### DashQ release observation
- The additive `release` object in `status.json`, snapshot schema 1 and the heartbeat (`agentq.release`, JSON) is installation-scoped observation of `selected`, `running`, `coreq_version`, `health` and `lifecycle` ([fields](docs/spec/SERVICE.md#release-observation-fields)). Installation identity and source `at` bind it to this producer; it grants no control authority.
- `health` proves only that the running release wrote status, never routing, workload or client health, and never the selected version when it differs. No elapsed-time or socket-only detach proof is added. No paths, arbitrary errors or native configuration are exported.

## 6. Telemetry to Langfuse
- **M6.1** Writers never wait for the network: an outage, missing keys or an unwritable queue never blocks or fails agent work, and the queue drains on the next successful pass.
- **M6.2** Metadata only: no raw content, arguments, keys or paths. Retained historical optimization events remain readable.
- **M6.3** Unknown is never zero: a missing value omits its attribute and names the reason. Estimates carry an `estimate` label and never count as observed usage.
- **M6.4** Secrets live in Keychain (`security`), Secret Service (`secret-tool`) or a 0600 file `data/langfuse` where neither works (Windows until a Credential Manager binding lands); never in config, argv, logs or telemetry. curl gets them through its stdin config.
- Target: Langfuse project `Agents` over OTLP HTTP JSON, `<host>/api/public/otel/v1/traces` with Basic auth. AgentQ queues its own native record events; it installs no loopback OTLP intake or native exporter overrides. The Hermes Langfuse plugin sends directly; agentq never edits its `.env` and adds no intake path for it.
- Standalone mode: `agentq langfuse` reads host, public key and secret key (three lines, secret without echo); the queue posts straight to Langfuse. The host is `https://`, or `http://` on loopback only.
- Team mode: the accepted [connection contract](docs/spec/ENROLLMENT.md) amends §§ 1/5/6/8. Coreq enrollment uses DashQ's identity provider; the service sends allowlisted snapshots and eligible events. DashQ projection works without Langfuse; forwarding credentials stay server-side; standalone stays available.
- Events, one OTLP span each named after the event: `agentq.installation` (heartbeat at service start and hourly), `agentq.incident` (§ 7), `agentq.resource`, `agentq.model`. dashq reads them for its Agents view.
- Every event carries `agentq.schema` (1), `agentq.event_id` (also the trace ID, so a resend is idempotent), time, `agentq.stage`, `agentq.installation` (from `data/installation`), `host.name`, `agentq.runtime` (`macos`, `linux`, `wsl`, `windows`), `agentq.version`, profile, agent and native IDs, else `agentq.ids_unknown` with a reason.
- Queue: one file per event `data/queue/<unix seconds>-<event id>.json`, capped at 64 MiB and 7 days on each write and send; overflow drops oldest and counts `dropped`. One service sender thread posts batches of at most 100 and deletes them after a standalone 2xx, or an explicit durable acceptance receipt when enrolled.
- `telemetry status` prints pending, oldest age, last delivery, dropped and the last error. `telemetry verify` queues one `agentq.verify`, sends now and reads it back for 60 s via `GET <host>/api/public/v2/observations?traceId=<id>`; a miss prints that query and its last answer (none: not ingested yet; 401: keys).
### Model calls
- **M6.5** One `agentq.model` span per native model call of a session no launcher records (M6.9), stage `call`, schema 1, additive. Event ID = first 32 hex of SHA-256 of `<installation>:model:<call id>`, so a duplicate counts once and a later reading of the same call replaces it in the queue and in Langfuse.
- **M6.6** Model calls export only model name, IDs and integers; no prompt, output, tool input or path. agentq computes no price: only a client-reported cost sets `agentq.model.cost`, else `agentq.model.cost_unknown` is `not reported`. The span is a Langfuse `SPAN`, not a `GENERATION`, so Langfuse infers no price. A disabled integration reads and queues nothing.
- **M6.9** Each agent turn is counted once. `agentq-runtime` (§ 12), or the frozen CoreQ `agent` during adoption, records the turns of sessions that TaskQ, PilotQ or another launcher start (`agent.turn`). agentq queues no `agentq.model` span for such a launched session.
- **M6.11** A manual or PM session writes the same `agent.call` and `agent.turn` records as a launched one to the CoreQ local store `data/observations.sqlite3`, through the Claude, Codex and Antigravity `Stop` hooks and the Hermes `post_api_request` and `on_session_end` hooks. A launched session writes none (M12.13). Metadata only (M6.2).
  The `agentq.model` and OTLP path keeps running until a later task shows parity and deletes it. Open gaps and [detail](docs/spec/TELEMETRY.md#session-records).
- A session is launched when its client environment carries a non-empty `TASKQ_TASK` (TaskQ) or `AGENTQ_LAUNCHER` (any other launcher, PilotQ included). Native results stay unchanged.
- **M6.10** A span of a session no launcher records (desktop, PM or manual sessions) carries `agentq.project`, the `owner/name` code of the nearest `taskq.json` `repo` at or above the session's working directory. Without such a project it sets `agentq.project_unknown` to a short reason. No path is exported (M6.2).
- Producers ([TELEMETRY.md](docs/spec/TELEMETRY.md#model-calls)): Claude Code's `Stop` hook reads transcripts from a saved offset in bounded passes inside the hook budget and the service drains the rest; Hermes's `post_api_request` passes one allowlisted record per call; Codex Stop reads its native transcript.
### Resource samples
- **M6.7** One sampler per installation, the service's sender (§ 5), on Linux, macOS, Windows and WSL; schema 1, additive. `windows` is the whole host and `wsl` the WSL 2 VM inside it; consumers never add a `wsl` installation to the `windows` one on the same `host.name`.
- **M6.8** No mount path, device or volume name, username, process or command output is exported; a filesystem `id` is a keyed hash. A failed, denied or missing probe omits its numbers and sets `agentq.resource.cpu_unknown`, `memory_unknown` or `disk_unknown` to a short reason.
- One sample per 300 s wall-clock bucket `floor(unix / 300)`, taken by the first sender pass (30 s pace) in a bucket; a start waits for the next bucket. Event ID = first 32 hex of SHA-256 of `<installation>:resource:<bucket>`, so a resend, second sampler or clock step replaces that trace, never adds one. CPU, memory and disk probes: [TELEMETRY.md](docs/spec/TELEMETRY.md#resource-samples).
### Profiling
- Profiling follows [specq profiling](https://github.com/taskqorg/specq/blob/main/engineering/profiling.md): it reuses local export samples and the queue `state.json`, adds no collector, process or network call, and stores operation names, numbers and class codes only. Operations, budgets, samples, trace switch and ranking: [TELEMETRY.md](docs/spec/TELEMETRY.md#profiling).
### Session archive
- **M6.12** Native session content uses a separate archive store and shipper with explicit `agent-history/1` approval, off by default. Observation grants never imply capture. Launched workers and managers are captured; manual sessions require a separately approved per-host `agent-history.manual` opt-in.
- **M6.13** Archive failures never block or fail work. Raw native bytes, with best-effort secret scrubbing and replacement counts, remain authoritative; steps are derived at read time. Sink content retention is 365 days; index and metadata do not expire. [Archive detail](docs/spec/ARCHIVE.md).

## 7. Incidents via coreq
- **M7.1** A report never changes agent work or an exit code.
- coreq incident reports, option A: an issue in `taskqorg/agentq`, label `incident`, through the user's existing github.com login; dedup by fingerprint; no relay and no GitHub App. Only built-in catalog faults report: update and queue overflow. Payload is coreq's allowlist JSON, at most 4 KiB, enumerated values only.
- No login, offline or unknown delivery: kept in `data/incidents/<fingerprint>.json` for manual submission. Opt-out: `incident_reports = false` in `config.json` or env `AGENTQ_INCIDENT_REPORTS=off`; setup discloses destination, visibility and account before any send.

## 8. State
- **M8.1** State other than local telemetry stays plain files with atomic rename and one lifecycle lock. Interrupted transactions resume or roll back from the journal on next start.
- `<install>/data/`: `owned.json` (journal), `config.json` (incident opt-out and `profile`; legacy optimizer keys stay unused), `installation`, `resource-key`, `langfuse` (fallback), `model/`, `sessions/` (M6.9), `observations.sqlite3` and `observe/` (M6.11), `queue/` (with `state.json`: last delivery/error/dropped), `incidents/`.
- Legacy `originals/`, `runs/`, `provider/`, `stats/`, `traces/` and `name-migration.json` stay retained and unused until `uninstall` removes `data/`.
- Local telemetry uses `telemetry.sqlite3` (SQLite/WAL, FULL, 2 s busy timeout, 64 MiB including sidecars); stats/traces retain 7 days, exports 256 samples per operation. Preserve event IDs, atomic coverage and optional export. Events persist locally before optional delivery; historical stats rows stay readable.
- `telemetry export` emits versioned metadata-only JSONL with errors and unknown completeness.
- Dashboard duties (fleet view, versions, last seen, health, usage, incidents, access control) belong to dashq, which reads optional Langfuse events and authenticated snapshots under the connection contract. agentq ships no panel and no dashboard JSON.

## 9. Ownership and uninstall
- **M9.1** The journal records each created or reused component and each edit (file, key path, before value, after value) before mutation.
- **M9.2** `uninstall` restores owned values whose current value still equals the recorded after value, keeps later foreign edits and reports conflicts. `uninstall --dry-run` prints every restore and removal.
- `uninstall` stops and removes the service unit, data and install folder. Logins, shared keys, reused components, exported history and the Codex direct definition (§ 4) stay. A second run says `already removed`.

## 10. Retired Node install
- **M10.1** agentq does not detect, name or remove the retired Node package `@alexkirs/agent-context`, its `~/.agent-context` folder or the retired namespace. Files left there stay untouched.

## 11. PRODUCT.md map
- R1/R5/R7: optional host installation, record hooks, ownership and removal (§§1/4/5/9).
- R2/R3: explicitly retired retrieval and optimization (§§2/3); the linked retirement decision owns the parked note.
- R4: preserve native work, saved-chat provider compatibility and authority (§§4/12).
- R6/R8/R9/R10: transactional updates, local lifecycle state, attributable call/turn observations, honest unknowns and optional DashQ projection (§§5/6/8/9/12).
- R11/R12: static binary, reusable library, shared primitives and spec-first changes (Principles).
- R13: one worker layer, per-runtime registry, policy, recovery, control and history (§12); history/archive ownership follows [specq agent runtimes](https://github.com/taskqorg/specq/blob/main/engineering/agent-runtimes.md#step-history-and-archive).
- R14: ordered worker preflight and owned repairs (§12); native qualification remains separate.

- R15: separately granted, nonblocking session archive (§6).

## 12. Agent runtime
- **M12.1** `agentq-runtime` (folder `runtime/`, Rust library `agentq_runtime`) starts, resumes, cancels and observes Codex CLI, Claude Code and Antigravity (`antigravity`) turns: CLI and ACP turns, Claude background jobs, Codex app and app-server sessions, Remote Control, session recovery, dynamic tools, private drafts, the model catalogue and account selection.
- **M12.2** It depends only on CoreQ primitives (process, telemetry store, `quiet`); never on TaskQ, PilotQ or the optimizer modules of `agentq`. CoreQ never depends on it.
- **M12.3** A plain turn needs no service, engine, MCP server or installation. The library and `agentq agent turn` never change installation or native configuration; an explicit archive grant permits asynchronous private capture only.
  Callers keep grants, permissions, model, effort, input files, storage, process ownership, stop authority and UI.
- **M12.4** Each finished turn appends exactly one attributable `agent.turn` observation to the caller's CoreQ store; recovery and repeated observation never count it twice. The child gets `AGENTQ_LAUNCHER` unless `TASKQ_TASK` or `AGENTQ_LAUNCHER` is set, so § 6 M6.9 holds.
- **M12.5** Compatibility API `Intent::{Inherit,Off,On}` and optimization reports remain until TaskQ, PilotQ and Pipe adopt the retired feature. Every intent is a no-op reporting `off`, reason `optimization_retired`; no installation or native configuration is read.
- **M12.6** A launch never edits global or client configuration, previously transmitted replay bytes, provider or model. Optimization intent adds no environment or provider-header override.
- **M12.7** Native output is unchanged; a runtime failure stays a runtime failure.
- **M12.8** Protected app-server launches optionally use a caller-trusted signed host adapter, existing grants and authenticated caller-owned permits; no TaskQ binary, command, slot service or certificate authority is required. Non-launching preparation binds the actual caller and existing authority.
  Launch revalidates preparation. Fresh readback binds project/profile, runtime/account/image, approver/revision, operation and native incarnation before any input. Unknown or changed authority refuses; uncertain work is observed, never replayed.
  Durable native/descendant completion and cleanup precede original-permit release; unknown retains occupancy. Retained operations recover without relaunch, receipt rewriting or TaskQ fallback.
  AgentQ verifies the adapter’s pinned SHA-256 and dedicated Ed25519 signature on macOS and Windows; private keys stay outside the repository. This provides neither macOS signature checks nor TCC grants. Detail: [RUNTIME.md](docs/spec/RUNTIME.md#protected-app-server).
- **M12.9** Callers reach runtimes only through the `agent::Runtime` trait and its `Descriptor` data ([specq agent runtimes](https://github.com/taskqorg/specq/blob/main/engineering/agent-runtimes.md)). Per-runtime adapters own every Claude, Codex and Antigravity flag, file, environment, session format, quota and login source; client identity, native transports and native parsers are private.
  Registered protocol callbacks select native interpretation; shared code owns runtime-neutral process, record and recovery mechanics.
  Consumer adoption follows the completed runtime contract; source API changes add no compatibility paths or consumer opt-outs.
  Read-only log events, background-job evidence, exhausted windows and account identity stay inside the adapters.
  Callers state needs in `Launch` and read typed events; the adapter chooses the transport ([one run path](docs/spec/RUNTIME.md#one-run-path)).
- The registry is the sole catalog for runtime descriptors and aliases, native policy/login methods, configuration roots and files, setup edits, hook actions and profile inspection. Each native client owns one module; Desktop Chat has its own integration-only entry.
  Shared execution selects registered methods. Runtime modules own event, report, transcript and quota parsing; native identities are registration data, not variants in a core enum.
- Registry modules are discovered at build time. Adding a module adds its entry without editing a core list; a scripted fake demonstrates launch, events and report through one added runtime module.
  A shared ACP adapter owns protocol state; registration supplies native launch data and differences. Antigravity uses the installed `agy acp`; legacy native seams remain until cross-platform comparison passes ([ACP sessions](docs/spec/RUNTIME.md#acp-sessions)).
  Required callbacks are typed; validation rejects incomplete metadata and duplicate identities. Behavioral checks compare declared and undeclared capabilities with their operations.
  Setup checks CLI presence before native writes; discovery skips missing CLIs, while explicitly selected missing clients refuse.
- **M12.10** Permissions arrive as an abstract `Policy`; adapters map it to native flags and refuse launch for a part they cannot enforce. An operation a runtime lacks returns a named cause, never a fake success.
  `Policy` also covers never-prompt approval (Claude `dontAsk`), exact or added Codex writable roots, unelevated Windows sandbox, built-in tools, isolation from user settings and subscription-only login.
  Readiness checks the native login and minimum version. ACP releases with known workspace-write defects require a verified safe version before launch; an unreadable version blocks that check. Trait, descriptors, policy map and causes: [RUNTIME.md](docs/spec/RUNTIME.md#runtime-trait).
- **M12.11** A CLI turn run checkpoints and recovers without a native call: the recovered run observes the still-running child by native process identity and log offset, stops it only through that identity and reads an exit it did not observe as unknown.
  `Launch.process` hooks run on the child `Command` after the adapter's own setup and on the spawned `Child` before input, so caller creation flags, process groups and Job Objects survive ([detail](docs/spec/RUNTIME.md#turn-recovery-and-process-setup)).
- **M12.12** Three caller inputs and outputs cross the adapter boundary as opaque data. `Launch.host_args` are operator-configured Codex turn arguments; they refuse a non-default `Policy`, because they could override any part of it.
  `Launch.settings_env` is session environment: Claude `--settings` `env` (values enter argv, so never secrets) or the Codex child. `Runtime::launcher_output` hands over the raw Claude background launcher exchange once ([detail](docs/spec/RUNTIME.md#caller-launch-data)).
- **M12.13** Under the [specq telemetry contract](https://github.com/taskqorg/specq/blob/main/engineering/profiling.md#telemetry-contract), `record` also appends one `agent.call` per model request with the turn, in the caller's CoreQ store, for every runtime and mode; products add no capture. A client reporting no tokens reads null with a reason, never zero.
  A finished run's records are frozen at its terminal observation: `Run::snapshot` returns them without a store lock, and later log bytes or queue delay never change them. [Detail](docs/spec/RUNTIME.md#deferred-persistence).
  Records and runtime children inherit the caller's CoreQ `project`, `task`, `attempt`, `actor` and `entry`; a missing value reads null with a reason. [Detail](docs/spec/RUNTIME.md#observation).
- **M12.14** `agent.turn` also carries the exit code, session id, user and system CPU and peak RSS. Peak RSS is the largest sampled sum of the live processes of the tree; per-process peaks are never summed. A process the run does not own reads null with a reason.
- **M12.15** A worker gets its input only after preflight: client, account, capacity, workspace, permissions, inputs and storage are checked, in that order, with no model call. A blocked check refuses before spawn with a named cause and next step; no tokens are spent and no work starts half ready.
  Unknown continues by default and stays visible, except the mandatory ACP safe-version check (M12.10), because a failed optional probe alone must not stop work; a caller may choose strict. A failure of AgentQ's optional parts (hooks, telemetry, archive) degrades evidence, never blocks work (P1). Target: #284. [Detail](docs/spec/RUNTIME.md#worker-preflight).
- **M12.16** Preparation fixes only what it owns, on explicit request, outside `launch`: exact-path client trust and AgentQ's own hook trust, under the client's lock and journaled. M12.6 holds for `launch` itself. It never changes login, account, model, permissions, provider, foreign hooks, or trust for a home or root folder.
- **M12.17** A worker never silently waits for a person: the child has no interactive prompt path, and a run reads started only on native proof of a session; until then it reads starting, visibly. A native refusal ends it with a named cause. Input already written is never resubmitted.
- **M12.18** Every live or finished session has read-only typed steps through `Runtime::history` and `agentq agent history`. Raw source bytes remain authoritative; stable step IDs link revisions, unknown records remain visible, and full output reads by source byte range. [Detail](docs/spec/RUNTIME.md#universal-step-history).
- **M12.19** `Runtime::stop_event` exposes an exit without native turn-end, signal, lost owner or a turn-end with a live process, with the last activity and its source. Unknown exit stays unknown. Native transport liveness and persisted run identity bind each stop; observation grants no completion or cleanup authority. [Detail](docs/spec/RUNTIME.md#stopped-run-observation).
- **M12.20** History exposes context, limit windows, per-call usage and cost source, plan, todos and per-turn file changes, plus native presentation and typed hook/queue/metadata facts. Missing facts stay unknown. Claude statusline input is read without polling its usage endpoint or editing user statusline settings; telemetry receives metadata only and never recounts tokens.
  Latest result cost never borrows from an older result. Usage identities retain native turn scope; unassociated overlap keeps accounting unknown, and only positively associated same-turn snapshots retire. [Detail](docs/spec/RUNTIME.md#live-run-data).
- **M12.21** History exposes media descriptors and byte-range reads within native session sources; image generation and saved-path evidence survive runtime events. Raw bytes and content hashes remain authoritative; native references survive physical revisions and generated assets cannot cross session roots. [Detail](docs/spec/RUNTIME.md#history-media).
- **M12.23** Explicit account sign-in uses native CLI authority through the runtime: start, relay the native link/code, submit a pasted code, status and guarded cancel. Credentials stay native; selected homes never fall back or roll back. No login runs during launch or repair. [Detail](docs/spec/RUNTIME.md#account-sign-in).
- Binding [runtime operation, observation, native lookup, benchmarks and adoption](docs/spec/RUNTIME.md#runtime-operation-and-adoption).

## Linked detail
- [docs/spec/PRODUCT.md](docs/spec/PRODUCT.md): product outcomes R1–R14.
- [docs/spec/VERIFICATION.md](docs/spec/VERIFICATION.md): minimal product acceptance and live check records.
- [docs/spec/ACCOUNTING.md](docs/spec/ACCOUNTING.md): archived optimization definitions O1–O4, not current acceptance.
- [docs/spec/OPTIMIZATION-RETIRED.md](docs/spec/OPTIMIZATION-RETIRED.md): retirement evidence, MCP usage and return condition.
- [docs/spec/COMPRESSION-BENCHMARK.md](docs/spec/COMPRESSION-BENCHMARK.md): retired compression qualification history and evidence limits.
- [docs/spec/CONFIGURATION.md](docs/spec/CONFIGURATION.md): profile projection and typed configuration A1–A5.
- [docs/spec/ENROLLMENT.md](docs/spec/ENROLLMENT.md): DashQ connection contract.
- [docs/spec/INTEGRATION-CONTROL.md](docs/spec/INTEGRATION-CONTROL.md): temporary control lifecycle, transitions, status and revisions.
- [docs/spec/SERVICE.md](docs/spec/SERVICE.md): install layout, release artifacts, command registration, service processes, update, status, host tools and release observation.
- [docs/spec/TELEMETRY.md](docs/spec/TELEMETRY.md): heartbeat attributes, model call mapping, resource probes and profiling.
- [docs/spec/ARCHIVE.md](docs/spec/ARCHIVE.md): separate grants, native capture, scrubbed byte identity, checkpoints, exclusions and archive status.
- [docs/spec/RUNTIME.md](docs/spec/RUNTIME.md): agent runtime arguments, end detection, usage, limits, accounts, compatibility modes, the `agentq agent` JSON contract, optimization intent, consumer inventory and adoption boundaries.
