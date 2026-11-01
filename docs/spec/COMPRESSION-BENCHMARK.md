# Compression qualification

R3/R4: a smaller result must preserve the task answer and recover the exact original.

## Decisions reused

- [Public methodology before the Rust rewrite](https://github.com/taskqorg/agentq/blob/b41638%5E/docs/spec/PUBLIC-BENCHMARKS.md): paired quality, native protocol fixtures and stress are separate evidence. A vendor score cannot qualify a client integration.
- [Tokenade evaluation](https://github.com/taskqorg/agentq/tree/90d5e28%5E/benchmarks/tokenade): noisy-output savings did not prove complete JSON handling or Codex integration. Do not compress complete JSON or add a second history optimizer.
- [Public-context screen](https://github.com/taskqorg/agentq/tree/90d5e28%5E/benchmarks/public-methodology): context-mode inflated code context in the small screen. Source code, constraints and short results stay unchanged.
- Current engine decision: [agentq.md §3](../../agentq.md#3-managed-engines). Qualified RTK capture replay, Headroom transport, native Claude PreToolUse; no paid compression engine. No rewriting signed reasoning, tool schemas, IDs, user text or transmitted results.

## Repeatable checks

| Command | Chain and limit |
| --- | --- |
| `cargo test --locked` | Core capture, one execution, exit/stderr preservation, replay verification, timeout, engine/storage failure, accounting and recovery; deterministic engine doubles. |
| `python3 tests/provider_adapter.py` | Native HTTP/SSE, fragmented/interrupted body, WebSocket cancellation, typed/JSON/code-mode carriers, opaque reasoning, stable replay/restart, changed history, session isolation, corrupt/full storage; compressor double. |
| `python3 scripts/benchmark-compression.py --output <empty-dir>` | Actual installed pinned RTK: native frozen 180-test success/failure captures, Unicode error, exact recovery, code/JSON/constraints/short/unqualified/disabled/missing-exit preservation. Isolated state, no model calls or production config writes. `--binary` qualifies a candidate with installed engines. |
| `python3 scripts/check-client-compression.py both --output <empty-dir>` | Paid, opt-in native clients using current settings/auth: ordinary shell -> automatic compression -> model quotes real ref/error -> exact original recovery and session-correlated stats. No manual wrapper in the prompt. |

The benchmark writes per-scenario bytes, duration, fixture hash, recovery and verdict. A protected unchanged result is PASS, not a failure to force reduction. Native success must retain `180 passed` and no false failure; failure must retain the error marker and failure count. Original captures execute once before replay. No settings are weakened globally; Claude's test-only allowlist permits exactly its audited cargo wrapper and does not change hook permission decisions.

## Native client check rules

Binding detail of `agentq check compression` ([agentq.md § 3](../../agentq.md#3-managed-engines)).

- Use native CLI events, or that exact session's local Codex transcript when code-mode tool events are absent from stdout.
- Pytest reads a fixture without cache writes for read-only clients; never change native permissions.
- Require native execution, local event correlation, a model-visible ref and complete recovery. Missing clients, permissions or transcripts are unknown. Record exact versions; Desktop needs its own real turn.
- The intentional Cargo fixture failure may surface as 101 or shell-normalized 1 (PowerShell); retain the observed shell exit. Both need the unique fixture marker and complete original output. An exit code alone never proves fixture execution.
- Native code-mode output may be a typed text array: require one structured command result; ambiguous output stays unknown.

## Evidence boundaries

Byte reduction is not billed-token savings. The local benchmark does not qualify LongBench/RULER quality, cache savings, long-session eviction, model retries or paired billed cost. Those historical results stay historical. For cost comparisons freeze client/model/prompt, use paired A/B then B/A order and collect actual input/cache/output usage and latency; never infer these from bytes.

2026-10-15 staged Mac candidate: Codex CLI 0.159.3, 4414 -> 420 bytes; Claude Code 2.1.299, 4206 -> 320 bytes. Both quoted the actual original ref and error marker; recovery was exact. This staged result is separate from installed-artifact qualification. Windows and FRA native clients remain unqualified by these Mac runs.

Hermes terminal adapter cases run in the same offline benchmark: native success/failure capture -> shipped transform -> actual pinned RTK -> exact MCP original recovery, preserving the result envelope and native exit. These two cases do not certify a running Hermes gateway or model-visible delivery; qualify those on the installed host.

Pinned pytest replay mapping follows RTK v0.51.0 `src/cmds/python/pytest_cmd.rs`: only its conditional display-flag prefix is expected, followed by the exact native argv. The capture and exit code remain native; no command is rerun. An exact mapping regression test retains rejection of other argv changes.

## Worker runtime benchmark methodology and viewer

Binding detail of worker benchmarking across `codex`, `claude` and `antigravity` ([agentq.md § 12](../../agentq.md#12-agent-runtime), [PRODUCT.md R13](PRODUCT.md#requirements)).

### Methodology and task categories

The worker benchmark runner (`python3 scripts/benchmark-workers.py`) evaluates every supported worker client (`codex`, `claude`, `antigravity`) under paired optimization intents (`off` vs `on`) through `agentq agent turn`. It builds on the public methodology screen (`PUBLIC-BENCHMARKS.md`) and native CLI comparison (`CLI-COMPARISON.md`) by testing four standardized engineering task categories:

| Task ID | Category | Prompt & Fixture | Evaluation Rubric (`quality`, `context`, `adherence`) |
|---|---|---|---|
| `T1-context-navigation` | Codebase orientation & symbol lookup | Navigate an isolated workspace snapshot (`agentq.md` + `runtime/src/`) and report exact client codes, env variables and turn-end event types as strict JSON | `quality`: exact required facts matched; `context`: cited real files/symbols with zero hallucinated paths; `adherence`: valid JSON schema with exact requested keys |
| `T2-instruction-adherence` | Two-turn session start + resume with negative constraints | Turn 1 starts a session with strict format + forbidden-token rules; Turn 2 resumes the same session ID and verifies state retention and constraint compliance | `quality`: both turns answered and session ID preserved across resume; `context`: Turn 2 recalls Turn 1 token without re-prompting; `adherence`: zero forbidden words/markdown wrappers |
| `T3-noisy-command-compression` | Noisy test/build output analysis (`off` vs `on`) | Analyze a 180-test Cargo output capture containing one intentional failure (`MARKER_BENCH_FAIL_42`) and report pass/fail counts, failure marker and recovery reference when present | `quality`: exact `180 passed`, `1 failed` and `MARKER_BENCH_FAIL_42` extracted; `context`: pinpoints failing test symbol and line; `adherence`: concise structured diagnostic |
| `T4-code-patch-verification` | Bounded bugfix & verification | Diagnose a bounded off-by-one limit window bug in an isolated fixture file, return the exact minimal patch and verify test expectations | `quality`: patch fixes the off-by-one boundary without breaking existing cases; `context`: targets only the defective function; `adherence`: minimal diff, preserves comments |

### Recorded metrics and historical storage

Each benchmark run writes a versioned JSON artifact to `benchmarks/runs/<YYYY-MM-DD>-<git-short-sha>-<run-id>.json`, updates `benchmarks/runs/index.json` and `benchmarks/runs/latest.json`, and embeds the run catalog into `benchmarks/viewer/data.js` for the standalone viewer `benchmarks/viewer/index.html`:

- **Run metadata**: `schema` (1), `run_id`, `created_at` (ISO-8601 UTC), `git_sha`, `agentq_version`, `host` (`os`, `arch`, `hostname`), `execution_mode` (`live`, `hybrid`, or `deterministic-harness`), and `notes`.
- **Per-worker, per-optimization-mode, per-task record**:
  - `client` (`codex`, `claude`, `antigravity`), `optimization_intent` (`off`, `on`), `effective_optimization` (`off`, `on`, `unknown`), `optimization_reason`, `model`, `session_id`, `resumed`.
  - `duration_ms` (wall-clock turn duration), `outcome`, `error`.
  - Token consumption from `agent.turn`: `calls`, `input_tokens`, `cache_read_tokens`, `cache_write_tokens`, `output_tokens`, `total_tokens`, plus `raw_tool_bytes` and `delivered_tool_bytes` for compression tasks.
  - Rubric scores (`0`–`100`): `quality_score` (task completion quality), `context_score` (context orientation accuracy), `adherence_score` (instruction adherence), and weighted `overall_score` (`0.40 * quality + 0.30 * context + 0.30 * adherence`), with per-criterion check receipts.
- **Results viewer (`benchmarks/viewer/index.html` and `python3 scripts/benchmark-workers.py --view`)**:
  - Displays historical runs selectable by date, Git SHA, and run ID.
  - Compares `codex`, `claude`, and `antigravity` side-by-side with filters for `optimization: off` vs `on` and task category.
  - Shows paired token savings (`off` -> `on`), latency (`duration_ms`), cache read/write breakdown, quality/context/adherence scores, and drill-down inspection of every task's prompt, worker response, and rubric checks.
