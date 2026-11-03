# Session observation with missing-link causes: task 213

Source: [task 213](https://github.com/taskqorg/agentq/issues/213), the owner-approved extension in [taskq#2261](https://github.com/taskqorg/taskq/issues/2261#issuecomment-6599180594) (answer 1, "Extend AgentQ observation").

## Applied rules and scope

AgentQ R13 / § 12 owns runtime observation. P3 is applied by adding the § 12 `observe` line and the RUNTIME.md "Session observation" table before code.
P2 is applied: a requested Remote Control link with no URL and no Claude debug log reads `unlogged`, never `pending`, and unreadable records read `evidence-unreadable`.
No launch argument, client configuration, consumer repository or working installation changes. The runtime never turns on Claude debug logging.

## Native evidence used (macOS, Claude Code 2.1.316)

- Background job state `~/.claude/jobs/<job>/state.json` carries `bridgeSessionId` `cse_<id>`; on this host the job of this worker session held `cse_017XK7FV…`, and its Remote Control link is `https://claude.ai/code/session_017XK7FV…`.
- The transcript carries `system` `bridge_status` records with the URL.
- The Claude binary logs registration to the debug log (`debug/<session>.txt`) as `[bridge:repl] Hook initialized, session=…`, `Init failed: <detail>; consecutive failures: N`, `notifyBridgeFailed detail="…"` and `Org policy now refuses the connected bridge`. No such log existed on this host, because debug logging is off by default.

## Acceptance evidence

- A1: `Runtime::observe` returns `Observation { state, error, session, url, cause, detail }` for every adapter. The fake-client trait test checks Claude (`not-requested`) and Codex/Antigravity (`no-session-url`) after a finished turn.
- A2: `claude_link_names_url_or_logged_cause` covers `not-requested`, `unlogged`, `pending`, `disabled` with the native policy text, `failed` with its detail, the latest logged event winning, unreadable job state (invalid JSON), unreadable debug log (directory) and missing home.
- A2 review fix: a finished Claude transcript replaced by a directory read `session-unknown` with no detail at 4a61c04 (`runtime_trait_runs_every_runtime_by_descriptor` failed: right `Some(Cause("evidence-unreadable"))`). The link now reads the transcript first and returns `evidence-unreadable` with `transcript: <error>`; the same test passes.
- A3 native macOS proof: `AGENTQ_LIVE_CLAUDE_JOB=5f771468 cargo test -p agentq-runtime --lib live_claude_job_link -- --ignored --nocapture` read the live background job of this worker session (Claude Code 2.1.316) and printed `https://claude.ai/code/session_017XK7FVci2QhbLDYeAJe46J`, the session's own Remote Control link. A recovered run reads the same job state.
- A3 fake checks: the background trait test observes `unlogged`, then a logged refusal, then a recovered run that reads the job's `bridgeSessionId` URL without a launch. The reader uses the client home layout shared by macOS and Windows.

## Native Windows proof (task 216)

Source: [task 216](https://github.com/taskqorg/agentq/issues/216), for A3 of this task. Host: native Windows 11, Claude Code 2.1.317 (`Runtime::ready` version `2.1.317 (Claude Code)`). These are live client runs, separate from the fake-client checks above.

Procedure: `AGENTQ_LIVE_CLAUDE_BACKGROUND=1 cargo test --locked -p agentq-runtime --test agent` runs `live_claude_background_link` (not in the CI list). It launches one real Claude background job through `Runtime::launch` (`Mode::Background`, `title` set, so `--remote-control`, model `haiku`), polls `Runtime::observe`, takes `Runtime::checkpoint`, drops the run, rebuilds it with `Runtime::recover`, observes again, and stops the job with `Runtime::cancel`. The consumer parses no log.

The workspace is the repository-local `benchmarks/.tmp/live-bg-<pid>`. Claude refuses `--bg` in an OS temp folder ("Workspace not trusted"), and accepting trust would edit `~/.claude.json`, so no trust was granted. No working profile or client configuration changed.

### Before: two Windows layout defects

The first live run (job `42b99951`) registered Remote Control natively (`state.json` `bridgeSessionId` `cse_017XK7FX1MKxz6aACdqFNZcN`), but `observe` read `session-unknown` for 180 s, and the recovered run read the same:

- Job ID: Claude Code 2.1.317 prints `backgrounded · <job> · <name>` followed by command hints. The launcher parser took the last token, so the checkpoint held `"job":"session"` and discovery never found the job. The fake client printed only `backgrounded · <job>`, which hid this.
- Transcript folder: `canonicalize` returns the verbatim root `\\?\C:\…` on Windows, so the expected folder was `----C--Users-…` while Claude writes `C--Users-…`.

### Fix

- `native::background_job_id` reads the job from the first launcher line. The fake client now prints the 2.1.317 shape; with the old parser `background_commands_preserve_native_mode` fails (`left: "session"`, `right: "c37eb75a"`).
- `project_dir` removes the Windows verbatim prefix (`\\?\`, `\\?\UNC\`) before naming the folder. The unit test `claude_project_dir_uses_the_plain_path` guards it. RUNTIME.md "Background" row states both rules.

### After: live run (job `61c8f489`)

- A1: the first `observe` read `Running`, session `61c8f489-ce26-40c9-9537-9e272067e4af`, cause `unlogged` (registration not yet written). The next `observe` returned `url: https://claude.ai/code/session_017XK7FVbkUsFjJJPqAdZ25m`, cause none. The job state held `bridgeSessionId` `cse_017XK7FVbkUsFjJJPqAdZ25m`, and the transcript held one `bridge_status` record.
- A2: the checkpoint held `"job":"61c8f489"`, `"attached":true` and the transcript path under `C--Users-…`. The run rebuilt by `recover` returned the same URL through `observe` with no launch. `cancel` stopped the job; `claude agents` then listed it as `stopped`.

## Native macOS proof (task 220)

Source: [task 220](https://github.com/taskqorg/agentq/issues/220), for A3 of this task. Host: native macOS (arm64), Claude Code 2.1.316 (`Runtime::ready` version `2.1.316 (Claude Code)`), candidate main at `0cb1acef1c480b89255e5a604278b1a27db43da0`. This is a live client run, separate from the fake-client checks above.

Procedure: the same `AGENTQ_LIVE_CLAUDE_BACKGROUND=1 cargo test --locked -p agentq-runtime --test agent` as task 216, with no code change. The workspace was `benchmarks/.tmp/live-bg-54706`; no trust was granted and no working profile or client configuration changed.

### Live run (job `912d254d`)

- A1: the first `observe` read `Running`, session `912d254d-edd3-4ce1-b81b-cc15e09f8bb7`, cause `unlogged`. The next `observe` returned `url: https://claude.ai/code/session_017XK7FUHRdK6oU6EkUX2Cj5`, cause none. The job state held `bridgeSessionId` `cse_017XK7FUHRdK6oU6EkUX2Cj5`.
- A2: the checkpoint held `"job":"912d254d"`, `"attached":true` and the transcript path under `-Users-kirs-Projects-agentq--worktrees-taskq-220-benchmarks--tmp-live-bg-54706`. The run rebuilt by `recover` returned the same URL through `observe` with no launch. `cancel` returned `Ok(false)` (a background stop does not observe the exit); the job state then read `done` and `claude agents --json` no longer listed it.
- The test printed `live result: ok https://claude.ai/code/session_017XK7FUHRdK6oU6EkUX2Cj5`.
- No macOS defect was found: the launcher-line job parser and the project folder name from task 216 hold on macOS.

## Native macOS proof from `.claude/worktrees` (task 222)

Source: [task 222](https://github.com/taskqorg/agentq/issues/222), A2. Host: native macOS (arm64), Claude Code 2.1.316, candidate `a1c9ade8b2d6dcdd1b378b16f55c9956fdfb33c5` checked out as a detached Git worktree at `.claude/worktrees/taskq-222-live`. This is a live client run, separate from the fake-client check `background_job_found_when_cwd_listing_misses_it`.

Defect before the fix: `claude agents --json --all --cwd <root>` lists `[]` for a root under `.claude/worktrees`, so discovery found no job. The fix retries the unfiltered `claude agents --json --all` listing; `BackgroundJob::read` still binds the exact root, job ID and session.

Procedure: `AGENTQ_LIVE_CLAUDE_BACKGROUND=1 cargo test --locked -p agentq-runtime --test agent` from that checkout. The workspace was `benchmarks/.tmp/live-bg-2289`. No trust was granted and no working profile or client configuration changed.

### Live run (job `1b6b8db9`)

- The first `observe` read `Running`, session `1b6b8db9-fe63-4f11-a47c-29219a65aceb`, cause `unlogged`. The next `observe` read `Finished` with `url: https://claude.ai/code/session_017XK7FSzMw23rTJxM81hGJr`.
- The checkpoint held `"job":"1b6b8db9"`, `"attached":true` and the transcript under `-Users-kirs-Projects-agentq--claude-worktrees-taskq-222-live-benchmarks--tmp-live-bg-2289`. The recovered run returned the same URL through `observe` with no launch. `cancel` returned `Ok(false)`.
- The test printed `live result: ok https://claude.ai/code/session_017XK7FSzMw23rTJxM81hGJr`.
- After the run, `claude agents --json --all --cwd <root>` still printed `[]` for this root, while the unfiltered listing held job `1b6b8db9` with that exact `cwd`. So the run attached through the fallback.

## Open

- Native platform proof: macOS and Windows each have one live background run with a registered Remote Control link. No live failed or disabled registration was observed on either platform.
- Failure causes need Claude debug logging. Enabling it for workers is a separate launch decision.
