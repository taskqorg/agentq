# Universal worker history: task 282

[Task #282](https://github.com/taskqorg/agentq/issues/282) gives every product one ordered list of typed steps through `agentq-runtime`. Applied rules: agentq.md M12.18 and M12.19, P2 and P3, and specq Testing. Observations below are revision-specific; the contract is [RUNTIME.md § Steps](../spec/RUNTIME.md#steps).

## A7: `history native session differs`

**Result: a binding artifact of the caller, not a reader defect. The original probe is not available, so its own sample is not reproduced.**

The task reports 10 of 40 real Codex rollouts failing. Neither the task nor this repository holds that probe, so its session choice is inferred from these readings. All rollouts are `codex-cli 0.162.1` on Windows x86_64, read with `History` at the task baseline and again at the result:

| Binding | Rollouts | Failed |
|---|---|---|
| The rollout's own thread ID, from its file name or `session_meta.id` | 442, of which 71 subagent | 0 |
| `session_meta.session_id` of a subagent rollout | 71 | 71, each `history native session differs` |
| The thread ID of a real fork | 1 | 0 |

- In a subagent rollout `session_id` names the root thread and `id` names the rollout's own. Every other record carries the own ID in `thread_id`. Binding the root ID therefore always refuses, and that refusal is correct: the file is not that session.
- Subagent rollouts are 71 of 436 at the survey, and a window of 40 by time holds up to 16 of them. A sample of 40 holding 10 is consistent with this cause.
- No rollout on this host holds records of another thread. A real fork made with `thread/fork` on a copied rollout in a temporary home holds only its own records and names its base in `history_base` (`thread_id`, `end_ordinal_exclusive`, `end_byte_offset`).

The fix removes the caller's choice: `Runtime::history` and `agentq agent history` locate the source by session ID, so a caller never binds a path to another ID. A child page names `parent`, `root` and `relation`; a fork page names its base, and `--until-byte` reads that base. `a7_forked_and_subagent_rollouts_read_under_their_own_id` guards both shapes and the refusal.

Open: a rollout from a Codex below the supported `minimum` 0.162.0 that copies parent records would still refuse. None exists on this host, and the runtime does not support that version.

## Failing before

Runtime `History` at the baseline read every Codex `item_completed` CommandExecution, FileChange, McpToolCall, Reasoning and ContextCompaction item, and Claude thinking blocks, as `unknown` or `unsupported-content-block`. `Step` did not exist.

Each mapping test fails with its mapping removed. With `dispatch` reduced to an `unknown` step for every record and the `History` mapping reverted, `cargo test -p agentq-runtime --test steps` gave 26 failed, 2 passed. The two that pass guard no mapping: `a7_until_byte_reads_a_fork_base` guards the byte bound, and `a8_fixtures_hold_no_private_values` guards the scrub.

The tests caught these defects while the task ran:

- A stale reason `running` survived on a finished step, because the merge kept reasons from the open revision.
- A partial revision replaced the real `started_ms`, and left reasons for fields of the generic kind on the final step.
- Blocks of one Claude record sorted by ID, because they shared one `order`.
- `Exit code <n>` was read from anywhere in failed output, so command output could set the code.
- A background job's duration ended at its start result rather than its notification.

## Real read

Reading the transcripts of this host through the reducer, as whole files:

| Source | Sessions | Steps with a console | Unknown steps |
|---|---|---|---|
| `codex-cli 0.162.1` rollouts | 442 | 14919 shell | 0 |
| Claude Code `2.1.281`-`2.1.318` sessions | 300 | 27535 shell, with subagents below | 4 (`attachment:plan_mode` 3, `attachment:command_permissions` 1) |
| Claude subagent files | 150 | counted above | counted above |

- Assistant messages repeated within a turn: 0, so the double native records converge.
- Real missing values read their reason, most often `merged-output` (Codex stdout and stderr), `encrypted` (Codex thinking), `not-reported` (Claude exit code of a successful call) and `not-retained` (Claude empty thinking).
- A page reads in about one second on the debug build. `history` locates the source on each call and verifies the prefix under the wire contract, so a very long session takes many calls. No limit is set on this: measure it when a consumer reads whole long sessions.

Session state on the same data: Codex 121 `turn-terminal`, 320 `open-turn-stale`, 1 `open-turn-recent-write`; Claude 3 `live-process`, 293 `no-live-entry`, 4 `process-unreadable`, and 150 subagent sessions read `subagent-session`. An `open-turn-stale` reading on 320 finished worker rollouts is why an old open turn reads `unknown` and keeps its resume command: those rollouts have no terminal record, so refusing them would block every one.

## Live output

`item/commandExecution/outputDelta` is proven by a real capture: `thread/shellCommand` on a `codex-cli 0.162.1` app-server, which runs a command without a model call. The fixture holds the deltas and the item that replaces them, and the same item ID appears in the rollout.

No Claude transcript read holds a tool output record before `tool_result`, so Claude reads `unsupported`. Antigravity was not run, so it reads `unsupported` too. Neither claim is a proof that no such stream exists in another mode.

## Fixtures

`runtime/tests/fixtures/steps/` holds real transcripts, scrubbed by `scripts/scrub_history_fixture.py`. `provenance.json` names the version, source digest and row counts of each. The scrub keeps structure, numbers and protocol values, maps IDs and replaces every other string; every surviving string and every non-identifier key was listed and reviewed. The current worker's own session is excluded from the sources.

| Version | Fixtures |
|---|---|
| `codex-cli 0.162.1` | `rollout`, `compaction`, `subagent`, `parent`, `fork`, `app-server` |
| Claude Code `2.1.318` | main, `compaction`, `agent`, `subagent` |
| Claude Code `2.1.295` | main |

Existing `fixtures/history/` files cover Codex `exec`, `agy` and the sidecar. Schema-only mappings, with no real capture: Codex approval and question requests, Claude `control_request` approval, and Claude `AskUserQuestion`.

## Not qualified

- macOS and Linux transcripts were not read on a real host; CI runs the fixtures there.
- Antigravity history uses earlier retained fixtures only, under the limits in the contract. No current `agy` run was made.
- A Codex client idle between turns reads `idle`, because Codex publishes no process registry. A second writer on such a session is not detected.
- No consumer has adopted this API, and no working profile changed.
