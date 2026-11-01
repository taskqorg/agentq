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

The worker benchmark runner (`python3 scripts/benchmark-workers.py`) evaluates every supported worker client (`codex`, `claude`, `antigravity`) under paired optimization intents (`off` vs `on`) through `agentq agent turn`. It tests four standardized engineering task categories:

| Task ID | Category | Prompt & Fixture | Evaluation Rubric (`quality`, `context`, `adherence`) |
|---|---|---|---|
| `T1-context-navigation` | Codebase orientation & symbol lookup | Read the generated fixture `contract.txt` and `src/worker.txt`; report exact client codes, home variables and terminal event types as strict JSON | `quality`: exact required facts matched; `context`: exact facts from the two fixture files; `adherence`: valid JSON schema with exact requested keys |
| `T2-instruction-adherence` | Two-turn session start + resume with negative constraints | Turn 1 starts a session with strict JSON, no-file-write and no-markdown constraints; Turn 2 resumes the same session ID and verifies state retention and constraint compliance | `quality`: both turns answered and session ID preserved across resume; `context`: Turn 2 recalls Turn 1 token without re-prompting; `adherence`: zero markdown wrappers or fixture file writes |
| `T3-noisy-command-compression` | Noisy test/build output analysis (`off` vs `on`) | Analyze a 180-test Cargo output capture containing one intentional failure (`MARKER_BENCH_FAIL_42`) and report pass/fail counts, failure marker and recovery reference when present | `quality`: exact `180 passed`, `1 failed` and `MARKER_BENCH_FAIL_42` extracted; `context`: identifies the failing test symbol and marker; `adherence`: concise structured diagnostic |
| `T4-code-patch-verification` | Bounded bugfix & verification | Diagnose a bounded off-by-one limit window bug in an isolated fixture file, return the exact minimal patch and verify test expectations | `quality`: patch fixes the off-by-one boundary without breaking existing cases; `context`: targets only the defective function; `adherence`: minimal diff, preserves comments |

### Execution and recorded evidence

`python3 scripts/benchmark-workers.py --mode live --binary <candidate>` runs the same isolated fixtures through `agentq agent turn`. It uses the caller's native authentication, model, permissions and optimizer attachment without changing configuration. `--client-args <JSON-file>` supplies caller-owned per-client arguments; no approval or sandbox flags are added. Paid calls run only with explicit `--mode live`. Missing clients, failed turns, absent counters and ineffective optimization remain visible as unavailable, failed or unknown; there is no synthetic fallback.

Default `--mode smoke` uses deterministic protocol doubles in OS temporary directories. It checks start/resume, JSON evaluation and the viewer pipeline without model calls. Every artifact and cell says `smoke`; emitted synthetic counters are discarded. Smoke artifacts prove no model quality, native compression, latency or token savings. They are retained as historical runner checks, never dated retroactively or presented as live benchmarks.

Each run retains schema 1, a unique UTC run ID, source SHA and dirty flag, binary hash/version, runner hash, Python/OS/architecture, fixture hash, mode and all 24 cells (3 clients × 2 optimization intents × 4 tasks). Pair order alternates off/on and on/off. Cells retain all turn reports, prompts, final responses, exact rubric checks and quality/context/adherence scores. Quality measures the fixture result; context measures the task-specific facts or resumed memory; adherence measures exact JSON/constraints. Overall score is `0.40 * quality + 0.30 * context + 0.30 * adherence`. Scores describe only these fixtures.

Usage totals are null if any contributing counter is absent. Latency uses observed monotonic wall time, never a profile table. Requested `on` and effective state/reason appear separately; effective `on` is configured, not proof of compression. Savings require a complete live pair, the same observed model, successful task checks, known input counts, effective off/on, and session-correlated compression evidence; otherwise the delta is unknown with a reason. There is no cost estimate or inferred billable-token total. Live cells read `agentq stats --records` after the turn and retain only records matching that client and its newly created native session; errors stay unknown. Raw/delivered tool bytes sum these attributable records when known; native compression qualification uses the existing checker above.

Runs are immutable JSON files under `benchmarks/runs/`. `--rebuild-viewer` generates `benchmarks/viewer/data.js` from their actual contents. Open `benchmarks/viewer/index.html` directly, or use `--serve <port>` (loopback only). The standalone viewer selects historical runs, filters client, optimization and task, compares observed usage/latency and scores, and expands prompts, responses, rubric checks and unknown reasons. Text is inserted safely; no network or external assets are required. `--selftest` protects unknown handling, scoring, immutable storage and the no-fallback boundary without paid calls.
