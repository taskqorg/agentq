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
- A3: the background trait test observes `unlogged`, then a logged refusal, then a recovered run that reads the job's `bridgeSessionId` URL without a launch. The reader uses the client home layout shared by macOS and Windows.

## Open

- Native platform proof: no live macOS or Windows run with a failed or disabled registration was observed. Windows reads the same home layout in code only.
- Failure causes need Claude debug logging. Enabling it for workers is a separate launch decision.
