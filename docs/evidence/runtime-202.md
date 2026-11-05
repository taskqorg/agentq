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
| Codex app-server | Isolated codex-cli 0.159.3 / macOS arm64 initialize and thread/start capture | Real no-model control/notification envelopes; thread lifecycle, binding and emittedAtMs are tested. Turn execution remains unqualified. |
| Antigravity sidecar | Original #198 resumed optimization-off transcript, all 8 rows | Real user/planner/system/generic records, created_at, name/args calls and native truncation; GENERIC authorship is unknown. TOOL_RESPONSE remains a synthetic schema case. |
| Claude stream/control | Isolated Claude Code 2.1.316 / macOS arm64 initialize control response, plus retained TaskQ Claude lifecycle/result observations | Real no-model control envelope and request ID; no reply channel or native model-turn qualification is inferred. The TaskQ log is not a raw CLI stream. |

All feature readbacks include support status, reason and test evidence. The selected native
version/OS is caller-supplied readback, not a version probe. Every new-history native tuple
remains `untested`; static adapter support does not qualify an installation.
Unknown source shapes retain their exact raw bytes rather than guessing parity.

## Coordination and open evidence

- #199 is the baseline protected lifecycle implementation. This work neither transfers its
  original operation nor changes its authority, sealed completion or recovery checkpoint.
- #198's existing native receipts (git history at `33cef656`) prove their stated
  macOS execution/resume/hook behavior only. They do not qualify this new history API.
- A5 now includes the original sidecar transcript under the exact-source authorization
  in note 6576105503. Its bounded redacted fixture replaces the diagnostic projection.
  App-server and Claude control captures use real isolated no-model native processes.
  These sources prove parsing only; native model turns, permissions, cancellation and
  version/OS qualification remain separate. The transcript embeds no version; the #198
  evidence reports agy 1.4.12. No current native version was probed or inferred.
- No model or paid calls, credential reads/copies, live session migration, deployment,
  native cancellation or account/model/permission changes were performed.
- Merge-result CI and downstream TaskQ adoption are not established by local checks.

## Previous candidate checks and independent review

Code candidate `c14ef38c7e6ee3e403f061320deb7c3951379e6e` was rebased once on `origin/main` before first push.
At that hand-in, subsequent evidence edits changed no tested source, dependency or fixture.
Checks used `AGENTQ_SELFTEST_OFFLINE=1`; legacy home and npm prefix were isolated in OS temporary directories.

| Check | Observed outcome |
|---|---|
| `cargo test --locked` | 117 passed; 1 sandbox-blocked: `legacy::tests::edited_after_journal_is_stripped_not_refused`, OS error 1 `Operation not permitted` creating the isolated legacy fixture. Full gate is not PASS. |
| `cargo test --locked -p agentq-runtime` | 15 unit tests and 16 process scenarios passed; external-consumer build sandbox-blocked while writing `target/runtime-consumer/.../find_msvc_tools-*.d`, OS error 1. Full runtime gate is not PASS. |
| `cargo test --locked -p agentq-runtime --test history` | 4 passed on the final code candidate; run separately because the sandbox-blocked process scenario stopped Cargo before this target. |
| `cargo run --locked -- selftest` | PASS, `selftest: ok 0.0.0-dev macos-aarch64`. |
| Workflow-pinned `contract_lint.py agentq.md` | PASS, 0 errors; existing root-contract headroom warnings remain. Pin: `7e9adf57287553176d8b3be61c2b22565543dc51`. |
| Changed-document local links and `git diff --check` | PASS. |
| Independent read-only review | Task fit and quality reviewed separately; partial-write blocker fixed and rechecked. Remaining real-source fixture gap explicitly retained. |

The macOS sandbox failures are not passing evidence and do not authorize a grant change.
No CI result is claimed here. TaskQ review must check green merge-result (`merge_group`) CI before acceptance.

## Review repair (attempt 6575787694)

The reviewer rejected the previous candidate against A1/A3–A5. Five new regressions
failed on its implementation (4 previous tests still passed):

| Defect before | Passing repair / retained protection |
|---|---|
| Claude result exposed an unknown block, hiding final text | Final text is projected verbatim before the native lifecycle/error envelope. |
| Codex function/custom tool call used item ID while output used call_id | Both blocks correlate by call_id; item_id remains separate. Both call types are covered. |
| Future agy step became system/lifecycle, limited=none | Unknown kind/author and unsupported-source-shape remain; known system/checkpoint lifecycle stays. |
| Future Claude media had limited=none | Native unknown block remains exact and unsupported-content-block is explicit. |
| agy history advertised supported despite missing emitted text | agy and sidecar history readbacks report limited with transport-specific reasons. |

The same history command passed all five repairs. A sixth regression from the real
app-server capture first failed (`thread/started` was unknown); it now passes with
native thread binding and exact `emittedAtMs`. Claude's real initialize control response
is lifecycle-only and retains its native request ID. No user message or `turn/start`
was sent in either capture; each child was reaped. Both used empty temporary client
homes and inherited no auth environment or client configuration. Source hashes, version/OS and redaction are recorded in
`runtime/tests/fixtures/history/provenance.json`. Protocol role/type/status enums stay
literal, including previously over-redacted Codex `developer` roles.

The source, exports, execution APIs and callers were searched before repair. Only the
history adapter, its existing tests/fixtures, specification and this evidence change.
Original sessions, #199 authority/recovery, #198 hooks, grants, auth, models and
optimization remain unchanged (§12 M12.3–M12.8).

Independent repair review found two direct effects in the new real-source coverage:
null `truncated_fields` was falsely reported truncated, and native rollout `UserMessage`
was left unknown after restoring protocol enums. Each added regression failed before
its fix. Null/empty truncation markers now mean no omitted fields; malformed markers
stay explicitly limited. Rollout user authorship and enclosing native thread binding
are preserved; a foreign thread is rejected. The matching affected tests pass after repair.

### Repair gate readback

`cargo test --locked` again observed 117 passing root tests and the unchanged legacy
fixture's `PermissionDenied` at `agentq/legacy.rs:473` (34.49 seconds including build).
`cargo test --locked -p agentq-runtime` observed 15 unit and 16 process scenarios passing,
then the unchanged consumer dependency write failed with `Operation not permitted`
(8.56 seconds). Both full gates remain sandbox-blocked, never PASS. These failures
were not retried by changing permissions, fixture paths or assertions.

`cargo run --locked -- selftest` passed offline in isolated legacy/npm homes (0.63 seconds).
The workflow-pinned contract lint passed with existing headroom warnings (0.03 seconds);
its Git blob `29609754df6067ed504621ed4fc11f5a4a6dd995` matches the pinned remote file.
The later two history-only repairs use the affected history gate; neither changes the
blocked fixture, execution API, dependency or existing runtime unit/process behavior.

At the first repair hand-in (`23ec725`), A5 was incomplete because original sidecar
transcript JSONL was still required. The authorization and source check below resolve
that fixture gap. Green merge-result CI remains required for acceptance.

Final affected gate: `cargo test --locked -p agentq-runtime --test history` passed all
13 tests on the repaired sources and final redacted fixtures. Independent read-only
re-review passed the two direct findings and their regressions; it retained the original
sidecar-source gap. Changed-document local links and `git diff --check` passed.

The no-model source captures use `codex app-server` (`initialize`, `initialized`,
`thread/start` only), and `claude -p --input-format stream-json --output-format stream-json --verbose`
(one `control_request`/`initialize`, no user message). They are one-time source evidence;
regression tests read only the retained redacted records. They are not a native turn gate.

## Authorized original sidecar coverage

Owner note 6576105503 authorizes only the exact recorded #198 sidecar JSONL, without
new model turns or unrelated-session/account/credential discovery. The resumed
optimization-off source was read in one bounded pass (at most 8 MiB). It has 8 records,
7866 bytes, SHA-256 `031bd65ede0900d233de878cae28a4aebd7b64ba41876dfad8168e299357737f`.
A second exact-source read confirmed identical bytes after redaction. The source and
live attempts were not changed. No full-transcript fallback or other native file was read.

The retained fixture preserves every original record field and numeric value; all
non-protocol strings are redacted, protocol enums and truncated field names remain,
and source timestamps are replaced. No diagnostic fields are added. The provenance
records source hash/size, OS, the reported #198 version and its evidence; absence of a
version in the source is explicit. No native parity claim follows from these fixtures (§12/P2).

The original-source regression first failed on missing `created_at` (`time: null`),
then on sidecar `name`/`args` calls (`unknown` instead of `tool_call`), then on omitted
`GENERIC` text. The same check passes after the adapter projects exact source time,
untyped calls and text without guessing `GENERIC` authorship. `SYSTEM_MESSAGE` is a
system lifecycle event. Absent call IDs stay null; original `truncated_fields` still
limits the content. A separate future-typed-call regression failed before adding the
shape guard and passes afterward, retaining unsupported content as native unknown.

The diagnostic projection fixture was removed. Original-source tests retain its
content/order risks; the independent null/empty/malformed truncation regression
continues to protect the earlier failure. The seven-format incremental/recovery check
now uses the real sidecar fixture and retains exact raw rows, physical duplicates,
partial appends, prefix/file identity checks and no input replay. No execution API,
consumer, grant, auth, model, hook or #199 completion/recovery path changes.

### Final candidate checks after original-source coverage

| Check | Observed outcome |
|---|---|
| `cargo test --locked -p agentq-runtime --test history` | PASS, 15/15; original-source and future-call regressions failed before their fixes and passed after. |
| Independent read-only review | PASS, A5 original-sidecar evidence gap resolved; task fit and implementation quality checked separately; native qualification remains separate. |
| `cargo test --locked` | Sandbox-blocked, not PASS: 117 passed, unchanged legacy fixture write denied at `agentq/legacy.rs:473`; 54.41 seconds including build. |
| `cargo test --locked -p agentq-runtime` | Sandbox-blocked, not PASS: 15 unit checks and 16 process scenarios passed, then consumer dependency-file write denied (`Operation not permitted`); 10.97 seconds. |
| `cargo run --locked -- selftest` | PASS, offline with isolated legacy/npm homes; `selftest: ok 0.0.0-dev macos-aarch64`; 0.89 seconds. |
| Workflow-pinned `contract_lint.py agentq.md` | PASS, 0 errors, existing headroom warnings; 0.07 seconds. |
| Changed-document links, fixture/provenance inventory, `git diff --check` | PASS. |

The full gates ran once on the final code candidate after the authorized source repair.
No permission, legacy assertion, dependency path or sandbox rule was changed to bypass
the two host failures. There was no rebase after the first push, no main-movement retry,
no new model turn, no deployment and no native session/auth/grant/model mutation.
Native version/OS qualification and native variants absent from source fixtures remain
explicitly limited or untested in the matrix. Merge-result (`merge_group`) CI is not
established by this worker; TaskQ review/publication must enforce it (TaskQ §7/R12).
