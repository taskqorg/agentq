# Typed progress and background fidelity: task 261

[Task 261](https://github.com/taskqorg/agentq/issues/261) lets a caller drop native event parsing without losing execution-health and background-continuation behavior. It applies agentq.md M12.9 through [log records](../spec/RUNTIME.md#log-records).

## Defect and correction

On runtime-v0.9.0 and v0.10.0, `Report.background_job` counted an unconfirmed tool-use record as a job, could not match task IDs from result text, and missed notifications in `queue-operation` records. A boolean could not name the jobs. `Runtime::log_events` also dropped native time, so valid and invalid timestamps produced identical events, and it hid turn starts and payload-free progress.

The correction adds `Report.live_background`, `Runtime::log_records`, `RecordTime`, `TimeFault`, `Event::TurnStart`, `Event::Progress` and `Event::Unsupported`. Native parsing stays in the adapters.

## Before and after

The development command was:

```text
cargo test --locked -p agentq-runtime --lib agent::events
```

Before the fix, a probe on the unchanged source exited 101 with:

```text
[("started", true, true), ("completed notification", true, false), ("KillShell", true, false), ("unconfirmed start", true, false)]
indistinguishable timestamps: true
```

After the fix, all 13 event tests pass. The new checks are:

- `report_keeps_the_eight_live_background_outcomes` uses the fixtures of TaskQ's `live_background_reads_jobs_the_last_turn_left_running` and its expected labels.
- `background_jobs_need_a_confirmed_start_and_match_by_native_task` covers each start form, stop tool, notification source and terminal status, the forged-text cases and the label rules.
- `log_records_keep_source_order_and_native_time` covers absent, null, malformed, valid and future time, and source order.
- `typed_records_keep_progress_guards_without_native_parsing` runs a copy of TaskQ's progress reader that reads typed records only, against its stall, deadline and timestamp-fault outcomes.

## Decisions a reader should check

- **A stop request now removes a job.** TaskQ's case reads a KillShell call with no result as not live, and A2 names it. The 0.9.0 assertion that a stop request keeps the job changed for this reason. A failed stop result still restores the job.
- **A start without a named task is not a job.** A3 requires a successful background-start result. TaskQ today accepts a Bash result with no task ID; the runtime reads it as unconfirmed.
- **`taskq.rules` is not in the runtime (M12.2).** The caller finds its own marker by `LogRecord.line` and passes only later records to `classify`. Case 7 passes this way, and a native `system` `init` gives the same result without a marker.
- **`Progress` is only in `log_records`.** The #246 guard keeps `Activity` for non-tool items in a live stream, so live consumers see no change for those records.

## Native evidence

The field names come from 61 local Claude Code transcripts, read for shape only, with payloads not printed:

- Start results carry `backgroundTaskId` or the text `Command running in background with ID:`.
- 6 foreground Bash results carried `backgroundTaskId`, so a foreground call can become a job.
- Notifications appeared in `queue-operation`, `queued_command` attachments and user text. They also appeared in `prompt_snapshot` and `api-request` records, which the reader does not trust.
- A `TaskStop` result carries the raw `command`, which stays in the adapter.

`system` `task_notification` comes from the Claude Agent SDK message types. It appeared in no transcript read here, so that path has only a fixture check. Monitor start text and `KillShell` input come from TaskQ's fixtures: the only Monitor result in these transcripts was an error, and only `TaskStop` calls appeared.

## Evidence limits

These are fixture and adapter checks with no native work launched (R12). They do not qualify a native client or a TaskQ restart. TaskQ adoption belongs to [TaskQ #2448](https://github.com/taskqorg/taskq/issues/2448). No service, policy, login or working profile changed.
