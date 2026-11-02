# Worker capabilities and history: task 202

[Owner requirement and acceptance](https://github.com/taskqorg/agentq/issues/202):
“Owner requires spec-driven Codex/Claude/Antigravity parity, especially missing Antigravity transcripts. One reusable contract and normalized observation API for TaskQ.”

The additive `agent::history` API supplies seven source adapters, a capability readback,
and a bounded, recoverable source cursor. It changes no execution API, caller or live installation.
TaskQ UI and consumer adoption remain dependent work.

## Applied rules and scope

- P3 / M12.1: `RUNTIME.md` specifies required functions, matrix, ownership and the history wire contract before implementation.
- P1 / P2 / M12.3: native source remains unchanged; missing data and unsupported fields stay explicit. The cursor is not an execution or acceptance receipt.
- M12.4: history produces no telemetry or terminal observation, so existing turn accounting is unchanged.
- M12.5–M12.7: optimization, models, profiles and child environment are unchanged.
- M12.8: no new grant, reservation, launch, input, cleanup, release or replay authority. Existing protected preparation/completion stays authoritative.
- TaskQ §7 / R12: adapter tests, native evidence, publication and merge-result CI remain separate claims.

Touched surfaces: root contract and linked runtime detail; one exported library module;
one new contract test target and redacted source fixtures. No dependency, config, CLI,
execution parser, persisted execution checkpoint or consumer pin changes.

## Failure and passing protection

The initial affected test failed to compile with `E0432` because `agent::history` did not exist.
After implementation, `cargo test --locked -p agentq-runtime --test history` passed.

Independent read-only review found a partial-write termination error: `more` could be false
while a partial record remained. Adding the assertion first reproduced the failure:

```text
history_preserves_sources_and_recovers_without_replay ... FAILED
partial write must not terminate incremental observation
```

After changing `more` to reflect all unconsumed bytes, the same test and the full history target passed (3 tests); additional media/error/session-binding assertions extend the final target to 4 tests.
The caller supplies polling cadence/deadline; the library performs one bounded read, never a retry loop.

Retained risk protection covers exact raw records, native text/media/tool envelopes,
physical duplicates and order, source timestamps/unknowns, partial append and continuation,
checkpoint recovery without duplicate delivery, event/byte bounds, malformed JSON/UTF-8,
file replacement, truncation and consumed-prefix changes. Failed polls retain the cursor.
Existing filtered agent tests also passed: both Antigravity submission scenarios, native
receipt/recovery, app transport tools/drafts/catalogue and protected preparation/completion (5 scenarios).

## Source evidence and matrix conclusions

`runtime/tests/fixtures/history/provenance.json` records original retained-source hashes
and redaction. All non-protocol strings are replaced; identifiers are mapped consistently;
timestamps are replaced. Field structure and numeric values are retained.

| Source | Fixture evidence | Conclusion |
|---|---|---|
| agy headless | Retained #198 native stream, 10 representative rows | Text arrives as deltas/result; completed steps can omit text. Tools/errors remain attributable to steps. |
| Codex exec | Retained #199 native stream, 8 representative rows | Items, tools and terminal usage have different envelopes; absent user input is not invented. |
| Codex rollout | Retained native rollout, 12 representative rows | Response/event records share one bounded source API; schema extensions remain raw. |
| Claude native | Retained native JSONL, 3 representative rows | Message/attachment envelopes retained; the shared stream message shape uses the same adapter. |
| Codex app-server | Synthetic protocol notifications | Contract mapping only; no retained real notification fixture or native version/OS qualification. |
| Antigravity sidecar | Synthetic USER_INPUT/PLANNER_RESPONSE/TOOL_RESPONSE schema cases | Contract mapping only; truncated source is explicitly limited; no retained real sidecar fixture. |

All feature readbacks include support status, reason and test evidence. The selected native
version/OS is caller-supplied readback, not a version probe. Every new-history native tuple
remains `untested`; static adapter support does not qualify an installation.
Unknown source shapes retain their exact raw bytes rather than guessing parity.

## Coordination and open evidence

- #199 is the baseline protected lifecycle implementation. This work neither transfers its
  original operation nor changes its authority, sealed completion or recovery checkpoint.
- #198's [existing native receipts](../../benchmarks/prerequisites/198.md) prove their stated
  macOS execution/resume/hook behavior only. They do not qualify this new history API.
- A5 remains partial for retained-real-source coverage: app-server and sidecar are synthetic;
  Claude stream-specific control events also lack a retained stream fixture. Original real
  sources for these paths are needed before claiming native source parity.
- No model or paid calls, credential reads/copies, live session migration, deployment,
  native cancellation or account/model/permission changes were performed.
- Merge-result CI and downstream TaskQ adoption are not established by local checks.
