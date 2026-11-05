# Compression core detail

Binding detail of [agentq.md § 3](../../agentq.md#3-managed-engines). The contract holds the mandatory rules; this file holds engine pins, qualified lists, adapters, the core entry, configuration, stats record and coverage. A change here is a decision change ([spec first](../../agentq.md#principles)). Native client checks: [COMPRESSION-BENCHMARK.md](COMPRESSION-BENCHMARK.md#native-client-check-rules).

Section references and P/M rule IDs below refer to [agentq.md](../../agentq.md).

## Engines and pins

- An engine downloads into `<install>/engines/<name>-<ver>/` on install and on an update that changes its pin, and is checked by sha256 and `--version`. Engines install in `service install` before the unit loads and on each `service run` start, so a release with a new pin installs it; an engine already checked is reused.
- **RTK:** prebuilt binary from `rtk-ai/rtk` releases, pin 0.51.0. `agentq run` passes only the qualified list (`pytest`, `cargo test|build|check`, `go test`) to `rtk`; others run natively. Native RTK execution keeps command semantics. Read-only sandbox without writable storage or no checked RTK: run natively, no compression.
- Raw output of each RTK run is retained in `data/runs/<id>/` for `get_original` with ref `rtk:<id>` and the hash of RTK's recovery hint (500 runs, 7 days, 512 MiB).
- **headroom:** loopback model proxy for routing and usage observation; its request compression stays off; its supported extension sends qualified new tool results to the core. It never injects tools, never rewrites transmitted prefixes and never switches provider.
- headroom pin `headroom-ai[proxy]` 0.40.0 installs with `pip --require-hashes --only-binary :all:` from the built-in lock `agentq/headroom.lock` into a private venv `<install>/engines/headroom-<ver>/`, checked by `headroom --version`.
- headroom uses Python 3.11–3.13 found on the host (`AGENTQ_PYTHON` overrides). The pin's wheels need macOS 14+ on arm64 and do not exist for macOS x64. No Python or no wheels: agents run unrouted, `doctor` says why.

## Compression core

- Accepted results target: [ACCOUNTING O1–O4](ACCOUNTING.md) defines outcome populations, paired byte measurements, coverage and whole-task evidence for PRODUCT R9/R10. This adds no optimizer, capture permission or universal savings claim.
- Core input: `agent`, `adapter` (`run`, `hook`, `provider`), `tool`, `command` (shell tools only; matched to a runner, never stored), `text`, native `exit_code` for captured results, native IDs when known.
- Config off for the agent, under `min_bytes`, code or complete JSON: pass through. Anything off the adapter's list passes with reason `not_qualified`. Every call writes one stats record.
- A compressed result keeps the original under a `get_original` ref (`<engine>:<id>`, store and limits of `data/runs/`) and ends with one line naming that ref. Unqualified, disabled or unavailable-engine runs stay native without capturing output; their byte measurements are unknown.
- The run adapter captures eligible native stdout/stderr once before RTK filtering, keeps stderr and the native exit code, and returns native bytes whenever verification fails; its 3 s filter budget excludes native execution. Retained originals are stdout bytes; stderr is never compressed.
- Qualified lists are built in per adapter and change only by a release; no config adds a tool. The runner set is the RTK list. Every other tool of every adapter (file reads, search, web, MCP results, other commands) passes unchanged.
- `run` (Claude Code through its hook, any agent calling `agentq run`): a listed command executes natively once; RTK executes a private replay of that capture through its qualified recipe. `provider` (Codex `exec_command`, `shell`) and `hook` (Hermes `terminal`) reuse that bounded capture replay with the native exit code.
- Replay verifies the native arguments and exit code; for pinned RTK pytest it also expects only its injected display flags (`--tb=short`, `-q`, `-rxX`, each omitted when the native args already set that category). A mismatch returns the original. Recipes stay limited to the runner set, with whole-task checks and the protected-result guards.
- Headroom keeps transport passthrough; its supported proxy extension calls this core, never its generic history optimizer. Entries, `compression` config, the stats record and accounting coverage: [COMPRESSION.md](COMPRESSION.md). `agentq check compression` pass rules: [COMPRESSION-BENCHMARK.md](COMPRESSION-BENCHMARK.md#native-client-check-rules).

## Adapters

- **Antigravity workers:** `agentq hook antigravity PreToolUse` wraps qualified `run_command` commands with the existing `agentq run` adapter, preserving other arguments and native permission decisions. Onboarding manages only the global hooks (§ 4); Antigravity has no proxy or MCP registration.
- An adapter maps the native result to the core input, writes the returned text back in the native shape and keeps the original on any failure (P1). It never holds thresholds, engine choice, retention, a retrieval tool, stats, telemetry, config or network calls of its own.
- **Claude Code:** outside native auto mode, the § 4 `PreToolUse` rewrite to `agentq run` is the adapter. In auto mode, the original command stays for native permission classification and only successful Bash stdout is compressed through `PostToolUse.updatedToolOutput`; stderr and the response envelope stay unchanged.
- Claude failed, interrupted and image results remain native because the failure hook cannot replace output. No hook grants permission or alters the client's mode.
- **Codex:** no compression hook. The loopback provider sends each new `function_call_output` or `custom_tool_call_output` (string, typed or JSON-wrapped text) of a listed tool through the core once per call ID and replays the returned bytes on later requests, so the transmitted prefix stays byte-identical. Other items pass unchanged.
- Codex sessions key on native `thread-id` (then `session-id`) for HTTP/SSE and WebSocket Responses. Code mode qualifies only one literal `const|let name = await tools.exec_command({...}); text(name);` carrier; arbitrary JavaScript and multiple calls pass unchanged.
- Codex replay is durable in `data/provider/<session hash>.json`: 512 results / 16 MiB per session, 64 MiB total, no eviction of transmitted bytes. Corrupt, unwritable or full replay storage preserves the original.
- **Hermes:** the shipped plugin `agentq` registers `transform_tool_result` and `post_api_request` (§ 6). `transform_tool_result` runs `agentq compress hermes` for `terminal`, the only listed Hermes tool, and unwraps its `output` field; other tools return before any call.
- The Hermes plugin has no proxy URL, own retrieval tool, lifecycle call or query memory: retrieval is MCP `get_original`, as for every agent.

## Entries

- In process for `agentq run`. `agentq compress <agent>` reads one input as JSON on stdin and prints `{"text": .., "ref": ..}` for any agent with a result hook.
- Input requires `adapter` (`hook` or `provider`), `tool`, `text`; optional `command` (string), `exit_code` (32-bit integer; required to transform a captured result), `session`, `turn`, `tool_call`.
- Run identity comes from `AGENTQ_AGENT`, `AGENTQ_SESSION`, `AGENTQ_TURN`, `AGENTQ_TOOL_CALL` when supplied, else agent `unknown` and null IDs.
- The service calls the core for the loopback provider.

## Configuration

One section `compression` in `data/config.json`: `{"enabled": true, "min_bytes": 2048, "agents": {"<agent>": false}, "stats": "queue"}`; absent keys take these defaults. `stats` is the sink: `queue` sends each record through agentq.md § 6, `off` keeps it local only.

## Stats record

- One JSON object, same keys everywhere: `schema` (1), `event_id`, `time`, `agent`, `adapter`, `tool`, `runner` (or null), `engine` (`rtk`, `headroom`, `none`), `outcome` (`compressed`, `passed`, `failed`), `reason`, `bytes_in`, `bytes_out`, `est_tokens_in`, `est_tokens_out`, `duration_ms`, `ref`, `session`, `turn`, `tool_call`, `version` (agentq), `engine_version` (pin of `engine`, null for `none`), plus `exit_code` for `run`.
- `reason` is one of `compressed`, `disabled`, `short`, `code`, `json`, `not_qualified`, `not_smaller`, `no_engine`, `timeout`, `error`.
- `est_tokens_in` and `est_tokens_out` are labeled `estimate` (agentq.md § 6). Byte/token fields are null for unmeasured native passthrough, never zero.
- Readers treat a record without `version` and `engine_version` as version unknown.
- Local copy: SQLite `data/telemetry.sqlite3`, kind `stats`, event ID unique, 7 days. `agentq stats [--json]` prints per agent and engine the call count, outcomes and bytes in and out.
- Sink `queue`: one `agentq.compress` span with each key as `agentq.<key>`; it replaces the `agentq.run` span.

## Accounting coverage

- `agentq/accounting.rs` is one pure aggregate over these records or their exported spans, shared by `agentq stats --accounting` and the consumer fixtures `docs/spec/accounting-fixtures.json` (ACCOUNTING "Published schema").
- Coverage is only what the producer proves: record and `meta.journal` (`since`, `resets`, recorded `gaps`, per-day `counts`) commit atomically.
- Missing/unreadable coverage over held records resets from the next second, rebuilding counts; its own second stays unproven.
- Failed transactions roll back both and leave `data/coverage.failed.*` outside SQLite until a later committed reset clears the markers it saw.
- Retention subtracts removed rows; pressure also advances `since`. Local totals always remain lower bounds: calls losing both record and marker are undetectable.
- Missing/corrupt stores, failed writes and fewer rows than journal counts read unknown, never zero. Recorded coverage gaps remain visible; exported events carry no completeness proof.
