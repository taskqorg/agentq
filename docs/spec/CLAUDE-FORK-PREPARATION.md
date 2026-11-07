# Claude fork: bounded source evidence and refusal preparation

This is an evidence/preparation note, not a new API or protocol contract. It preserves F300-20261107-1 signatures and authority semantics, immutable Codex implementation `98cf38870fe1a9ed7fa78842a44ea060bdd94ecf`, and its unaccepted coherent #285 development dependency. Claude advertises neither `fork` nor `rewind`; both remain the existing `fork-unsupported` / `rewind-unsupported` refusals. No successful Claude fork, native qualification or full #300 acceptance is claimed. No L2 field or history/archive/sink source is consumed or changed.

## Exact inspected sources

Official npm package `@anthropic-ai/claude-agent-sdk@0.3.284` declares packaged Claude Code `2.1.284`. Tarball integrity: `sha512-NSoJwEq6nFSf8dtaacYx37QdGgqApI3eHFUlxclMLYi8irb6ZJwEaUnPnLUCyAyg9W/tMkgZC0GXWLRCc01I0w==`. It was read as data, never installed or executed.

| Artifact | SHA-256 |
|---|---|
| Official npm tarball | `4550e830246026133fc1802a2208dd0f3a785cae1eec83f261d114c33d797771` |
| Extracted `package/sdk.d.ts` | `048ae2e6c796cc2aa3c423afaad59a08972cb48c271ffcc9847d910ff65f61b2` |
| Extracted `package/sdk.mjs` | `32d062c37b03e10870fbf839f54694545ee01bc0ec719e47078fbed76e30ef71` |
| [Official CLI reference](https://code.claude.com/docs/en/cli-reference.md), retrieved 2026-11-07 | `1d7b735d97fa016124d3fd336223c8c17d463ea4a7dd1dfda48a772d8ae0073a` |

Line numbers below refer to this exact `sdk.d.ts`. The CLI reference snapshot lines 94 and 128 document `--fork-session` and explicit `--session-id`. The SDK JS mapping emits `--resume`, `--fork-session`, `--resume-session-at=<uuid>`, `--resume-drops-turn=<uuid>` and `--session-id=<uuid>` from their corresponding options. This is source evidence of client options, not execution evidence of the native CLI boot path or a pre-prompt receipt.

## Interface / invariant boundary

| Existing interface and source location | Establishes in inspected source | Does not establish |
|---|---|---|
| `Options.resume`, `sessionId`, `resumeSessionAt`, `resumeDropsTurn`, lines 2071–2139; `forkSession`, line 1702 | Requested distinct child UUID on fork/resume; inclusive native chain-entry cursor; headless-only truncation guard | Applied child identity, loaded prefix, effective conditions or no autonomous work before prompt |
| Offline SDK `forkSession()`, lines 817–843 | Native transcript copy/remapping operation; no copied file-history snapshots | An authorized owned Rust transport path; no transcript-copy implementation is added |
| `getSessionMessages()`, lines 884–921; `SessionMessage`, lines 6362–6378 | Read-only JSONL conversation-chain projection using `parentUuid`; user/assistant and optionally system messages | A complete terminal chain-tail/attachment witness, physical generation binding, loaded child prefix or live child authority; missing session returns an empty array |
| `initialize`, lines 4499–4538 | Commands, agents, model catalogue, account and hook/plugin application metadata | Typed applied child session/model/permission/prefix receipt |
| `get_binary_version`, lines 3913–3918 | Binary version read request | Child identity, boundary or effective conditions |
| `get_settings`, lines 4185–4190; SDK JS `getSettings()` | Effective merged settings are described; JS returns opaque response | A discovered typed pre-prompt applied child model + permission + prefix contract; no such guarantee was found |
| `list_permission_rules`, lines 4774–4791 | Live rules, workspace directories, original cwd, managed-only state and settings errors | Current permission mode, applied model, child retained prefix or autonomous-work suppression |
| `get_context_usage`, lines 3922–3956 | Response model and context usage; `summary` avoids per-category token-count API calls | Child permission/prefix/identity/no-autonomy receipt; default `full` performs token-count API calls and was not invoked |
| `system/init`, lines 5828–5902 | Per-turn session/model/cwd/permission metadata | A promise of emission before first prompt; prompt must not be submitted to obtain the evidence |
| `system/session_state_changed: idle`, lines 5780–5789 | Turn-over after held result flush and background-agent loop exit | Terminal native chain cursor, child startup/prefix or future suppression of autonomous work |
| Control-request union, line 4931 | Enumerated controls, including separate file rewind | No conversation fork/revert/read-chain control was found in that union; this is not proof that no native implementation can expose one |
| `background_tasks`, lines 3849–3857 | Mutating Ctrl+B equivalent | Read-only task inventory; it must not be sent as an observation query |
| `prewarm` / `SpareProcess.claim`, lines 9240–9258, 9300–9313 | Claim binds a session and sends its first message; model/thinking option refusal can occur while the prompt runs without that option | The required pre-prompt effective-condition guarantee; no spare-process transport is introduced |

The SDK explicitly states that end-turn tool output can end in a successful tool-result carrier plus `structured_output` attachment after the last assistant UUID (lines 2105–2137). Transcript-only appends also matter. Native chain UUID is not Anthropic `message.id`. `Step.state=completed`, a completed assistant block, or a result alone cannot manufacture the kept whole turn's last chain entry. Interactive/background boots ignore the truncation pair (lines 2123–2129). The SDK's plain-resume recovery advice does not authorize a fallback that changes F300 selection.

## Current controlled producer and useful preparation

`registry/claude/control.rs::Session::open` uses ordinary caller hooks and native lifetime ownership, holds controls, initializes, then immediately submits ordinary input. `exchange` validates/observes a correlated response but returns no payload. `writable` requires the original live transport/current owner and refuses uncertain/closed state; recovered sessions have no writable transport. The producer contains no child-before-input verification stage or native terminal chain-tail resolver. Those facts do not justify adding dormant fields, a speculative parser, or an unused startup wrapper.

The reusable slice is a focused refusal proof through the existing owned Claude fixture. With pending requests, terminal results, foreign ownership, uncertain native state and recovered observation-only runs, `None`, any available observed assistant Step and a caller-forged `completed` claim still refuse. Refusal must not call child preparation, submit input, create child log/owner/lineage, alter the source checkpoint/log/owner bytes/frozen snapshot, or change a working-file sentinel. It tests the existing default boundary; it does not simulate successful native fork, prove a native chain cursor, or add a production resolver.

The first actual implementation still needs distinct child admission and durable private selected-boundary lineage before native creation, one owned child transport, authoritative child identity/retained-prefix/effective-condition evidence and unchanged-parent readback before exactly one explicit prompt. Current owner, canonical scope, known idle state, no pending/uncertain operation or stop, and durable unknown-mutation fencing remain required. Recovered observation grants no mutation authority. Archive delivery cannot be a prerequisite for local durability. No copied-prefix billing or file restoration is inferred.

## Minimal history-owner factual input

For an existing exact Step, the adapter needs revalidated native records bound to the owned physical source/checkpoint/generation, the selected prompt/whole-turn association and its last actual retained chain-entry UUID, including carrier/attachment/compaction/side-chain cases, or an explicit unknown/refusal. A native UUID from a stream with no witnessed physical mapping is insufficient. Keep native JSONL, snapshots, source labels and scrubbed archive domains separate; existing `Source.parent_step` remains a native call-ID field. No shared type/signature or general history hook is proposed or implemented here.

Even that history witness would establish only source selection. It would not establish the child's loaded prefix, applied model/policy or no-autonomous-work state before input. The inspected SDK/CLI interfaces do not currently supply a proven complete solution to those pre-prompt requirements. Native availability remains unexecuted/unknown, not disproved. Any native-only experiment or semantic decision belongs to the owner; guarantees must not be relaxed silently.

## Exhaustive remaining gaps

Actual Claude native fork, distinct child admission, pre-prompt child readback, durable unknown-child mutation handling and different-model qualification remain unimplemented. Empty-prefix fork, broad/historical/snapshot selection, carrier/attachment/compacted/side-chain selection, goal-bearing/autonomous-work safety and extra effective-condition mapping remain gaps. Claude conversation rewind remains unsupported; file rewind is a separate native operation and no file-state restoration is implemented. Codex98cf limits remain unchanged. Accepted coherent #285, independent complete new-source review and later authorized Windows/native qualification remain required. No full suite, native/model/auth/paid/prompt/export action, CI, consumer pin, push/PR/merge/tag/deploy occurred in this slice.
