# Runtime changelog

## 0.14.0

- Add `agent::steps`: `Turn`, `Step` (14 kinds), `Output`, `Source`, `Session`, `Timeline` and `Step::merge`. Any run reads as one ordered list of typed steps, live or finished. See [Steps](../docs/spec/RUNTIME.md#steps).
- Add `Runtime::history`, `read_source`, `log_steps`, `steps`, `session_state` and `resume_command`, and the command `agentq agent history`. Consumers implementing `Runtime` must implement them. `progress` and `steps` share one native read.
- Add `Descriptor.resume`, the native terminal command, and the Codex capability `live-output`. Consumers constructing a `Descriptor` must add the field.
- `History` events now map Codex `CommandExecution`, `FileChange`, `McpToolCall`, `Reasoning`, `ContextCompaction` and `AgentMessage` items and Claude `thinking` blocks instead of reading them as unknown. `kind` gains `thinking`, and blocks gain `thinking` and `lifecycle`. Consumers matching these values exhaustively must add them. `capabilities` gains the feature `live_output`.
- `History` binds a Claude `agent-<id>` session by the record's `agentId`, because a subagent file carries its parent's `sessionId`.

Existing event variants, history checkpoints, launch arguments and policy are unchanged.

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
