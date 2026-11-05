# Native Antigravity events for #246 A5

Task: [AgentQ #247](https://github.com/taskqorg/agentq/issues/247), evidence for [#246](https://github.com/taskqorg/agentq/issues/246) A5.

**Verdict: partial.** Real typed `Message`, `ToolStart`, `ToolEnd` and `Activity` events and real quota windows were observed. The native tool never executed: the host's pre-tool hook denied it, and the probe still passed. A successful Antigravity tool completion stays unknown, so this evidence does not qualify that path (agentq.md P2).

## Candidate

| Item | Value |
|---|---|
| Candidate | `9b0e29de31c688d1c90132c9ff5be0090719fcfe`, from `taskq-246`; `git rev-parse HEAD` read back the same SHA |
| Checkout | Detached worktree inside this task's worktree; removed after the runs |
| Native client | `agy --version`: `1.5.1` |
| Platform | macOS, existing host authority; no TCC or sandbox denial observed |

The candidate was not changed. Nothing was installed. The probe added two native conversations to the existing Antigravity store; those conversations remain there. No profile, credential, grant or hook file was edited.

## Commands and results

### Events

```text
cargo run --locked -p agentq-runtime --example qualify_events -- antigravity
```

The command ran twice, because the first native log was deleted before its tool error was read. Both runs exited 0 with the same event counts.

| Run | Session | Outcome | Message | ToolStart | ToolEnd | Activity | Text, Quota, Context |
|---|---|---|---|---|---|---|---|
| 1 | `c993344d-a853-4343-a8bb-b1cfc5686f07` | `ok` | 1 | 1 | 1 | 5 | 0 |
| 2 | `b5e9552e-ecf9-4d14-ac23-ac1eb4dccd51` | `ok` | 1 | 1 | 1 | 5 | 0 |

The native stream of run 2 had these steps. Run 1 had the same step types and states.

| Native record | Typed event |
|---|---|
| `init` | `Activity` |
| `user_input` `DONE` | `Activity` |
| `agent_response` `DONE` | `Activity` |
| `tool` `run_command` `ACTIVE` | `ToolStart` |
| `tool` `run_command` `ERROR`, 0.04 s | `ToolEnd` with status `ERROR` |
| `agent_response` `ACTIVE`, then `DONE` | `Activity`, twice |
| `result` `SUCCESS` | `Message` |

The `ToolEnd` error, read from the run 2 log:

```text
TOOL_ERROR: tool call denied by pre-tool hook:
```

The final text was the requested `PLAIN_OK`, even though the tool had failed. The model did not report the denial.

### Limits

```text
cargo run --locked -p agentq-runtime --example qualify_events -- antigravity limits
```

Exit 0. Account windows, observed once:

| Window | Used | `resets_at` | Unknown |
|---|---|---|---|
| `gemini-weekly` | 0.7353 | 1794135234 | none |
| `gemini-5h` | 0.0009 | 1793930246 | none |
| `3p-weekly` | 0.0 | 1794519741 | none |
| `3p-5h` | 0.0 | 1793932941 | none |

The turn report carried no window of its own: `limits` read `unknown: "not-reported"` in both event runs. `event_id` is null in every window.

## Missing capabilities

- `Text`: `delta_capability` is false. The native stream does carry `text_delta`, but the Antigravity descriptor does not declare `turn-deltas` and this probe does not request deltas.
- `Quota`: no event; quota comes only from the account limits read above.
- `Context`: no event, although native `agent_response` and `result` records carry `usage`.
- `ToolKind`: the Antigravity mapping sets `ToolKind::Other`, including for `run_command`.

## Errors and open items

- **Denied native tool.** Unknown: which hook denied the call. Earlier evidence (`docs/spec/VERIFICATION.md`, task #198) ties the same error text on `agy 1.4.12` to an AgentQ hook that returned `{}`. The candidate's `agentq/hooks.rs` returns `{"decision": "allow"}` on those paths, and #246 does not change that file. This task read no hook configuration, because `~/.gemini` is outside its allowed folders. The installed hook version is also not established. The probe does not exercise the candidate's hook.
- **Not observed.** `ToolEnd` with status `DONE`, and any tool output.
- **Not covered.** Sidecar transport, resume, cancel and any platform other than macOS.

## Correction needed in #246

1. Make `qualify_events` fail when no Antigravity tool ends with status `DONE`. Today it prints only counts, so a denied tool passes as `outcome: ok`.
2. Print each `ToolEnd` status in the probe output, so the JSON line alone shows the tool result.
3. Rerun A2 after the host owner finds the denying hook. Do not bypass the hook, and do not isolate the native home to avoid it: that would drop the real host path this evidence must prove.

Until then, #246 A5 reads as: Antigravity typed events observed, tool success unknown. Under #246 A5 ("A failure edits the contract"), the #246 owner decides whether this result changes its contract text.

## Privacy

This file holds metadata only: SHA, version, session IDs, step types, counts, quota numbers and native error text. It holds no prompts, tool arguments, credentials or host paths.
