# Runtime changelog

## 0.11.0

- Add `Run::snapshot(&Context) -> Res<Frozen>` for deferred persistence, and `Frozen::{events, append, save, restore}`. A snapshot needs no store, lock or native call, and repeated calls return the same value with the first caller context.
- A terminal poll freezes the CLI report, its duration and resource use once. `Runtime::report`, `Turn::report` and `Run::record` read that value, so later shared-log bytes, queue delay, a late cancel or a passing deadline never change a finished turn. A repeated `poll` no longer rereads the log.
- A CLI `agent.turn` now has an immutable ID instead of a random one, so a retry or a recovered run counts once. Stored records and `agent.call` IDs stay valid; new CLI turn records carry the new ID form.
- `Run::record` retries after a failed append instead of refusing; a second call after a success stays refused. A background or queue record no longer reads the transcript again.

No existing signature changes. `Run` checkpoints gain an optional `frozen` field; a checkpoint without it restores as before. Grants, policy, native arguments and the CoreQ pin stay unchanged.

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
