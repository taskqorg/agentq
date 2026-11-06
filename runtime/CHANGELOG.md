# Runtime changelog

Entries describe behavior at their released revisions. Current AgentQ identity and support: [the worker-layer contract](../agentq.md); historical optimization intent remains a compatible no-op under [the retirement decision](../docs/spec/OPTIMIZATION-RETIRED.md).

## 0.15.0

- Require a caller-owned `Launch.session_store` on every worker launch, resume and recovery. Untracked sessions require explicit adoption; concurrent writers refuse with `session-busy`. Native submission seams require the same `SessionOwner`. There is no no-store mode.
- Replace public spawnable native command builders with argument inspection; submissions require an owned runtime boundary.
- Replace caller-reported process state in history resume helpers with explicit adoption and an owned `ResumeCommand`; spawning journals native process identity before terminal input.
- Add caller-selected `Launch.inactivity` and `stop_on_stall`, `Observation.activity`, owner observations and `Event::Stalled`. Detection only reports by default; declared native tool deadlines and unknown activity remain distinct. Checkpoints retain the separate activity cursor and ownership token.
- Public struct literals must supply the new fields; older run checkpoints without ownership evidence refuse with `session-owner-unknown`. Protected-host authority and observation-only recovery never resubmit input.

## 0.14.0

[Regression and local transcript evidence](../docs/evidence/runtime-2448.md).

- Add `Window.rejected: Option<bool>` for independent native rejection status; utilization never invents it. Claude unified-window siblings identify the top-level binding rejection. Old serialized windows read absent rejection as unknown. Exhaustive window constructors must add `rejected`.
- `Report.detail` retains Claude terminal result subtypes and Codex failed-turn messages, preserving typed outcome/error and `Report.reached`.
- Add `Runtime::account_limits_with_executable(home, executable, now)`; Codex uses the supplied executable with the same read-only credential guards as the existing method. The old method retains native lookup; other runtimes ignore this override. Consumer-defined `Runtime` implementations must implement this required method; no compatibility default discards the executable.
- Add `History::with_native_session_alias` for caller-validated copied Claude jobs. The alias is explicit and checkpointed; records with any other session ID still fail, as do source replacement and prefix changes. Aliased events retain caller session and expose the original native session. No alias is inferred from filenames.
- Native start-limit, terminal, call/usage and history parsers now live behind per-runtime protocol callbacks; shared report assembly interprets no native record types. No native flags, limits, admission policy or execution behavior change.

- Add ordered `Runtime::preflight`, `Readiness`, `ReadinessMode` and explicit journaled `Repair` for worker launches. Launch and resume refuse known blockers before input; unknown checks continue by default and remain visible. Account probes use the selected command, home and environment.
- Add account-bound `ready_for`, `catalogue_for` and `account_limits_for` probes; existing probe methods stay available.
- Add exact Claude workspace trust and native Codex project/hook trust preparation, with ownership checks, client locks, native revisions and guarded uninstall. Hooks remain optional evidence.
- Reports and checkpoints retain local readiness; turn telemetry keeps only fixed preflight counters. Observations expose `phase` and `starting_ms` without changing poll states.
- `Launch`, `Report`, `Observation` and installation `Edit` gain fields; consumers constructing them literally must add these fields. `Launch::new` keeps its signature. Protected-host authority and no-replay recovery remain unchanged.

## 0.13.3

- Pin CoreQ v0.23.4.

## 0.13.2

- `Report.exit` and the `exit_code` observation read unknown when the runtime stopped a client that had already ended its turn. They used to report the kill's own code (Windows 1, Unix -1) as the client's exit. The outcome stays as it was.

## 0.13.1

- `History` events add `validity: {status, reason}`. Claude records with a non-empty string `type` are `valid`, including unsupported metadata such as `file-history-snapshot`. Missing or malformed discriminators are `invalid`. Other transports report `unknown`. A consumer can skip valid metadata and refuse invalid envelopes without parsing native JSON. See [task #288 evidence](../docs/evidence/runtime-288.md).

## 0.13.0

- `LogRecord` and `History` events expose assistant failures, lifecycle display labels and saved Claude permission modes without consumer native parsing. `AssistantFailure` retains the native code, `auth-rejected`/`client-error` classification and local detail. Exhaustive LogRecord constructors/patterns must add the new fields or use `..`.
- `classify` recognizes Claude assistant API failures, including `authentication_failed`, and retains the cause through a final failed result. Later successful results and new turn boundaries do not inherit earlier failures. Live completion, launch policy, source identities and checkpoints are unchanged.
- Native-free migration checks retain exact-input/duplicate/restart observation and verify lifecycle/quota separation and permission-baseline readback. See [task #277 evidence](../docs/evidence/runtime-277.md).

## 0.12.0

- Add `Run::snapshot(&Context) -> Res<Frozen>` for deferred persistence, and `Frozen::{events, append, save, restore}`. A snapshot needs no store, lock or native call, and repeated calls return the same value with the first caller context.
- A terminal poll freezes the CLI report, its duration and resource use once. `Runtime::report`, `Turn::report` and `Run::record` read that value, so later shared-log bytes, queue delay, a late cancel or a passing deadline never change a finished turn. A repeated `poll` no longer rereads the log.
- A CLI `agent.turn` now has an immutable ID instead of a random one, so a retry or a recovered run counts once. Stored records and `agent.call` IDs stay valid; new CLI turn records carry the new ID form.
- `Run::record` retries after a failed append instead of refusing; a second call after a success stays refused. A background or queue record no longer reads the transcript again.

No existing signature changes. `Run` checkpoints gain an optional `frozen` field; a checkpoint without it restores as before. Grants, policy, native arguments and the CoreQ pin stay unchanged.

## 0.11.0

- Add `Report.live_background` (also in the JSON turn report): one fixed `Bash` or `Monitor` label per live Claude job, oldest first. Descriptions, commands, arguments and task IDs stay inside the adapter. Equal labels remain separate entries. `Report.background_job` is true exactly when this list is not empty.
- A tool-use record no longer counts as a live job. A job needs a non-error result naming its native task, in structured fields or in the text `with ID: <id>` or `(task <id>`. A foreground Bash call that the client moved to the background counts through its structured `backgroundTaskId`.
- Terminal notifications now also close jobs from `queue-operation` content, `queued_command` attachments and `system` `task_notification` records, by task ID or tool-use ID. The statuses `error`, `timeout` and `expired` are terminal.
- A TaskStop, KillShell or KillBash call now takes its job out of the live list before its result arrives; a failed result restores it. This intentionally changes the 0.9.0 behavior, where only a successful result closed the job.
- A `system` `init` record clears earlier jobs. A caller that marks its own turns in the log passes only the records after its last marker.
- Add `Runtime::log_records(log)`: `LogRecord { line, time, events }` per source record, with `RecordTime` separating absent, null, malformed and valid native time, and `RecordTime::observed(now)` adding future time against the caller's clock.
- Add `Event::TurnStart`, `Event::Progress` and `Event::Unsupported { reason }`, and the `TurnEnd` outcome `unknown`. Consumers with exhaustive event matches must handle them. `Progress` appears only in `log_records`; a live stream keeps `Activity` for those records. `Activity` itself proves no progress.
- Claude assistant text emits one `Message` per text block in native order with tool blocks. Repeated blocks remain deduplicated, and the aggregate final result remains suppressed. Log-only repeated or user text progress keeps block order too. Null, malformed or empty text and unknown Codex item or Antigravity step types prove no progress.
- Codex rollout tool calls, their outputs, `message`, `reasoning`, `task_complete` and `turn_aborted`, Claude `turn_duration`, and the Antigravity tool state `FAILED` now produce typed events. `ToolStart.timeout_ms` also reads `timeout_seconds`, and treats zero as absent.

`Report.outcome`, quotas, account identity, exec launch and resume, and protected app-server behavior are unchanged. Consumers implementing `Runtime` must implement `log_records`.

## 0.10.0

- Pin CoreQ v0.23.0 (`34ae02cbe35065b08a5e0643d45881ec494276b6`). A consumer that uses CoreQ directly must spell the same dependency, `coreq = { git = "https://github.com/taskqorg/coreq", tag = "v0.23.0" }`, so that it resolves one CoreQ source and its `Store` is the one `Run::record` takes.
- `agent.turn` and `agent.call` inherit `project`, `task`, `attempt`, `actor` and `entry` from the current CoreQ caller context. An explicit `Context::project` or `Context::task` wins; a `Context::project` of `other` yields to a caller project. A missing value reads null with a reason.
- `agentq agent turn` treats a valid `COREQ_TRACE_CONTEXT` as its caller for its records and child.
- Protected host launches carry validated caller context through launchd and the clean native environment; invalid context is omitted without changing grants or native ownership.

No public signature changes. Without a caller context, records keep their existing payload. Tokens, privacy, immutable IDs, recovery, native ownership and the CoreQ shipper and cursor stay unchanged.

## 0.9.0

- Add `Runtime::log_events(log)` to read typed events from complete attempt-log records without a `Run`, for Claude, Codex and Antigravity.
- Add `ToolStart.timeout_ms` and `Event::TurnEnd { outcome }`. Consumers with exhaustive event matches must handle `TurnEnd`; named `ToolStart` matches must add `timeout_ms` or use `..`. Terminal events do not prove process or descendant completion.
- Typed terminal events retain pre-work refusal context across incremental reads and cursor recovery; limits after work starts remain failures. Partial Claude `quotaLimits` retain readable fallback names and resets from `apiErrorParams.rate_limit_info`.
- Add `Report.background_job` (also in the JSON turn report) for Claude Bash background tools and Monitor, successful TaskStop/KillShell results and terminal task notifications.
- Add `Window.exhausted: Option<bool>`. Rejected Claude quota windows retain their names and resets even without utilization; `Report.reached` preserves the reset before work starts. Missing utilization remains unknown.
- Omit null Claude legacy windows and null scope dimensions from account limits; malformed non-null values remain unknown. This intentionally changes the former null-window partial-snapshot behavior.
- Read Codex account limits through app-server even without `auth.json`; existing unreadable or changed credentials still invalidate a read.
- Add read-only `Runtime::account_identity(home)`: Codex email or account ID, Antigravity Code Assist project, otherwise `None`. Tokens remain inside the adapter; no refresh or worker turn is performed.

Existing launches, policy enforcement, optimization intent, observation IDs and recovery authority remain unchanged. Consumers implementing `Runtime` must implement the two new read methods. Existing serialized window checkpoints accept the absent `exhausted` field as unknown.
