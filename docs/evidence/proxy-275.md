# Windows proxy start and stall: task 275

[Task 275](https://github.com/taskqorg/agentq/issues/275) records why the Headroom proxy on Windows did not answer while the service said running. It adds agentq.md M5.5 and M5.6. All observations are from one Windows 11 host on 2026-11-06, release 0.10.0 and a debug candidate of this branch.

## Result

- Measured cause: the proxy inherits low priority from the Task Scheduler start of the service. Under that priority its start did not finish.
- Measured defect: the service replaced only an exited worker and `agentq status` did not show the proxy, so a silent worker stayed unseen.
- Not removed: the start stays slow on this host at normal priority, once 562 s. Host load is the remaining factor; its share is not separated.

## A1: why the start blocks

The unit XML from CoreQ has no `Priority` element, so `Get-ScheduledTask` reads `Priority 7`. The service and the proxy read:

```text
agentq.exe  service run  class=0x4000 ioPriority=1 pagePriority=2
python3.12  proxy        class=0x4000 ioPriority=1 pagePriority=2
powershell  (interactive) class=0x20  ioPriority=2 pagePriority=5
```

These are below-normal CPU, low I/O and low memory priority.

The start reads many files. `headroom/cli/proxy.py` imports `transformers` as a required module, and `create_import_structure_from_path` in `transformers/utils/import_utils.py` opens and reads nearly all of its 2719 Python files (it skips `__init__.py`, `convert_` and `modular_` files). A stack dump of a normal-priority run shows this frame at 20 s.

Same interpreter and files, two minutes apart, with one difference:

| Sample, 21:51 | Service worker (low) | Diagnostic probe (normal) |
|---|---|---|
| CPU | 1 s after 70 s | 6 s after 7 s |
| Read operations | about 1.5 per second | about 275 per second |

A controlled run imported only `transformers`, with a child killed at the 120 s budget. The kill itself overran, because the runner was also at low priority:

| Round | Low (inherited as from Task Scheduler) | Normal |
|---|---|---|
| 1 | not finished after 174 s | 20.57 s |
| 2 | not finished after 352 s | 19.26 s |

The service log agrees. The last `Hard watchdog armed` line is 17:42:43. The worker started after the 20:49 watchdog exit logged nothing until the service restart, so it never reached `armed` and the headroom watchdog could not cover it. At 21:47 a worker started at 21:35 held 92 MB and no listener.

The 442 s `/v1/models` request is inferred, not reproduced. The log has no line from 20:44:01 to 20:48:43, and the watchdog dump shows the main thread in plain `starlette/datastructures.py`, with no lock wait. The same window logged upstream `httpx.ConnectTimeout`. The service's own resource samples have a hole from 20:11 to 21:10 and gaps up to 838 s against a 300 s pace, so the service was starved too or absent. No memory sample covers that stall.

Host load is measured later only. At 22:41 the host read 1071 MB free memory, 1775 MB free of a 51110 MB commit limit, 9392 pages/s and a disk 95% idle, with 20 `claude`, 19 `node` and 15 `ChatGPT` processes. Process Lasso runs on this host; its effect was not measured.

## A2: visible state

`agentq status`, `status --json`, `doctor`, `status.json` and the heartbeat now carry `proxy`. Only `HTTP/1.x 200` from `GET /livez` proves `answering`, so a stalled interpreter that the kernel still accepts for reads `failed`.

Lines from the candidate, started from a low-priority parent in an isolated install:

```text
codex: proxy starting (proxy_starting, no HTTP reply on 127.0.0.1:50977 for 80 s)
codex: proxy failed (proxy_start_timeout, no HTTP reply on 127.0.0.1:50976 for 451 s); run `agentq service restart`, or `agentq direct codex` for direct access
codex: proxy answering (127.0.0.1:50977)
codex: proxy failed (proxy_not_responding, no HTTP reply on 127.0.0.1:50977 for 29 s); run `agentq service restart`, or `agentq direct codex` for direct access
```

The acceptance allows a fallback or a named cause; this delivers the named cause. A client already waiting on the proxy keeps waiting until it is moved to direct access ([reason](../spec/SERVICE.md#proxy-supervision)).

## A3: before and after

```text
cargo test --locked --bin agentq routing::
```

With the pre-fix behavior restored in `routing.rs` (no priority raise, no replacement of a silent worker, a TCP accept counts as an answer), 3 tests fail:

```text
only_a_200_proves_the_proxy_answers ... FAILED
silent_worker_is_replaced_and_stays_failed ... FAILED   left: 1, right: 0
worker_inherits_normal_priority_after_task_scheduler_start ... FAILED   left: (16384, 1, 2), right: (32, 2, 5)
```

After the change, all 9 pass.

The first candidate replaced a worker by the silence clock alone. The live run showed it: after 320 s each new worker was killed one pass later, so no start could finish. Replacement now also needs the worker to be 900 s old. With the first form restored, the test fails with `a replacement gets its own time`, `left: 0`, `right: 1`.

## Live Codex turn

The candidate ran in a temporary install with its own data and port, from a parent lowered to the Task Scheduler state. It raised itself and its worker:

```text
service        class=0x20 io=2 page=5
python.exe     class=0x20 io=2 page=5
python3.12.exe class=0x20 io=2 page=5
```

The proxy read `failed` from 300 s and first returned 200 at 562 s, with the worker kept. The earlier 300 s replacement rule would have discarded this start. Free memory at start was 3970 MB and free commit 6446 MB. Another run in the same hour reached `Hard watchdog armed` at 128 s. One `codex exec` (codex-cli 0.162.1, native login, per-command `-c` provider override) then returned:

```text
codex exit=0 seconds=52 ... READY
event=proxy_inbound_response method=POST path=/v1/responses status=200 duration_ms=4933.91
```

`~/.codex/config.toml` and the live install were not changed.

## Limits

- Routing on the working Windows profile stays as found: Codex was already disabled there before this run. This candidate is not released or deployed. Re-enabling is a separate step after release, with a live turn on that profile.
- The Task Scheduler start itself was simulated by a low-priority parent. The installed unit was not run with the candidate.
- CoreQ writes the unit without `Priority`, so other products that use the same unit type may inherit the same state. This task did not inspect them.
- The DashQ snapshot keeps its enumerated `routing`, so a stalled proxy can read `routed` there. The heartbeat `agentq.routing_problems` names it.
- macOS and Linux have the replacement and state changes through their shared code and tests. Their OS priority was not examined.
