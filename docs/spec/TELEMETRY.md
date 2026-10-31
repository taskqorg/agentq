# Telemetry event detail

Binding detail of [agentq.md § 6](../../agentq.md#6-telemetry-to-langfuse). The contract holds the mandatory rules, event names, identity rules and budgets; this file holds field formats, producer mechanics and probes. A change here is a decision change ([spec first](../../agentq.md#principles)).

## Heartbeat attributes

`agentq.installation` carries `status.json` fields ([agentq.md § 5](../../agentq.md#5-install-layout-and-service)) plus:

- `agentq.routing` is the `status.json` `agents` object as a JSON string: per agent with a config root its `routing` (routed, direct, bypassed, unknown), `reason`, `intended` and `intended_reason`; `{}` when none.
- `agentq.routing_problems` lists `<agent> <routing> (<reason>)` for each agent intended routed but not routed and `<agent> intended unknown (<reason>)`, else `none`.
- `agentq.hooks` is a JSON array with one object per agent and event that has a record ([agentq.md § 1](../../agentq.md#1-commands) hook evidence): `agent`, `event`, `state` of the latest run (`ok`, `failed`, `unknown` with `reason` `unreadable success stamp` or `unreadable failure record` when its success stamp, failure record or retained last failure is unreadable), `last_ok_at` (unix seconds or null) and `last_failure` (`{at, error}`, kept after a later success; null when it never failed).
- `agentq.hook_failures` lists `<agent> <event> (<error>)` for each hook whose latest run failed, else `none`.
- `agentq.model_backlog`: see [Claude transcript reading](#claude-transcript-reading).

## Model calls

### Claude transcript reading

- The Claude `Stop` hook reads the session transcript at `transcript_path` and its `<session>/subagents/*.jsonl` from a per-transcript byte offset in `data/model/` (7 days), up to the last complete line. A rewritten or shorter file reads from the start.
- It shares the hook's failure record. An existing installation gains the `Stop` registration through `onboard --yes`.
- Bounded passes: a pass reads in batches of at most 512 lines or 4 MiB; each batch writes its spans in chunks of 32, one local SQLite transaction each, and one queue prune (one queue file per event), then saves its offset.
- A hook pass starts no batch and no chunk after 3 s, inside the 5 s hook budget; the offset then stops at the first line of the first call not written, so a pass never runs more than one chunk past its deadline. A pass's first chunk always runs, so every pass progresses.
- The offset file also records the bytes left. The service sender drains transcripts with bytes left on each pass (10 s per pass), so a session that ends mid-backlog still completes.
- `status.json` `model_backlog` and the heartbeat attribute `agentq.model_backlog` report `{state, transcripts, bytes}` (`pending` or `none`); an unreadable offset file reads `unknown` with a reason, never `none`.
- One call per native call ID: the Anthropic `requestId`, else the message `id`. Claude repeats a call once per content block; the last line of a call wins. Client-made `<synthetic>` messages are not calls.
- Span start = end = the line's source `timestamp` (hook time when unparseable). A call without a native ID keys on its transcript line.
- Attributes besides the common ones: `agentq.agent` (`claude`), `agentq.session`, `agentq.model.source` (`claude_transcript`), `agentq.model.name`, `agentq.model.call_id`, `agentq.model.input_tokens`, `agentq.model.output_tokens`, `agentq.model.cache_read_tokens`, `agentq.model.cache_creation_tokens` (the transcript's `usage` integers).
- Cost: only `costUSD` on the line gives `agentq.model.cost` (double), `agentq.model.cost_currency` (`USD`) and `agentq.model.cost_source` (`claude_transcript`). Current Claude Code reports none: `agentq.model.cost_unknown` is `not reported`. agentq computes no price; estimates belong to dashq with their own source.
- A missing field omits its attribute and sets `agentq.model.<field>_unknown` to `not in transcript`.

### Hermes provider calls

- The plugin's `post_api_request` hook runs `agentq hook hermes post_api_request` once per provider call with only `api_request_id`, `session_id`, `turn_id`, `model`, `response_model`, `ended_at` and Hermes's normalized `usage` buckets; request, response and message content never leave the plugin.
- One `agentq.model` span per call, same event ID rule with call ID `api_request_id` (a call without it keys on session, turn and end time), start = end = `ended_at`.
- Attributes as for Claude with `agentq.agent` `hermes`, `agentq.model.source` `hermes_hook`, `agentq.turn`, name `response_model` else `model`, `cache_creation_tokens` from `cache_write_tokens`; a missing value reads `<field>_unknown` `not reported`. Hermes reports no cost in this hook: `agentq.model.cost_unknown` is `not reported`.
- An existing installation gains the hook through `onboard --yes`, which upgrades an owned plugin file whose bytes still equal its journal record (journal first) and keeps any other content a conflict.

### Codex responses

- The loopback provider ([agentq.md § 3](../../agentq.md#3-managed-engines) Codex) reads each `response.completed`, `response.incomplete` or `response.failed` event (SSE, WebSocket or a plain JSON response) it already parses. It passes only the response `id`, `model`, `usage`, `created_at` and the native session to `agentq model codex` in a background thread; the stream stays byte-identical and a failure only logs.
- Call ID = the response `id`; `agentq.agent` `codex`, `agentq.model.source` `codex_provider`. A response without `id` keys on a hash of its entry. Start = end = `created_at` (entry time when absent).
- Tokens keep the Claude meaning: `input_tokens` is uncached input (`usage.input_tokens` − `input_tokens_details.cached_tokens`), `cache_read_tokens` is `cached_tokens`, `output_tokens` is `usage.output_tokens` (reasoning included).
- The Responses API reports no cache creation and no cost: `cache_creation_tokens_unknown` and `cost_unknown` are `not reported`; any other missing field reads `not in response`.

## Resource samples

### Scope

- Native Windows and WSL are separate installations with their own `agentq.runtime` and describe different scopes: `windows` is the whole host, `wsl` is the WSL 2 VM, whose CPU, memory and virtual disk the Windows host already counts inside its own numbers.
- WSL uses the Linux probes and excludes Windows drives (`drvfs`, `9p`), so no filesystem is counted twice. WSL 1 (kernel release containing `Microsoft`) has no VM: its `/proc` mirrors the Windows host, so CPU and memory are unknown with `wsl1 mirrors the Windows host`, and its `lxfs`/`wslfs` root is not a local filesystem.
- Standalone samples use Langfuse. Enrolled samples use the same queue and DashQ durable ingestion/history; no additional sampler, local queue or client database.
- The queue keeps one file per resource event ID, so a replaced sample never adds a file.
- Span `agentq.resource`, stage `sample`, start = end = wall time of the reads (source timestamp).

### CPU and memory

- `agentq.resource.cpu_percent` (double, 0–100, step 0.1): busy share of all CPUs over the interval since the previous read in this service run; `agentq.resource.cpu_interval_ms` (int) is its monotonic length.
- Linux `/proc/stat`: busy = user + nice + system + irq + softirq + steal of total = busy + idle + iowait (guest is inside user). macOS `HOST_CPU_LOAD_INFO`: busy = user + system + nice of total = busy + idle. Windows `GetSystemTimes`: busy = kernel − idle + user of total = kernel + user (kernel time includes idle).
- CPU: the service reads a baseline at start, no state survives a restart; an interval under 10 s gives `warmup`; counters that went backwards (32-bit macOS tick wrap) give `counter reset`.
- `agentq.resource.memory_total_bytes`, `memory_available_bytes`, `memory_used_bytes` (int bytes, used = total − available). Linux `/proc/meminfo` `MemTotal`, `MemAvailable`. macOS `hw.memsize`; used = (internal − purgeable + wired + compressor) pages × page size, Activity Monitor's Memory Used. Windows `GlobalMemoryStatusEx` `ullTotalPhys`, `ullAvailPhys` (Task Manager's In use = total − available).

### Filesystems

- `agentq.resource.disks` (string: JSON array, one object per filesystem): `id`, `role` (`root` for `/` or the Windows system drive, else `other`), `fstype`, `used_bytes`, `free_bytes`, `total_bytes`.
- Total = blocks × block size, free = blocks available to unprivileged users, used = total − all free blocks; reserved blocks make used + free < total. Linux matches `df`; macOS APFS counts the whole container (Finder, `diskutil apfs list`), where `df` Used shows only the volume. A filesystem whose stat fails has null numbers and `unknown` with the reason.
- Filesystems are re-read on every sample; a mount change adds or drops an entry in the next one, never a zero.
- Linux reads `/proc/self/mountinfo` and keeps local block filesystems (`ext2`, `ext3`, `ext4`, `xfs`, `btrfs`, `f2fs`, `zfs`, `bcachefs`, `jfs`, `vfat`, `exfat`, `ntfs3`), one per device number, the shortest mount point first (bind mounts, btrfs subvolumes); pseudo (`proc`, `sysfs`, `tmpfs`, `overlay`, `squashfs`, ...), network and FUSE filesystems are excluded.
- macOS keeps `getfsstat` mounts flagged `MNT_LOCAL` without `MNT_DONTBROWSE`: `/` (its APFS container) and browsable external volumes.
- Windows keeps drive letters of type `DRIVE_FIXED` (`GetLogicalDriveStringsW`, `GetDriveTypeW`), fstype from `GetVolumeInformationW` (`unknown` when it fails, e.g. a locked BitLocker volume), numbers from `GetDiskFreeSpaceExW`: free = available to the caller, used = total − total free; removable, network, optical and RAM drives and folder mounts are excluded.
- WSL 2's `/` is its virtual disk: total is the disk's maximum size, while the Windows drive that holds it bounds real free space.
- A filesystem `id` is the first 16 hex of HMAC-SHA-256 over mount point and fstype, keyed with `data/resource-key` (32 random bytes, never exported): stable while the mount stays, not reversible to a path.

### Clocks, freshness and overhead

- Bucket and timestamp use the wall clock, intervals the monotonic clock. When wall and monotonic deltas differ by over 2 s (clock step, suspend), `agentq.resource.clock_step_ms` holds wall − monotonic.
- dashq shows the newest sample as stale after 900 s (three buckets) and draws a graph only from three or more samples, else says too few samples.
- A query spans at most 7 days (the queue cap); older history lasts as long as the Langfuse project retains traces. An outage delays samples within the queue caps; they arrive with their source timestamps.
- A sample is a few file reads or syscalls and one span of about 1 KiB: 288 spans per host per day; a test bounds one sample at 50 ms.

## Profiling

| Operation | Sample | p95 budget | Failure budget |
|---|---|---|---|
| `compress.run` | agentq.md § 3 stats record, adapter `run` | none: `duration_ms` includes the native command; the 3 s filter budget is enforced inline | 1 % |
| `compress.hook`, `compress.provider` | stats record of that adapter | 1000 ms | 1 % |
| `export.langfuse` | one per send pass that posted or failed | 10 s | 5 % |
| `export.dashq` | one per delivery of grant-bound records (ENROLLMENT) | 10 s | 5 % |
| `proxy.request` | none: the headroom proxy serves model requests, agentq records none | unknown | unknown |

- Counters: always on. A stats record is the compression sample: outcome `failed` is a failure and its `reason` the error class.
- An export sample is `time`, operation, `duration_ms`, `class` in SQLite kind `export`, last 256 per operation; legacy state snapshots import without deduplicating equal observations. Class is null on success, else `auth`, `http`, `timeout`, `network`, `queue` (Langfuse) or `no_reply`, `stopped`, `rejected` (DashQ).
- Traces: `agentq profile trace on [<minutes>]` sets `profile.trace_until` in `config.json`, default 15, at most 60 minutes; `off` removes it. While on, `compress.run` writes its `native` and `filter` steps and each export its posted batches to SQLite kind `trace`: `time`, `op`, `step`, `duration_ms`, `class`.
- Traces are kept 7 days; a day stops at 1 MiB of canonical payload; legacy JSONL stays readable through import. `agentq profile` reports the switch state.
- Report: `agentq profile [--json]` over the 7 days up to the report time, later timestamps excluded: per operation its calls, failure rate and classes, p50/p95/max ms (nearest rank), mean, standard deviation and its coefficient of variation `cv`.
- Over budget: p95 above its budget (`time`), failure rate above its budget (`failures`), `cv` above 1 (`unstable`). Rank: the largest of p95/budget, failure rate/budget and `cv`, worst first, computed by CoreQ (`profile::summarize`, `profile::rank`); unknown operations last. Trace lines add per-step calls and p95.
- Unknown: no samples in the window, an unreadable stats record (all `compress.*` operations), an unreadable or non-object export state or `profile` value (all `export.*` operations) or a malformed sample list or sample (that operation only) reads unknown with the reason, never zero or ok; the other operations still report.
- An unreadable `config.json` or invalid `profile.trace_until` makes the switch unknown, never off, and writes no trace; an unreadable trace directory, file or line makes the steps unknown.
