# Independent protected host

Owner request: [AgentQ #241](https://github.com/taskqorg/agentq/issues/241), acceptance A1–A4. Signing decision: [accepted answer](https://github.com/taskqorg/agentq/issues/241#issuecomment-6665555869). Applied rules: §12 M12.2, M12.3, M12.6–M12.8 and P2/P3; TaskQ R12/R13 and §7; specq Testing development/candidate policy.

The native gate and scope code derive from the immutable [TaskQ source](https://github.com/taskqorg/taskq/tree/5292e63ffaeda41ca90d1b2e21b93f78fe2d3e39/src/runtime), with product-owned admission, policy, account and signing dependencies removed. This is source provenance, not a runtime dependency.

The independent host is part of the existing AgentQ binary and runtime library. It requires no TaskQ binary, command, slot service or certificate authority. Its Ed25519 signature and pinned digest do not provide macOS signature checks or TCC grants. Windows verifies the same signature; native containment remains macOS-only. No live policy, account, model, capacity, integration, service or grant was changed here.

## API and authority

`ProtectedHost::signed(executable, sha256, public_key, signature)` selects only `agentq agent host`; it refuses legacy request schemas rather than falling back. The explicit legacy constructor remains source-compatible for existing consumers. A consumer independently pins the published public key and digest; it must not learn trust from task input, environment or an operation receipt.

The host reads only the OS user's fixed `.codex/agentq-host`. Its `trust.json` is the adapter manifest. Existing operator authority is imported into `policies/<sha256 of canonical project UTF-8 bytes>.json`; it is not taken from project configuration or a receipt. The envelope is `{ "payload": "<exact JSON text>", "signature": "<hex>" }`. Ed25519 signs the domain, one NUL byte, then the exact payload bytes. Domains are `agentq.protected-adapter/1`, `agentq.protected-policy/1`, `pipe.permits/1` and `agentq.retained-operation/1`; the adapter payload is its lowercase hexadecimal SHA-256, not raw executable bytes.

A policy payload has `schema: "agentq.protected-policy/1"`, `authority`, `profile`, `args`, and `permit_public_keys`. `authority` retains the existing canonical `project`, `runtime: "codex"`, `account`, `account_home`, `executable`, `executable_sha256`, original `profile_sha256`, `requested`, `resolved.effective: "workspace"`, and `saved` revision plus explicit project grant (`mode`, `runtime`, `project`, `by`, `at`). `profile` is the original approved Seatbelt profile; `args` are the existing app-server config arguments, including the outer-boundary mapping. The host adds protection for its own control store and adapter/native image directories. Missing, changed, Full, unknown or unsupported authority refuses. Signed policy replacement is the independent operator revocation channel; an unsigned/stale project request grants nothing.

`permit_public_keys` maps the immutable caller name to Pipe's existing permit issuer public key. A permit is `{ "binding": <binding>, "key": "<private 64 hex>", "signature": "<128 hex>" }`. Its binding has exactly `schema: "pipe.permits/1"`, `caller`, `op_id`, `root`, `runtime`, `pid`, `started`, `boot`, `expires_at`. The signature covers compact UTF-8 JSON with keys in this order: `{"binding":<binding in its retained original field order>,"key_sha256":<SHA-256 of the lowercase key text>}`. Preserve those bytes/order when signing; do not normalize an existing operation. The host validates issuer, payload/key, actual preparation parent and process creation/boot. Expiry applies to first preparation; existing occupancy survives expiry. Pipe alone owns admission, its protected issuer key, occupancy and final release acknowledgment.

`prepare` accepts only `schema` and `permit`. Retain the independent checkpoint and a durable one-shot intent before `launch_prepared`; persist the private original permit/key. `observe`, `cancel` and `reconcile` target only that request. Cancellation seals input and signals the exact native incarnation; neither cancellation nor root/transport exit proves completion. Host cleanup never releases Pipe capacity. The journal is locked, bounded to 16 MiB and never evicted/reset to recover occupancy. Corrupt, lost or full state refuses effects and remains unknown.

## Retained operations

`ProtectedHost::reconcile_retained(checkpoint_bytes, receipt_bytes, deadline)` invokes only the independent `retained` action. Operator authentication wraps the two exact original JSON strings in a signed envelope under `retained/op-<sha256(caller + NUL + op_id)>/<sha256 of exact receipt bytes>.json`. No TaskQ runtime or CA is used. Later retained observations append signed snapshots to the same operation directory; they never overwrite an earlier receipt. Original receipts and the old adapter identity remain unchanged. `cancel_retained` requests exact native stop only after live parent, image/hash and scope readback; its durable one-shot intent never proves completion. Import does not manufacture a certificate, grant or dispatch authority. A matching new permit cannot launch an archived operation.

A missing or changed archive, private key/binding mismatch, missing descendant certificate or uncertain original-generation cleanup remains unknown and occupied. A valid original certificate can be checked against the same-boot kernel coalition and exact launchd job; independent bootout intent, acknowledgment and exit proof live separately in the host journal, in one fence per operation bound to its native incarnation. Request and receipt bytes do not name a fence, so a reformatted request or another signed snapshot of that incarnation cannot repeat a stop. A snapshot that carries original cleanup intent consumes the fence, and a changed incarnation refuses. Lost bootout acknowledgment, including an interrupted original-host cleanup, is never retried. The caller's original retained bytes are returned unchanged; replay is never part of recovery.

## Operator provisioning

Do not import a policy or receipt from untrusted task output. Use only existing operator-approved authority and the original caller-retained private evidence. No grant-creation command is supplied. Provisioning is separate from building or publishing this source.

The operator creates the private host directory with user-only permissions, then generates the dedicated key with `cargo run --locked -p agentq-runtime --example sign_adapter -- keygen <private-key>`. The command uses exclusive creation and never prints the key. `sign <private-key> <final-agentq-binary>` prints only the public manifest; pin that manifest independently and place it in the private host's `trust.json` when explicitly provisioning. `seal <private-key> agentq.protected-policy/1 <existing-policy-json>` creates its operator envelope; redirect it directly into protected storage. The same utility seals `agentq.retained-operation/1` archives. Private policies/receipts must never be sent to the board or CI logs.

For an empty, never-used host, the operator provisions `initial.json` as `{"schema":"agentq.host-initial/1","operations":0}`. The host consumes an exclusive initialization marker before persisting the empty journal. Losing the journal afterward cannot silently initialize another one. Back up the private key, signed policies, original permits/checkpoints/receipts and journal under the caller's existing protected storage procedure. Restoration preserves exact original identities and uncertainty; it never grants another launch.

## Verification and limitations

The existing preparation process scenario now runs against both explicit legacy and signed independent wire contracts. It preserves drift refusal, every-input incarnation checks, cancellation, descendant/cleanup requirements, immutable operation identity and no-replay checks. The independent double accepts only `agent host`; native containment remains separate. Portable host checks cover signatures, issuer/key/payload substitution, policy drift, lost/corrupt journals, consumed launch intent across recovery, surviving descendants, regressed/reordered counters, partial cleanup and authenticated exact-byte archive import.

Before the digest fix, `cargo test --locked -p agentq-runtime --test agent -- protected_preparation` failed at `/receipt/preparation`: a different well-formed digest recovered successfully. Recovery now recomputes and checks the independent preparation digest; the same assertion is retained. Initial independent-double runs also exposed incomplete fixture schema conversion; only the invented wire serialization was corrected, with the original no-replay assertions retained.

Native authority stays unqualified locally: neither portable checks nor artifact signing prove native Mac boundaries, TCC grants, Windows containment or consumer Pipe integration. No model turn, image generation, accepted-crate replay, new grant or account/capacity change occurred. Merge-result CI remains mandatory for publication; the worker does not wait for it (TaskQ §7).

The candidate review also found that importing an interrupted original-host cleanup must preserve its consumed intent. The independent reader now refuses another bootout when the original acknowledgment is missing; an existing acknowledgment allows only exit observation. Retained snapshots append under their receipt digests, so later evidence does not rewrite an original receipt. A separate exact-incarnation retained cancellation path persists one-shot intent and never reports completion from a stop request.

## Review corrections

The publication review requeued the candidate at `8a3f58e9e01832873372a0b05f9106fbf52cd9d1` with two defects. Both are fixed at the source pin below, and the digest published with that candidate is superseded.

**Retained stop fence.** Cleanup and cancellation were fenced by a hash of the raw request, so request formatting or another snapshot created a new fence and repeated a bootout or native stop after a lost acknowledgment. `protected_retained_fences_bind_operation_and_incarnation_not_bytes` checks a compact request, a reordered and reformatted request, a pretty-printed request and a later signed snapshot of one incarnation, each with a lost acknowledgment.

- With the fence keyed by request bytes again, it fails: `one incarnation gets one stop of each kind`, left `(3, 3)`, right `(1, 1)`.
- With the incarnation keyed by snapshot bytes again, it fails: `retained cleanup binding changed`.
- With the fix, it passes with one bootout and one stop. A changed incarnation refuses and leaves the journal bytes unchanged; an older snapshot after inherited intent issues no effect.

The stop itself runs behind a closure in these checks, so they prove the fence and not a live launchd or process stop.

**Windows journal sync.** The journal synced through a read-only handle, which Windows refuses. [CI run 42086892150](https://github.com/taskqorg/agentq/actions/runs/42086892150) failed `protected_independent_signatures_permits_and_missing_state_refuse` with `protected journal sync unknown`. The handle now has write access and the assertion is unchanged. `rust (windows-latest)` passed in [CI run 42088896349](https://github.com/taskqorg/agentq/actions/runs/42088896349), together with `rust (macos-latest)` and `rust (ubuntu-latest)`. That is a pull-request result, not the merge result.

**Cross-action incarnation drift (second review).** Candidate `5688cc8a2e9d83a40a3398ca7fe50fb7f11510f8` still let cleanup and cancellation bind separate incarnations of the same operation. Under R13 / §12 M12.8, the existing retained-fence regression now also exercises cleanup A followed by cancellation B and cancellation A followed by cleanup B, using authenticated snapshots with different native incarnations.

`cargo test --locked -p agentq-runtime --lib protected_retained_fences` failed before the fix on Windows: `cross-action incarnation drift must cause no stop`, left `2`, right `0`. After the fix the same check passes, with zero second-action stops and byte-identical journal state after each refusal. Both action records are checked under the journal lock before persisting either intent, including a recheck after native readback. Existing action-only journals bind both actions without migration, receipt rewriting or intent reset. Inherited cleanup checks the same binding. The earlier formatting, snapshot, lost-acknowledgment, original-intent and no-replay guards remain unchanged.

The two earlier requeues exposed the same missing relationship at different boundaries: byte-based fences first, then separately bound action fences. The smallest remedy is the shared operation/incarnation check in the existing journal, with both cross-action orders retained in the existing deterministic regression. This adds no new authority, storage or launch path.

## Immutable candidate

- AgentQ runtime/API and locally built Windows adapter source: `12507a213df2e6244e6163589bc92b5516ddcd8d`.
- macOS adapter source: PR build merge `385dd54967d9c9456b80a11fefc2a5b45ad57e7c` (parents `1e364e505e797f36e61682100ced6b51f83f230e` and `12507a213df2e6244e6163589bc92b5516ddcd8d`). This includes newer main changes; the protected host, journal/permit/scope code, `ProtectedHost`, signing utility and command dispatch are identical to the locally tested source. Their diff was checked explicitly. The CI checkout log binds the binary to this exact merge, not the Windows source tree.
- CoreQ: `v0.19.2`, lockfile revision `b78ca64cac615500b57b4a04983e79333035dd65`.
- Native containment source provenance above is retained only as history; the built binary links no TaskQ crate and dispatches no TaskQ command on the signed route.
- The signed artifacts and public manifests accompany the task result. This attempt generated a dedicated signing key in private Windows host configuration, outside the repository and the host authority store. Its directory has inheritance disabled and grants access only to the current user and SYSTEM, verified by ACL readback. The key is not in this repository, artifact or board. No policy, trust manifest, initial journal, permit or grant was provisioned into a working host. Same-user uncontained processes remain outside the private-file boundary.

## Merge with newer main

After the publication review, main moved past the candidate with [#242](https://github.com/taskqorg/agentq/pull/242) and [#243](https://github.com/taskqorg/agentq/pull/243). The branch merged main at `7fa3b83e4dbeed67d2fa87813f6cf8669d158645` and was not rebased, so both source pins above stay reachable from it.

`git diff 12507a2 7fa3b83` is empty for `runtime/src/agent/protected.rs`, `runtime/src/agent/host.rs`, `runtime/src/agent/host/`, `runtime/examples/sign_adapter.rs` and `agentq/host.rs`. The merged tree builds with CoreQ `v0.21.0`, revision `18b177c445e92c5df58bd1dea1b473776dd0c6ae`.

The signed artifacts were not rebuilt. They remain binaries of the pinned sources with CoreQ `v0.19.2`, so they do not contain #243. A consumer that needs #243 inside the adapter needs a new build and a new signature.

The publication reviewer must require green `merge_group` CI for this PR's merge result (private fallback: exact-SHA CI); PR/head success alone is insufficient.

## Final artifact and checks

The final Windows x86_64 adapter is the 11,123,352-byte AgentQ release binary built with `cargo build --locked --release --bin agentq` from the source pin above. SHA-256: `9ee905960a57dbe4649c42626a2a5c6b1ed871b440ecfbfa5aebe8cff71fb7ba`. Its [public detached manifest](runtime-241/adapter-windows-x86_64.json) contains the Ed25519 public key `7f739abab8625718e2e5dbedf0deb5a98373260d3c22ecd20a1148f42bf6d584` and signature. This is a new publication key, not a change to any working host's trust. A consumer independently pins the key and artifact digest. Signatures do not modify the binaries. The task attachment is not an installed or native-qualified release.

The final macOS arm64 adapter is 7,146,016 bytes, SHA-256 `425c5924046338bf13167e5bf76eb827533435ed09e5f3cf336b6744d9bf2118`. The [macOS manifest](runtime-241/adapter-macos-aarch64.json) uses the same new public key and signs those exact bytes. The existing [release workflow run 42095863846](https://github.com/taskqorg/agentq/actions/runs/42095863846), macOS job `127485600771`, built it from the merge above with `cargo build --release --target aarch64-apple-darwin`, passed the platform's tests and offline artifact selftest, and uploaded artifact `13447726902` (`macos-aarch64`). This is PR build evidence, not `merge_group` acceptance. It was downloaded and signed on Windows without transferring a private key or executing the macOS binary there.

`ProtectedHost::signed` accepted both staged binaries and refused a zeroed signature for each (`protected signature invalid; authority unknown`). The Windows release artifact's offline selftest also passed locally. The package records sources separately and contains only binaries, public manifests and this evidence; no private key, policy, permit or original receipt is included.

At the source pin, on Windows x86_64:

- `cargo test --locked` passed every target, including 113 AgentQ unit cases and 30 runtime unit cases with 1 ignored. The authority-snapshot case, five protected process scenarios and checked-launcher regression also passed in this full run.
- `cargo run --locked -- selftest` passed offline.
- The contract lint at workflow-pinned specq `7e9adf57287553176d8b3be61c2b22565543dc51` passed with its existing headroom warnings. The local rule cache lacked the script; the pinned GitHub blob was fetched and checked against `29609754df6067ed504621ed4fc11f5a4a6dd995` before execution.
- The four workflow Python suites passed: 9, 11, 2 and 4 cases, with none skipped on this host.
- `cargo test --locked -p agentq-runtime --lib protected_retained_fences` failed before and passed after the cross-action binding fix, as recorded above. Existing assertions and lost-acknowledgment protection remain.

The previous macOS artifact at source `97a9ba59d6f60bfefdab0c6c9e09827ee4a326df`, digest `3485e70404fa96f1e7d23ec63c686311bdeb9a57b82dad6ba2d7bd3b825059a3`, is superseded by the cross-action correction; its earlier test observations do not qualify the corrected source.

No native protected launch or receipt import was performed on a working profile. Native Mac enforcement and lifecycle qualification remain unknown, and Windows native containment is not qualified. Pipe owns its consumer integration and live image qualification in assetq#47.
