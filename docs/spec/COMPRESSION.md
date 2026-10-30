# Compression core detail

Binding detail of [agentq.md § 3](../../agentq.md#3-managed-engines). The contract holds the mandatory rules, qualified lists and adapters; this file holds the core entry, configuration, stats record and coverage. A change here is a decision change ([spec first](../../agentq.md#principles)). Native client checks: [COMPRESSION-BENCHMARK.md](COMPRESSION-BENCHMARK.md#native-client-check-rules).

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
- Local copy: SQLite `data/telemetry.sqlite3`, kind `stats`, event ID unique, 7 days; legacy JSONL imports without deletion. `agentq stats [--json]` prints per agent and engine the call count, outcomes and bytes in and out.
- Sink `queue`: one `agentq.compress` span with each key as `agentq.<key>`; it replaces the `agentq.run` span.

## Accounting coverage

- `agentq/accounting.rs` is one pure aggregate over these records or their exported spans, shared by `agentq stats --accounting` and the consumer fixtures `docs/spec/accounting-fixtures.json` (ACCOUNTING "Published schema").
- Coverage is only what the producer proves: record and `meta.journal` (`since`, `resets`, legacy `gaps`, per-day `counts`) commit atomically.
- Missing/unreadable coverage over held records resets from the next second, rebuilding counts; its own second stays unproven.
- Failed transactions roll back both and leave `data/coverage.failed.*` outside SQLite until a later committed reset clears the markers it saw.
- Retention subtracts removed rows; pressure also advances `since`. Local totals always remain lower bounds: calls losing both record and marker are undetectable.
- Missing/corrupt stores, failed writes and fewer rows than journal counts read unknown, never zero. Legacy coverage gaps remain visible; exported events carry no completeness proof.
