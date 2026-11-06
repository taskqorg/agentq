# Telemetry event detail

Binding detail of [agentq.md § 6](../../agentq.md#6-telemetry-to-langfuse). The contract holds the mandatory rules, event names, identity rules and budgets; this file holds field formats, producer mechanics and probes. A change here is a decision change ([spec first](../../agentq.md#principles)).

## Heartbeat attributes

`agentq.installation` carries `status.json` fields ([agentq.md § 5](../../agentq.md#5-install-layout-and-service)) plus:

- `agentq.legacy_route`: fresh safe legacy endpoint diagnosis (`absent`, `problem` with next `agentq direct`, or `unknown`). `agentq.retirement`: the service startup migration result, independent of release health.
- `agentq.hooks` is a JSON array with one object per agent and event that has a record ([agentq.md § 1](../../agentq.md#1-commands) hook evidence): `agent`, `event`, `state` of the latest run (`ok`, `failed`, `unknown` with `reason` `unreadable success stamp` or `unreadable failure record` when its success stamp, failure record or retained last failure is unreadable), `last_ok_at` (unix seconds or null) and `last_failure` (`{at, error}`, kept after a later success; null when it never failed).
- `agentq.hook_failures` lists `<agent> <event> (<error>)` for each hook whose latest run failed, else `none`.
- `agentq.model_backlog`: see [Claude transcript reading](#claude-transcript-reading).

## Model calls

### Launched sessions and project

- Launched sessions (agentq.md M6.9): the Claude `Stop` hook and the Hermes `post_api_request` hook read the launch marker from their own environment, which the client passes on. A launched session reads no transcript, writes no offset and queues no span. A hook run that skips a launched session is a successful run.
- The retained Codex model record entry does not see the client environment. The Codex `SessionStart` hook therefore writes `data/sessions/<first 32 hex of SHA-256 of the session ID>.json` with `launched` and the project code (no path). `agentq model codex` reads the record of the response's native session. A launched record queues nothing. Records are kept 7 days after their last write; a resumed session writes its record again.
- A Codex call without a session record reads `agentq.project_unknown` `no session start` and is queued; a call that arrives before its `SessionStart` record can be counted twice.
- Project (M6.10): the working directory is the transcript line's `cwd` for Claude, the hook input's `cwd` for Codex `SessionStart`, and the hook process's working directory for Hermes. agentq walks from it up to the first `taskq.json`. Its `repo` gives `agentq.project` when it is `owner/name` of `[A-Za-z0-9._-]`, neither part starting with `.`, at most 64 characters.
- `agentq.project_unknown` reasons: `no working directory`, `no known project` (no `taskq.json` up to the root, or no valid `repo` in the nearest one), `no session start` (Codex).

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
- The same hook writes the `agent.call` record ([Session records](#session-records)); its allowlist also carries `reasoning_tokens` for that record.
- An existing installation gains the hook through `onboard --yes`, which upgrades an owned plugin file whose bytes still equal its journal record (journal first) and keeps any other content a conflict.

### Codex responses

- Historical `codex_provider` events retain their mapping below. The loopback producer is retired; `agentq model codex` remains a compatible record entry. Current native Codex calls are recorded by Stop from the rollout (Session records).
- Call ID = the response `id`; `agentq.agent` `codex`, `agentq.model.source` `codex_provider`. A response without `id` keys on a hash of its entry. Start = end = `created_at` (entry time when absent).
- Tokens keep the Claude meaning: `input_tokens` is uncached input (`usage.input_tokens` − `input_tokens_details.cached_tokens`), `cache_read_tokens` is `cached_tokens`, `output_tokens` is `usage.output_tokens` (reasoning included).
- The Responses API reports no cache creation and no cost: `cache_creation_tokens_unknown` and `cost_unknown` are `not reported`; any other missing field reads `not in response`.

## Session records

M6.11 applies the runtime records of [RUNTIME.md](RUNTIME.md#observation) to sessions no launcher records. `agentq-runtime` `agent::session_records` and `agent::session_call_records` build them; the hook only finds the bytes and writes the records.

| Runtime | Entry | Source of `agent.call` | One `agent.turn` per |
|---|---|---|---|
| Claude | `Stop` | transcript with its `<session>/subagents/*.jsonl`, each message id once | `Stop` that found new bytes |
| Codex | `Stop` | rollout at `transcript_path` | `Stop` that found new bytes, or a `Stop` without a transcript |
| Antigravity | `Stop` | `transcriptPath`, each completed step index once | `Stop` with `fullyIdle` not false that found new bytes, or without a transcript |
| Hermes | `post_api_request` | the hook's allowlisted record, written at once | `on_session_end`, which fires when one `run_conversation` ends |

- A launched session and a disabled integration read and write nothing.
- Store: `data/observations.sqlite3`, the installation's CoreQ local store. Sending it is the CoreQ shipper's job, in a later task after parity; this page adds no sender.
- Transcript reads: a per-file offset in `data/observe/` advances only after the append. One `Stop` reads at most 8 MiB in total, always through the whole line it started, and stops reading 3.5 s after the hook started. Unread bytes wait for the next `Stop`, so the hook stays inside its 5 s budget (M1.2). The turn key is the first read file and its start offset, so a retry after a failed append keeps one turn. A shorter file was rewritten and reads from the start; immutable call ids keep a reread from counting twice.
- Records go to the store in chunks of 1024 with the turn first, and the hook starts no chunk after that deadline. An append that stopped early keeps the offset, and the retry keeps the first payload of each stored id.
- A record has `route` `hook`, and `agent.turn` sums the tokens of its calls. `exit_code`, CPU and `peak_rss_bytes` read null `hook-session`, because the client process is not ours; `duration_us` reads null `not-reported`.
- A `Stop` without a session writes `session_id` null `session-unknown` with a turn key unique to that `Stop`. A `Stop` without a transcript writes null tokens with reason `no-transcript`. A Claude `Stop` without a transcript writes nothing, because the `agentq.model` path already records that failure.
- Outcome: Claude and Codex read `ok`. Antigravity reads `failed` for an `error` value or a `terminationReason` naming an error or the step limit, else `ok`. Hermes reads `ok` for `completed`, `cancelled` for `interrupted`, `failed` when both are false and `unknown` when neither is reported.
- Antigravity reports no tokens in its hooks. A transcript step without token fields is one `agent.call` with null tokens `not-reported`, and `modelName` fills a model the transcript does not name. The Antigravity `Stop` answers `{"decision":"allow"}`, on failure too, so the hook never re-enters the loop.
- Hermes keeps each call's allowlisted record in `data/observe/hermes/<session hash>.jsonl` until `on_session_end` sums it into the turn and removes it. A file left by a session that never ended expires after 7 days. A call stays recorded when no turn follows. A model name that is not a code, such as `vendor/model`, reads null `not-reported`, because the store accepts only codes.
- `project` is the M6.10 code of `cwd`, else of the first `workspacePaths` entry, else of the working directory, else `other`.
- A failed read or append is a recorded hook failure (§ 1) and still returns the normal hook response (M1.2). It never stops the `agentq.model` path.

### Open gaps

- Claude Desktop Chat has no hook and writes no records (PRODUCT R1).
- No native Hermes or Antigravity client ran these hooks yet. Their hook fields come from the [Hermes observer hooks](https://hermes-agent.nousresearch.com/docs/developer-guide/observer-hooks) and the [Antigravity hook schema](https://antigravity.google/docs/hooks).
- An existing Antigravity `agentapi` server does not inherit the launch marker (§ 12). If that server runs the global `Stop` hook, a launched sidecar turn writes a second turn under another key.

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
- A sample is a few file reads or syscalls and one span of about 1 KiB: 288 spans per host per day.

## Profiling

`agentq profile` ranks retained export samples; it starts no compressor or proxy. Empty or unreadable samples remain unknown. `profile trace on [<minutes>]|off` retains its existing switch (default 15, at most 60 minutes), now recording export steps only. Historical compression rows remain retained, with no new producer; profile does not rank retired operations. Local telemetry SQLite retention and export rules remain unchanged (agentq.md §8).

## Local store

The installation owns `data/telemetry.sqlite3`: bundled SQLite, application ID `0x4151544d`, schema 1, FULL durability, lifecycle writer lock and the existing two-second busy timeout. Pages stay at 28 MiB; cache spilling is off and a TRUNCATE checkpoint precedes each transaction, preserving the existing WAL below 29 MiB, SHM below 1 MiB and total database/sidecar budget of 64 MiB. Temporary storage is memory-only; no VACUUM copy. Older JSONL stays unread until uninstall. Records retain the existing 64 KiB bound and seven-day window.

Events persist locally before optional delivery. A malformed queued event stays and reports an error. Resource IDs preserve same-bucket replacement. Same-ID retries are idempotent; conflicting payloads, corruption, missing coverage and pruned history remain unknown. Historical compression rows remain retained with no new producer. Export samples keep the existing latest 256 per operation. Existing service writes run retention; no second process or database is added.

`agentq telemetry export` emits metadata-only `agentq.telemetry/1` JSONL with coverage/errors and retained event, sample, trace or historical stats rows. It never claims complete collection. `doctor` reads SQLite integrity; missing optional remote credentials alone do not fail local health.
