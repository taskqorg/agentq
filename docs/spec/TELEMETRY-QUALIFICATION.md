# Telemetry-led compression qualification

Acceptance procedure for existing PRODUCT R1-R5/R9-R12, VERIFICATION A2/A4/A6 and ACCOUNTING O1-O4. Reuse installation-local SQLite and existing instrumentation; no second collector, service, telemetry store, optimizer, remote sink or experiment runner. Reuse `check-client-compression.py`, native exporters, `agentq.compress`, the service queue, Langfuse `Agents`, and `taskq profile export` for TaskQ-owned attempts. A procedure is not proof all platforms passed.

## Contract sources loaded

- AgentQ `agentq.md` sections 3, 4, 6; PRODUCT revision 2; ACCOUNTING O1-O4; CONFIGURATION A2/A3/A5. Baseline `dbc57f094baf8930bd71370e55e87e6da69941ca` (0.6.2).
- TaskQ `taskq.md` R8/R9/R12/R13 and sections 4/8; `docs/design/profiling.md`; `docs/runtime-qualification.md`; `docs/design/standalone-telemetry.md`. Read from main `56659e68d15be2174f89abe56e1cf15dc1d75ac0`; the local checkout is older and dirty, with missing linked files. Do not reset it. Standalone retention/guide targets are not assumed shipped.
- TaskQ stages hold context input including cache, not a cost breakdown. A direct client session is not a TaskQ attempt. TaskQ standalone records stay local; do not assume Langfuse contains them or add a second upload path.

## Matrix and version identity

[Machine-readable matrix](telemetry-qualification-matrix.json): twelve primary cells: macOS/native Windows/Linux times Codex CLI/Desktop and Claude Code CLI/Desktop Code. Expand actual host architecture, shell, native/WSL runtime and account mode; Windows and WSL are separate installations. Identify app build and embedded runtime separately. Record exact AgentQ/engine/client/config versions, installation/profile/revision and native session/tool IDs. Shared config does not prove a shared tested runtime.

Use TaskQ's runtime adapter only where its contract supports it. A real Desktop turn is required to qualify Desktop; a CLI, app handshake, Remote Control or model API call cannot substitute. For a Desktop without supported automation, execute the same fixture in the actual app and read its native telemetry. Missing UI/native session evidence stays unknown.

Claude Desktop Chat is MCP-only under PRODUCT R1/CONFIGURATION A1: qualify index/search/get_original separately; automatic terminal-output compression is not applicable. Cowork/cloud/SSH run elsewhere and require their own runtime identity; a local pass does not qualify them.

A pass is for exact recorded versions and configuration. Client/app/engine/adapter/permission changes invalidate the affected qualification until a fresh real turn passes. Do not promise all past or future releases.

## Mandatory local collection; optional export

Collection is always on and independent of enrollment, Langfuse keys or a serverq connection. The local test is `check-client-compression.py`: native execution, the matching core stats record, model-visible ref/error, exact recovery and available native usage. AgentQ stats/accounting/profile and TaskQ profile/export are the existing analysis interfaces. No connection is needed to run them.

`check-compression-telemetry.py` is an OPTIONAL connected-mode delivery check over the same records. Its central export/usage verdict never gates an unconnected installation's local collection/compression verdict. Missing optional server credentials are not a local telemetry failure. Absence of an event for an operation that actually ran is a collection gap; an unavailable client usage field is unknown, never zero.

TaskQ's common `profile::measure` wrapper records a named operation on scope exit with duration/outcome and optional trace identity; service/CLI initialization does not depend on a connection. Existing instrumentation covers the six product operation names, board kinds, queue transitions and attempt stages. This does not prove every Rust helper is automatically instrumented. Verify each required operation boundary against the product's named operations, rather than adding per-function tests or a second profiler.

TaskQ v4.5.0 now retains local SQLite/WAL telemetry under a 30-day/64-MiB bound and embeds `profile guide` (PR #1264). The earlier v4.4.4 log readbacks below are historical evidence, not the current storage behavior.

## One native scenario, existing telemetry

1. Discover installed versions and current effective integration read-only. Start with local `agentq stats --accounting`, `agentq profile` and `taskq profile export`; read `agentq telemetry status` only for export diagnostics. Read existing installation/routing/hook observations where available. Service health/configuration alone never qualifies compression. No update/restart or waiting for open sockets to disappear.
2. Use the existing deterministic 180-pass/one-intentional-failure cargo fixture, with a fresh hidden error marker. Ask the real client to execute the ordinary command once; do not ask it to invoke `agentq run`. A denied or unexecuted command is unknown. Preserve model, auth, sandbox and approvals.
3. The existing test records native session, call identity where available, `event_id`, original ref, model acknowledgment, exact recovery, raw native usage, client version and installation/runtime. Core `run` is the Claude hook path; `provider` is the Codex path. A manual wrapper cannot qualify automatic integration.
4. Check the existing local stats first. Only when connected-mode delivery is being qualified, read the specific `agentq.compress` observation back from Langfuse by trace ID = core `event_id`. Compare schema, reason/outcome, bytes including ref, adapter/runner/engine/version and session/tool IDs. Match installation/runtime from the host identity. A synthetic `agentq.verify` span proves only the export path, not compression.
5. In connected mode, query native observations for that session and join by native tool-call ID. AgentQ session identity is in `metadata.attributes.agentq.session`; it need not be Langfuse's top-level `sessionId`. Native tools use their own sessionId/tool_id. Existing exporter shapes need explicit normalization, not name/time-only joins.
6. Prove model-visible ref/error and exact original recovery using the native check; where approved native capture contains it, corroborate with the exported model/tool output. Do not enable new raw-content capture or send prompts, arguments, outputs, paths or credentials in AgentQ events. Telemetry from before a result hook is not proof of the delivered output.
7. Compare complete per-session native usage with delivered generations. Deduplicate observation/trace IDs, retain retries and failed attempts, and preserve input/cache/output/reasoning categories. If any generation/category is absent or incomparable, usage coverage is unknown or failed independently of working compression. Never substitute estimated tokens.
8. For a TaskQ-owned run only: read `taskq profile export --since 30m`, use its actual project/task/attempt/runtime and native-session evidence from the owning runtime. Check terminal build/review stage and known-token coverage. Do not infer task/attempt from timestamps, host aliases or project names. No attempt means TaskQ is not applicable, not missing. Profile collection itself must not fail the task.

## Scenario table

Reuse VERIFICATION's families; do not duplicate one suite per version.

| Family | Native/client cases | Telemetry expectation |
| --- | --- | --- |
| A2 positive | Qualified success and intentional failure; supported cargo/build/check, pytest, go test recipe cases | One logical core event per call, smaller delivered output, same IDs/versions, correct summary/error/exit, original recoverable; local/exported fields agree. |
| A2 protected | Code, full JSON, dense constraints, short output, unqualified command; Unicode/CRLF/native shell | Required pass-through and canonical reason where core is called; byte counts may be unknown for native uncaptured bypass. No false gain, permission change or re-execution. Core-only guard tests do not qualify a native client. |
| A2 continuation | Resume/retransmit, code-mode carriers, typed/nested output, HTTP/SSE/WebSocket, async command/poll where claimed | Same transformed prior result, no repeated logical compression, correct session/call association. Unsupported native shapes are explicit gaps. |
| A4 fault | Missing engine, bounded timeout, full/unwritable store, queue outage/duplicates | Native work and original preserved, failed vs passed reasons correct, truthful backlog/dropped/gaps; delivery retried idempotently. Inject faults only in isolated owned fixtures. |
| A6 identity | Separate users/profiles, native Windows vs WSL, CLI vs Desktop | Correct installation/runtime/profile/revision and native client identity; missing profile/client build cannot qualify a fleet/version claim. |

## Optional connected-mode reader check

`python3 scripts/check-compression-telemetry.py --local <client-new-records.json> --observations <compression-export.json> <native-export.json> --evidence <one-native-result.json>`

Inputs are existing checker/telemetry exports. New native checks include event ID, installation/runtime and observed usage in their result. Use complete query pages; HTTP errors or remaining cursor are unknown. No credentials or network access are needed by the checker.

The native checker records hook cancellation separately as `native_telemetry_delivery`; no stderr is not export proof. Local compression acceptance never depends on optional export.

The checker verifies one native compressed run, exact producer/export fields, installation/runtime, native tool association, duplicate handling and output-token coverage. It does not invent input/cache normalization, auto-collect Desktop evidence, run hosts, or certify the entire scenario matrix. Exit 0 requires compression, identity and output-usage coverage; input/cache coverage remains an explicit separate unknown. `python tests/telemetry_qualification.py` proves its rejection paths offline in CI.

Example read-only queries (use the existing authorized credential source; never print credentials):

```
npx langfuse-cli api observations list --trace-id <core-event-id> --fields core,basic,metadata,usage --limit 100 --json
npx langfuse-cli api observations list --session-id <native-session> --fields core,basic,metadata,usage --limit 100 --json
```

Use the configured real Agents endpoint. A loopback exporter may accept ingestion but not observation queries; HTTP 501 `unsupported_local_telemetry_endpoint` is a read-path capability gap, not zero events. Do not rewrite production routing/config to query the documented endpoint.

## Current evidence and gaps

Readback 2026-10-15: installed Mac Claude CLI 2.1.299 fixture, session `1b2d27d2-86fc-4ff6-956a-39ae48ef0dd7`, core event `000018deaf8c0730fba80000f0a30001`. AgentQ 0.6.2/RTK 0.51.0; local record and Langfuse `agentq.compress` agree: 4206 -> 420 bytes. Native Tool: Bash has the same tool call. Prior native check proved model-visible ref/error and complete original recovery. This is one historical fixture, not a current fleet pass.

Usage coverage fails: CLI reports 242 output tokens; the delivered native GENERATION reports 94, with one generation returned. The other 148 output tokens are not represented in this readback. Query succeeded, meta has no remaining cursor. Diagnose the native exporter before using these observations as whole-turn cost/usage. Do not infer compression failure from this export gap.

TaskQ 4.4.4's 30-minute local export reads 780 samples, zero corrupt: 493 operations and 287 board records, no stages. A subsequent 24-hour request returns 39,014 retained samples, zero corrupt: 28,258 operation rows, 10,022 board operations, 270 queue transitions and 464 stages. Its actual retained observation window is shorter than the requested 24 hours; earlier/pruned history is unknown. The compression fixture was not a TaskQ-owned task, so stage absence does not fail it. Collection of these local records is confirmed; model stage coverage for a TaskQ test remains unqualified.

Mac follow-up: Claude Code 2.1.299, AgentQ 0.6.1, RTK 0.51.0, session `2419ea48-5ae2-4e92-911e-ead8960e0da0`, event `000018deb24119a4bc6800014a640001`: native/local/export compression PASS, 4206 -> 420 bytes, complete recovery and model reference PASS, output usage 272 native = 94 + 178 delivered. The earlier failing run reports `SessionEnd ... Hook cancelled`; offline replay through the plugin's own test harness emits both ended generations (94 + 148 = 242). The local async Stop edit was removed, restoring the installed plugin's native synchronous Stop. The new run has no hook error and all four native observations read back. This qualifies that run only; SDK progress-before-flush remains a delivery-loss risk on cancellation/network failure. Input/cache accounting and other surfaces remain unknown.

All other OS/Desktop/version combinations remain unknown until their own native scenario and telemetry readback. Current compression records do not establish complete native client build/surface/profile attribution or whole-task input/cache coverage. Extend existing producer metadata only when a named missing field blocks qualification; follow spec-first and keep the same queue and schema compatibility.

## Cost experiment, separate from functional qualification

Reuse existing paired experiment evidence under ACCOUNTING O3. Same task/model/config, balanced baseline/candidate order, independently checked quality, native usage/cache/retries/retrieval/latency and a documented cost source. Invalid/incomplete pairs remain visible. No fixed savings threshold is invented here; VERIFICATION requires owner-agreed budgets before engine selection/release. Functional compression can pass while savings or usage coverage stays unknown.

## Official references checked 2026-10-15

- OpenAI configuration/telemetry: https://developers.openai.com/codex/config-advanced/ (redirects to https://learn.chatgpt.com/docs/config-file/config-advanced). Native API/stream/tool observations and version metadata are available when enabled; opt-in and batching matter.
- OpenAI caching: https://developers.openai.com/api/docs/guides/prompt-caching . Cache reuse depends on matching rendered prefixes; byte reduction is not billed-token reduction.
- Claude Code Desktop: https://code.claude.com/docs/en/desktop . Shared settings/hooks do not replace a real interactive Desktop run.
- Claude hooks: https://code.claude.com/docs/en/hooks . Newer documented result replacement is schema-sensitive; telemetry can represent original pre-hook output. Capability-test the actual installed version; do not change our approved PreToolUse recipe based on latest docs alone.
- Claude usage: https://code.claude.com/docs/en/monitoring-usage . Input excludes cache reads/writes; preserve those categories when normalizing. Do not add parent totals and their subcategories.

Native Codex qualification inherits the installed permission and approval settings. The checker must not force `--sandbox`, `--full-auto`, approval flags, model or profile overrides: forcing workspace-write created a sandbox runner failure on a Windows profile already configured for full access. A native launch failure stays unknown and never becomes a compression failure or PASS.
## Accepted SQLite migration

Owner selected SQLite/WAL for local telemetry in AgentQ and TaskQ. Each product owns its installation-local store; no shared mutable cross-product database, database server, second collector or parallel analytics source. Preserve the existing event IDs, schemas, coverage accounting and local-only `stats=off` semantics. Replace the existing telemetry history rather than dual-writing a new history indefinitely. Legacy readers/writers and rollback need an explicit migration transition; preserve retained data and test idempotent retry, crash/restart, corruption, locking and bounds before deployment. JSONL remains export. AgentQ retains its named per-kind windows until a separately accepted change; TaskQ uses 30 days / 64 MiB. TaskQ SQLite is released in v4.5.0 (PR #1264); AgentQ SQLite remains a candidate until its release and native deployment checks pass.

### AgentQ SQLite transition

`data/telemetry.sqlite3` belongs to the existing data directory, preserved by update and rollback. Bundled SQLite uses application ID `0x4151544d`, schema 1, FULL durability, the lifecycle writer lock and a two-second busy timeout. The database has 28 MiB of pages. Cache spilling is off; a TRUNCATE checkpoint must succeed before a transaction starts, bounding one WAL transaction below 29 MiB and SHM below 1 MiB. Temporary storage is memory-only; no VACUUM copy. The 64 MiB database/sidecar budget excludes older JSONL files, which stay unread until uninstall. New records are at most 64 KiB. Pressure pruning marks the removed coverage interval unknown; age retention keeps the existing seven-day event window.

Compression event IDs are migration keys: identical retries are idempotent and conflicting same-ID payloads fail as unknown. Stats and their coverage journal commit together; every local consumer reads the same snapshot. Existing failure markers remain outside the database so an unwritable database can still leave evidence. A marker reset starts after its own second, never proving an earlier lost call in that second. Source corruption, missing coverage and truncated history remain unknown, never empty healthy statistics. Old writers may keep appending JSONL; import on change and preserve old files. Old readers do not see SQLite events: keep this reader or export before downgrading. Local qualification consumes `stats --records`, not private database/filesystem layouts. Optional remote delivery remains a separate check.

### Installed checks and local analysis

`agentq check compression [codex|claude|both]` embeds the existing checker; Python 3.9+, cargo, the selected client and its native login are prerequisites. It uses the checked Headroom Python when available, otherwise an installed interpreter. `--codex PATH`/`--claude PATH` select exact client versions. Windows can locate the installed Codex embedded CLI without PATH edits; that remains a CLI cohort. Codex inherits native permissions/model. Claude uses an explicit command-only fixture allowlist with other tools disabled and `dontAsk`; the report records this scope. Neither test rewrites persistent client settings. Client denial remains unknown.

All existing producer events are retained locally independently of remote delivery; compression keeps its single canonical stats record. Before deleting a sent queue event the sender ensures its local history is durable. A malformed queue event is kept and reported. Resource events preserve their existing same-bucket replacement identity. `agentq telemetry export` prints `agentq.telemetry/1` JSONL: coverage/errors header and typed stats, trace, export-sample, existing-event or legacy-export rows. It contains metadata only and never claims complete collection. `doctor` checks SQLite read-only; missing optional remote keys is normal standalone operation.

Pressure removes oldest event-time rows and advances coverage. Export samples keep the latest 256 per operation. The existing service's local event writes run retention; no second process or database. The binary's offline selftest exercises the bundled SQLite store, while paid native checks remain explicit.
