# Runtime changelog

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
