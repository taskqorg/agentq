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
