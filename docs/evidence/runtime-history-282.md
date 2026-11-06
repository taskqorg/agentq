# Universal step history: Mac readback

Candidate: the commit containing this evidence; no deployment or client configuration change.

## Native sources

Read-only discovery and `History::steps` were exercised on native Mac transcripts.
A temporary Rust probe used `Runtime::history_sessions(None)`, then read each selected
source with `History::steps(path, u64::MAX)` and counted step kinds without printing content.
Selection: the first forty sources plus every discovered fork; Claude had no fork entries.

| Source | Files | Bytes | Derived steps | Unknown steps | Elapsed (debug) |
|---|---:|---:|---:|---:|---:|
| Codex rollouts | 97 | 662889946 | 36357 | 6403 | 16.140 s |
| Claude native | 40 | 9906646 | 1993 | 638 | 2.201 s |

This is decoding evidence, not a latency comparison with the older probe: the corpus,
step cardinality and output shape differ. Raw content remains local and unchanged.
The retained fixture versions are listed in `runtime/tests/fixtures/history/STEPS.md`.
Installed versions observed: codex-cli 0.159.3, Claude Code 2.1.319, agy 1.5.2.

## Fork failure

A read-only scan found 61 rollouts containing multiple session metadata IDs. Every
first metadata ID matched the rollout filename. These fork files start with the
child's metadata (`forked_from_id` naming its parent), then copy parent metadata.
Working event thread IDs in inspected examples identify the child. The previous
per-record equality check rejected the copied parent metadata, so this is a reader
bug rather than evidence that the probe chose the wrong first ID.

The reader retains subsequent metadata with `session_binding=provenance`; initial
metadata and working event IDs still validate against the caller's bound session.
A regression checks both the fork and a foreign working event, including unchanged
checkpoint on refusal. All 61 fork sources passed the native read probe.

## Qualification limits

Offline tests exercise all existing transport fixtures, new scrubbed completed-item
and Claude result records, stable revisions, source-range reads and a 17 MiB shell
output. Repeated reads and recovery do not execute native clients.

Codex app-server live output has a typed outputDelta decoder; this change does not
claim a model-turn native qualification of that transport. Claude/Antigravity live
console output remains unsupported. Antigravity persisted steps retain their stated
truncation and authorship limits. Windows native history/resume/media proof remains
open for the owner's Windows host; portable adapter CI is separate evidence.

## Revision convergence and source identity review

The review found real failures in the initial step derivation: a later Claude result
replaced its shell command with null; metadata blobs replayed embedded tool calls;
parentUuid was incorrectly treated as a task-parent link; string prompts lost text;
and output deltas were not cumulative. The initial fixture also retained a private
path in a map key and native integer timestamps. Those are fixed in this candidate.

Derivation now folds source records at read time, producing self-contained revisions
and one latest step per ID per page. `origins` retains every contributing raw range.
Recovered page reads rebuild the same state from raw bytes, with no derived cache.
Metadata replay records remain raw unknown steps. Child `agentId` is scoped by its
parent; opaque source IDs distinguish physical copies and locations. Source opening
resolves parent-call links inside the adapter and does not require a caller path.

A fresh read-only probe used the reviewer's source lists, seed 7, ten forks/children
and thirty/forty ordinary files, selecting ordinary files at most 8 MiB (a probe
selection only, not a runtime ceiling). Files missing since the review were excluded.

| Source | Files | Bytes | Converged steps | Unknown steps | Unknown share |
|---|---:|---:|---:|---:|---:|
| Codex | 40 | 182467420 | 10908 | 667 | 6.11% |
| Claude | 50 | 36294775 | 6178 | 3057 | 49.48% |

All **527/527** unique native Claude Bash commands were retained in their latest shell
steps, with **zero** message steps missing text and zero source-read errors. The
unknown percentage is measured after coalescing action revisions: Claude metadata
is retained while duplicate tool revisions no longer inflate the denominator. It
is not directly comparable to an uncoalesced record/step percentage.

Fresh Claude discovery returned **5503 unique source IDs**, including **315** child
sources. **43** real parent sources with subagent files opened by opaque ID without
passing a source path; none returned an ambiguity error. Native working files remained
unchanged. The count is a live snapshot, not a permanent source inventory.

Code-mode `exec` deliberately remains a composite tool: arbitrary JavaScript does
not prove which shell commands executed. The step keeps its complete code/output
and projects structured console fields when actually present; emitted native
CommandExecution records remain shell steps. Native token_usage_record and
SubAgentActivity have explicit mappings. The Codex sample has 61 child-session links in 65 subagent/control steps; the four
without a single child ID are native `wait` operations, not spawn results. Unknown
child targets are not invented.

Regression checks cover command/cwd/kind preservation, cumulative deltas across a
recovered page boundary, metadata replay, native parent/child call linkage, prompt
strings, code-mode semantics and fixture path/timestamp scrubbing. The legacy
session_binding vocabulary remains native/caller; inherited metadata has a separate
provenance flag. Terminal resume argv is advisory; verified writer fencing remains
#287, and native subagent terminal resume is explicitly unsupported.
