# Portable session lineage and conditions

**F300-L20261107-2 — unacknowledged proposal.** This is the one revised field artifact for #300; it replaces only the incompatible history/archive field proposal in F300-20261107-1. Preserve its original commit `1af9b67e3081b02c5b555193a413cd48f1a6494e` / blob `20f490ead4055c94490741a976659e17ba511864` and central technical ACK. Runtime `fork`/`rewind` signatures, selection/owner/fence semantics, C285 types and final coherent #285 prerequisite remain unchanged. No new field set has central ACK; no source, producer, sink or upload implementation is authorized by this document.

Exact inputs: prepared producer `8076839b6578916088f537d541e8e7cb36025df1`, history/media `3779faabf6a3a4950e9477c59d45bbcadfcf9eb0`, SourceSpec readers `5fd7f27d16bc54107593a5da811d6d419dc99dc1`, DashQ `4ca6e1c4252d301c0e743b9d990f12849ed1da94`. Existing history/archive owners supplied facts, not competing types. This proposal does not qualify those held sources or a different-model native fork.

## Keep the existing range/manifest wire

Use the current `POST /machine/archive/1`, protocol `1`, schema `agent-history/1`, unchanged grant/owner-only reads, capture mode, receipts and append-only contiguous byte ranges. **Keep exactly the existing 15 manifest keys.** Do not add `parent_step`, `parent_revision`, `changed_conditions` or hide an extension in `policy`.

The proposed addition is a producer-owned UTF-8 JSONL **content source** with `format: "agentq-lineage/1"`, a fresh durable opaque `source` UUID and ordinary scrubbed `bytes`/range digests. It contains read-only lineage observations, not native transcript records, a control transport, reply tokens or mutation authority. No `media` or `turns` binding is permitted for this auxiliary source. It creates no `agent.turn`, model call, usage, completed-turn claim or native-byte activation. No new route, manifest key, grant, configuration or numeric operational limit is needed.

Its enclosing 15-key manifest identifies the result session/runtime and existing project/capture attribution; condition fields are null, because metadata-source activation is not native-condition observation. For a fork, existing `manifest.parent_session` may name its confirmed source session. For an in-place rewind, preserve the session's original native ancestry; never replace it with self-parent ancestry. New-format readers treat this format as auxiliary: its manifest activations must not become candidates for native-turn comparison, including the existing single-manifest shortcut. Old readers may retain/raw-read it but cannot claim lineage/conditions support. This is not a fallback or a claim that the new reader semantics already exist.

## One closed content record

Each complete line has exactly these keys. The typed grammar below is normative; aliases and additional keys refuse normalization. Ordinary raw storage remains authoritative when a line cannot normalize.

```text
Lineage = {type:"agentq_lineage", version:1,
 operation_id:OpaqueId, revision:u64, kind:"fork"|"rewind", effect:"applied"|"noop",
 session:ArchiveSession, parent:{session:ArchiveSession, point:Point},
 predecessor:SourceRef|null, activation:Activation, conditions:Conditions}
SourceRef = {session:ArchiveSession, source:OpaqueId}
Point = {kind:"empty"}
      | {kind:"unmapped", reason:"not-archived"|"mapping-unavailable"|"source-domain-unsupported"}
      | {kind:"record", source:OpaqueId, start:Offset, end:Offset, sha256:ScrubbedDigest,
         projection:{decoder:OpaqueId, part:u64}|null}
Activation = {kind:"pending", reason:"no-native-bytes"|"mapping-unavailable"}
           | {kind:"byte", source:OpaqueId, start:Offset, manifest_sha256:ManifestDigest}
Conditions = {model:Delta<Text>, effort:Delta<Text>, policy:Delta<PortablePolicy>,
 account_label:Delta<Text>, cli_version:Delta<Text>, instructions_hash:Delta<ConditionDigest>}
Delta<T> = {requested:Pair<T>, observed:Pair<T>}
Pair<T> = {before:T|null, after:T|null, changed:bool|null}
```

The following invented record shows an empty-prefix fork with requested model change and no observed child model/native-byte activation. It contains no native source bytes or mutation authority:

```json
{
  "type": "agentq_lineage", "version": 1,
  "operation_id": "fixture-operation", "revision": 0,
  "kind": "fork", "effect": "applied", "session": "fixture-child",
  "parent": {"session": "fixture-parent", "point": {"kind": "empty"}},
  "predecessor": null,
  "activation": {"kind": "pending", "reason": "no-native-bytes"},
  "conditions": {
    "model": {"requested": {"before": "model-a", "after": "model-b", "changed": true}, "observed": {"before": "model-a", "after": null, "changed": null}},
    "effort": {"requested": {"before": null, "after": null, "changed": null}, "observed": {"before": null, "after": null, "changed": null}},
    "policy": {"requested": {"before": null, "after": null, "changed": null}, "observed": {"before": null, "after": null, "changed": null}},
    "account_label": {"requested": {"before": null, "after": null, "changed": null}, "observed": {"before": null, "after": null, "changed": null}},
    "cli_version": {"requested": {"before": null, "after": null, "changed": null}, "observed": {"before": null, "after": null, "changed": null}},
    "instructions_hash": {"requested": {"before": null, "after": null, "changed": null}, "observed": {"before": null, "after": null, "changed": null}}
  }
}
```

`OpaqueId` is a nonempty control-free producer identifier, never a path or source-derived secret. `ArchiveSession` is the exact already-authorized archive manifest session label; it is not a local physical source ID. Workspace/installation come solely from the authenticated grant/read scope and are not selectable in a record. All referenced sessions/sources are in that same workspace/installation. If no existing safe session label can be bound without exporting a host path, keep lineage private/unmapped; do not invent a path-hash session alias. Runtime opaque-ID acceptance is unchanged. `Offset` is the existing wire nonnegative integer domain; digests are existing lowercase SHA-256 encodings of their named domain. `revision` starts at zero and increases by one using checked arithmetic within one operation; overflow refuses a new observation. It is not `Step.revision` or a native generation. `decoder` is an immutable history decoder source revision agreed by producer and reader, not an executable, download location or configuration knob. Do not execute/load an older decoder on demand.

A record may assert `effect:"applied"` only after native application/result identity is known. `effect:"noop"` is permitted only for a read-verified rewind of the already-current prefix. A fork always has `effect:"applied"`, distinct `session`/`parent.session`, and null `predecessor`; a rewind has equal session labels. A rewind `predecessor` names its exact old primary archive source generation when mapped, otherwise null/unknown, never an inferred source with matching bytes. `Point.empty` is a proven empty retained prefix, not missing selection evidence. Unknown native delivery/identity remains in the private durable operation fence; it is not published as applied lineage under a guessed child/session.

A record anchor requires `start < end`, one complete JSONL physical record (including its framing newline), and a non-media, non-auxiliary source in `parent.session`. A nonnull rewind predecessor must belong to `Lineage.session`; unknown predecessor is never absence proof. The enclosing manifest session must equal `Lineage.session`. Schema, privacy and comparison validation precede projection. An invalid complete line is explicit unknown with raw content retained; it cannot grant authority, mutate prior observations or suppress other sources. Native unsupported/refused operations do not become successful lineage records.

## Three separate identity domains

| Domain | Authority and retained facts |
|---|---|
| Private runtime selection | Mandatory owner/run scope, re-resolved native turn/chain cursor, local `Step.id`, original source incarnation, origins and native hashes/paths; all remain private. `Step.state=completed` is item completion, not whole-turn proof; `Turn.native_id` may be null. Native adapter re-resolution/refusal remains mandatory. |
| Private capture frame | Binds that selected native incarnation/physical part to one archive generation/segment UUID, its native base/end and queued/accepted scrubbed bytes. Snapshot generation, row/part/pointer/encoding are distinct from JSONL offsets. Persist this mapping independently of optional delivery. No original source hash/path is exported as content identity. |
| Portable archive point | Authenticated namespace + `parent.session` + immutable archive `source` + scrubbed `[start,end)` + hash of exactly those decoded archived bytes. Optional projection refers to an archive-side decoder part, not an assumed local `Step.id`, native call ID or universal revision. |

Preserve `Source.parent` as native ancestry and `Source.parent_step` as its existing native call/item ID. Resolved history parent may be an opaque physical source ID. Do not relabel either as a stable archive step. Local `Step.id` includes source label/session/native ID; anonymous IDs also depend on offset/raw hash. Sink reconstruction uses the opaque archive source label and scrubbed bytes. Local and sink IDs generally differ. This wire does not export local Step IDs; keeping them as private provenance supplies no sink identity equality claim.

For a physical native JSONL record `[n0,n1)` fully retained inside a capture segment whose private native base is `b`, length-preserving scrubbing permits archive `[n0-b,n1-b)`. The frame must prove same incarnation/part, complete record framing and no cut boundary. Hash the exact durable queued/accepted scrubbed bytes, not native bytes, a re-scrubbed newer prefix or a whole-file prefix digest. All anchor bytes must already have durable accepted archive coverage; a record may cross upload chunks, so no single chunk hash substitutes for its record hash. No parent upload authority is inferred from having child authority. Excluded, unsent, ungranted or unavailable parent bytes yield unmapped/pending linkage, never a hash of content that was not sent.

A capture `source` UUID is the portable generation/segment identity. Replacement, truncation, rewrite, changed format/incarnation or terminal-attribution segmentation gets a new UUID. `Source.start` is a private segment base, not an outgoing offset; each new UUID begins at archive zero. Even byte-identical copied prefixes in two generations have different scoped points. A bare local revision or archive offset cannot select another generation. Media whole-object digests, compressed/encrypted object positions and manifest digests are not native-record anchors.

SourceSpec snapshot revisions are `snapshot_revision + row index`; generation lives on `StepPage`. Their origins can identify a file-part digest, JSON pointer, projected SQL row or encoding rather than original JSONL bytes. Do not apply the subtraction formula to these domains. Until a format-specific physical-to-queued mapping is proven, use `Point.unmapped(source-domain-unsupported)` or a record anchor with null projection if only bytes are proven. Current producer8076839 refuses raw database authority; this proposal adds no database export or normalized-row substitute.

## Resolve a parent step without equating IDs

A record anchor is portable byte identity even when its step projection is unknown. A nonnull `projection` requires a separately proven mapping witness for the selected local physical contributions to the scrubbed archive projection. Re-projecting under a new label, matching text, copying `Step.part` or observing identical display output is insufficient. Validate original selection privately; prove the capture frame; prove record/block structure and identity-bearing fields survive scrubbing or have an explicit decoder mapping; and compare the mapped contribution set. Scrubbed identity collisions, omitted origins or multiple candidates leave projection null. No native ID/hash must be exported to repair that gap.

Producer and sink agree the decoder revision and test its deterministic archive projection. The exported `part` is that **archive projection's** part, derived after the mapping witness, not a blind copy of the local decoder index. The reader verifies complete accepted coverage/digest, re-projects the archived prefix through `end` with archive source/session labels, and matches exactly one step whose latest physical `source` range/hash and part equal the anchor. Use the prefix as of the chosen boundary, not an arbitrarily later coalesced revision. `Step.source` is latest origin; `Step.origins` is the whole contributing set and must be available for the witnessed mapping.

A verified match yields the sink's own `(session, source, Step.id)` link. It does not rename the local Step, alter native ancestry or authorize a cursor/reply. Missing decoder agreement, zero/multiple matches, non-JSONL origin, missing parent, deletion, expiry, unavailable audit/content or changed source stays explicitly unresolved; the record anchor itself is immutable. Existing guarded/audited raw/step readers govern every parent read. Do not bypass reader authority via a child link or fetch a foreign installation/path/URL.

## Typed requested versus observed conditions

The six condition names above are mandatory. `Text` is an approved nonempty control-free condition string. `ConditionDigest` is only an already-authorized manifest-condition SHA-256 value; do not hash/read an unsent instruction file to populate it or reuse it as archive content identity. No instruction text, email/token, environment value or host path belongs in these cells.

`PortablePolicy` is the complete path-free abstract `Policy` value with exactly `read`, `write`, `network`, `tools`, `approval`, `roots`, `unelevated`, `builtin`, `isolated`, `subscription`. Preserve existing lowercase enum values; `roots` must be `"native"`. `tools` is null or a list of nonempty identifier labels using ASCII letters/digits/underscore/dot/hyphen, not paths/URLs; `unelevated`, `isolated` and `subscription` are booleans. If the exact policy has `Roots::Exact/Add`, unsafe tool labels, unsupported fields or cannot be fully observed, the whole policy cell is null, not an incomplete summary pretending to be the full policy. This is a disclosure/typing rule, not a change to runtime permissions or allowed native tools.

Requested-before/after are the exact captured caller intent for the selected parent conditions and new child/future-head conditions. An omitted `Launch.model`/effort/instructions value is null, never an inherited observed default. Observed-before is native evidence scoped to the selected retained parent point; observed-after is evidence for the new child's first applicable native turn/rewound head. A fork acknowledgement that reports an effective field can supply that field; a requested setting, latest unrelated parent result, account label guess or mixed legacy manifest cannot. A no-input rewind preserves requested intent where known but does not invent a new native model observation.

`changed` is nullable and computed separately for requested and observed pairs: it is null if either value is null/unverifiable/withheld; otherwise it equals inequality of canonical typed JSON values (existing canonical object sorting, array order retained). It is not requested-versus-observed comparison and not proof that the native client honored the request. Known equal requested values can coexist with unknown or different observed values. Readers reject an inconsistent definite comparison instead of trusting an arbitrary boolean.

Sanitize each condition before serialization with existing archive disclosure/scrub rules. If scrubbing or path/secret withholding changes a value, represent that value and its comparison as null; two redacted strings cannot prove native equality. A final byte scrub that alters typed structural fields/IDs makes normalization unknown, not a license to change anchors or comparisons. Original values and hashes needed for private selection stay private. Legacy 15-key manifests mix requested Launch values with later observations in producer8076839; their values must not be promoted into either typed basis without actual scoped evidence.

## Append-only activation, replay and no-byte cases

Required local operation intent/selection and native result/uncertainty are durably recorded under existing owner fences independently of archive authority, queue, storage, network or receipts. Persist private mapping and export intent without waiting for the sink. The mandatory local native record remains complete even when no capture grant/frame exists; archive mapping may remain unknown and never requires capturing excluded parent content. Optional delivery failure never changes native success, grants replay of a mutation or drops its fence. No sidecar is captured/sent without the existing archive grant; manual capture still needs its separate capability.

Sidecar lines append to their own durable source and use ordinary idempotent range receipts/checkpoints. A later observation may append a higher `revision` for the same `operation_id`, filling previously unknown mapping/observed values after exact scoped evidence arrives. Kind/effect/result session/parent session, requested intent and any already-known point/anchor/activation cannot change. An equal revision must be byte/canonical-content identical; conflicting duplicates or attempts to rewrite known facts make that operation's lineage unknown, without deleting raw evidence. Revision zero establishes the base; a new refinement must be the next checked revision. Missing/expired prior revisions make consistency incomplete, not proven. Stale exact lower revisions do not override a higher one. A null projection may be filled only for the same immutable record anchor; no already-known byte identity changes. Do not use timestamps or arrival order to select facts. A later different native head is a new operation, not a refinement.

`Activation.byte` must reference an actually accepted **native** source byte and the exact manifest then in force for this operation's first affected native segment. Require the producer's private segment/operation binding and existing durable `turns` receipt when attributing an `agent.turn`; do not infer a first byte from model, time or copied history. Sink lookup uses the greatest native manifest activation at or before that byte. A missing/conflicting receipt stays unknown. A sidecar line's own upload activation is never this native activation and never counts as work.

Accepted range/end/digest/format/manifest hashes are immutable. A new condition manifest activates only on a new native contiguous range beginning at its first affected byte. Exact old-range replay requires the old manifest hash; it may add a permitted missing turn binding but cannot activate this record's conditions or change any accepted binding. Late terminal attribution uses the producer's separate source/segment with predecessor mapping; it does not rewrite an earlier accepted live manifest. The sidecar retains its own append-only metadata manifest activations independently.

| Case | Required local/portable behavior |
|---|---|
| Empty-prefix fork | Prove native creation/child identity locally; `Point.empty` and a real nonempty sidecar line can archive lineage under the child session even with no native bytes. `Activation.pending(no-native-bytes)`, no fake zero-length native range, turn or condition activation. |
| No-input rewind | Record known application and exact old source predecessor privately; sidecar carries source/point mapping if available. Native rewrite gets a fresh generation UUID when captured; no new native bytes means pending activation. Old accepted parent/source bytes and manifest hashes remain unchanged. |
| Empty source/current-prefix noop | Only read-verified rewind may say noop; no replacement/generation or model call is invented. A predecessor may be null when no native source was ever archived. |
| Parent/child not captured, excluded or offline | Local lineage survives. Unmapped point/pending activation or not-yet-delivered sidecar is explicit incomplete archive evidence; native execution never waits for it. Late accepted bytes may refine unknown linkage append-only. |
| Rewind/copied prefix | Link exact before/after generations/segments when proven; identical prefix bytes never identify a generation or create billable work. Only existing exact durable agent.turn observations can contribute usage. |

A metadata-only archived session must remain visibly without native content/turn proof; sidecar receipt success is not fork/native/model qualification. No byte-range or manifest-only mutation endpoint is added. No current native/history/source fields are silently repurposed.

## Required future changes and focused contract proof

The existing ingest manifest validator stays exactly 15 keys. Current range/receipt/gap/replay/media invariants stay unchanged. The actual required schema extension is one read-only `agentq-lineage/1` **content-record decoder/validator** and cross-source resolver, plus auxiliary-source exclusion from native-condition comparison. Validate the closed record/types/scopes/comparison/refinement rules at producer construction and guarded reader projection. This new decoder is not present in prepared producer or current sink, so byte-envelope compatibility alone is not semantic support.

After exact central field ACK: runtime privately records native lineage/mapping under unchanged APIs; history owner supplies witnessed physical/archive projection and keeps existing parent fields; archive owner adds the grant-checked auxiliary source constructor/spool/refinements without bypassing native-source validation; AgentQ history registers the read-only format/Link projection; DashQ pins that decoder, resolves links under existing audit/role/deletion checks and excludes auxiliary activations from turn comparison. No native name/parser branch or arbitrary policy JSON tunnel is added to a consumer. No new ingest route, manifest keys, sink table/index is required for correctness: raw sidecar ranges remain source of truth and projection is derived on guarded reads. Any later index is outside this proposal.

Focused no-model contract cases: unchanged exact sink manifest validator accepts its 15-key object and rejects each old proposed extra key; sidecar stays inside the same range envelope; native-to-segment offset and cross-chunk scrubbed-record hash match accepted bytes while original hash/paths never export; two source generations with identical bytes remain distinct; local/sink labels and scrubbed anonymous IDs differ without being equated; failed mapping/collision/snapshot row stays unresolved; prefix-boundary projection uses the agreed decoder and exactly one part; requested/observed nulls, typed policy disclosure, inconsistent booleans and redacted-equality cases; exact replay versus new manifest activation; append-only refinements/conflicts; empty/no-byte/noop operations; auxiliary source never supplies native activation, agent.turn, usage or permission; parent reads fail closed on role/revocation/expiry/deletion.

These are contract/fixture proofs, not implementation or upload proof. Source-specific witnessed mapping, compiled decoder equality, full guarded producer/sink replay/read tests and native different-model forks remain later prerequisites. No Runtime/C285 signature, production/history/archive/sink source, content upload, native/model/paid action, full suite, push/PR/merge/tag/deployment or live change occurs in this preparation.
