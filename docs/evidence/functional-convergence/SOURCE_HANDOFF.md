# Initial functional source join

This is an implementation handoff, not acceptance or verification evidence. No build, Cargo, test, check, lint, selftest, formatter, probe, hash-verifier, independent review, native or Windows execution was run for this join. Earlier author reports remain historical evidence for their own revisions only.

## Exact provenance

The actual base is `d3b0513653c788ad2f9a06ce91b50c283080a3db`: retired runtime core `71a32aa`, Windows locator join `a96004c`, neutral AssetQ join `68adde6` and the native-scope P1 correction `d3b0513`.

The following complete cumulative source differences were applied from `f0d129fc223c3db42a7273a68385b22f7a0e9140`, without importing their parent histories wholesale:

- Owned background stop: `1ed1ca8699b781194a422d32d473e4d0cca527ba`, including `d786506` protected stop, `aad2f6c` previous-incarnation refusal and `1ed1ca8` flat ownership history. Author handoff: `/tmp/agentq-owned-background-control-flat-1ed1ca8/SOURCE_HANDOFF.md`.
- Durable hook spool: `51a1ae2a93a38d9c8c92a6c5c5d0df11cec4989a`. Author report: `/tmp/agentq297-frozen-d3d564411ea8/report.md`.
- Explicit version canary: `dece8c5a7d1212f5e66f803fc59cdf4fe3f1055f`, including `ae78c84` authority/evidence entry, `410c510` Runtime collection and `dece8c5` version-context/schema corrections. Frozen author reports were copied unchanged into `docs/evidence/canary-298/`.

No diagnostic `978`/`452` source, old raw fallback, compatibility shim, Intent API or consumer opt-out was imported. Original refs and frozen author artifacts were not rewritten.

## Manual functional joins

- `runtime/src/agent.rs` exports both the existing `InputStage`, `LaunchError`, `Binding`, `Image` and the added `BackgroundExecution`, frozen hook builders and public physical identity helper. The private diagnostic exclusion fixture and launcher attribution remain.
- `agentq/main.rs` retains both CLI entries, both modules, hook-spool doctor/service fields and canary offline selftest registration.
- Runtime trait/Adapter/history-only Reader carry the owned-stop operation alongside the base typed launch/resume errors. History-only execution still refuses. The canary's scoped `version_for` default remains nonexecuting; Adapter uses the shared preflight command boundary.
- Native schema decoding was moved from the canary consumer into `runtime/src/agent/schema.rs`, exposed through `Runtime::record_shape` as typed `RecordShape`. The original known-name list, recursive unknown-key/type hashes, numeric counts and drift handling remain; unsupported readers refuse. Canary receives opaque raw record strings only to return them to Runtime, never parses their native JSON, and persists only projected diagnostics. Existing schema/privacy fixtures now call this Runtime operation.
- `Report` and `Launch` use the actual base APIs without reverting typed start failures, images, no-Intent retirement, archive-install isolation, owner fences or scope-conflict refusal. Three-way source joins retain the base security/environment code and additive operations.
- The full owned source includes secure source traversal, source/prefix/alias proof, current receipt/controller/native identity fences and linear journal reconstruction. The spool includes private durable batches, protected registration, current revoke authority, checkpoint ordering, service drain and doctor projection. Source fixtures and callers accompany each delta; none was executed.

## Finite pending functional deltas

Central must provide frozen exact source for these existing producers; this integrator must not duplicate them:

1. Claude history facts: integrated exact `29b48cce03925519a2a603a52835dcbae9a01cb8` against `71a32aa28e82134278347c2e15bb3fb170346332`; author report `/tmp/agentq-claude-source-facts-29b48cc/report.md`.
2. Launcher: integrated exact `d3b0513653c788ad2f9a06ce91b50c283080a3db` to `c233839eb97cb6a03ef3b2273111349152e23d61`; author report `/tmp/agentq-launcher-observation-source-c233839/report.md`.
3. Claude fork: `ctx_a9231c3bac43`.
4. Lineage: integrated exact cumulative `68adde6809829b56ac2a715de746424b97f42810` to `6445b86f407b784e7a9c563ffd69749d6a4d78fc`; author report `/tmp/agentq-300-a2-source-report.md`.

Partial Claude fork source `acd030ff8f35d4ac3585468db8f2a262ecbfb4f3` on `68adde6` remains retained on its original ref for a later source join, not imported in this continuation. Author report: `/tmp/agentq-300-claude-native-source-implementation.md`. It does not implement a successful native fork; capability remains undeclared and `fork-unsupported` remains required.
5. After those functional joins, central supplies exactly one source to the TaskQ cold caller for adoption.

No unresolved source conflict is intentionally left in this initial join. This does not establish GLOBAL functional completion: Claude fork completion, the pending locator correction and actual TaskQ/DashQ caller/pin convergence remain pending. Independent review and the single coordinated compile/test batch remain deferred until central GLOBAL functional completion. Native, Windows, service, deployment and external delivery acceptance remain separate and unknown.

## Claude source-facts continuation

The exact 29b48cc delta includes typed record-local `presentation.session_kind`, the explicit caller-witnessed `history_with_native_session_alias` resolver, bounded physical/native copy witness, read/recovery/artifact/resume guards, registered identity conflict refusal, contracts and all five regression scenarios. TaskQ `ctx_4cda7595f7aa` owns consumer migration; no TaskQ files changed here.

The additive join needs one semantic adjustment in owned-stop proof: a witnessed copied handle retains logical `Source.session = destination` and observed `Source.native_session = original`. Control binds the logical destination and requires the supplied original to equal that retained native identity; it does not rewrite either identity. The existing source/prefix/alias/controller/process checks remain. Control also validates the copied witness, and the old mutating alias method refuses an already witnessed handle rather than invalidating its binding. Old non-witnessed alias semantics stay unchanged.

This continuation ran no verification commands. The initial join and its source archive remain preserved; the continuation gets a distinct source commit and archive. Native outcomes and regression outcomes remain unknown.

## Launcher observation continuation

The exact d3-to-c233839 delta is integrated onto `bce7d38`. It includes exported read-only `BackgroundLaunchExchange`, its Runtime default/Claude implementation, existing native decoder reuse for unverified lookup hints (including failed launcher replies), fresh owned resumed Codex CLI source/positive-exit-qualified `Report.start_failure`, canonical contracts and regression source. The sole textual conflict was the agent export list; the join retains both `BackgroundLaunchExchange` and all existing `BackgroundExecution`, typed errors, binding, image and frozen-hook exports. No source parser or stage constructor was added to a consumer.

Historical launcher bytes retain an explicit unknown and never grant BeforeInput, accepted input, owner, session/process or control authority. The new plain CLI proof remains qualified by the actual owned original source, requested resumed thread, positive nonzero observed exit, exact native grammar and absent cancellation/timeout/unknown exit. Cold/recovered/copied text does not gain this authority. Existing optional native exit code remains optional; no synthetic -1 conversion is introduced.

Actual background positive correlation remains missing and explicitly incomplete: the launcher exchange has no correlated native child/session/source witness, and a provisional returned job token or refusal text cannot supply one. This source does not populate background start_failure from those bytes and does not claim no-start, replay safety, capability completion or acceptance. TaskQ owns final caller migration. All added regression source remains uncompiled and unexecuted; no intermediate verification or review was run.

## Portable lineage continuation

The complete 68-to-6445b86 source delta is integrated onto `0de460d`, including initial `69adf8d` API and the measured selected-frame bridge. The join includes the closed typed decoder, mandatory nullable values, duplicate/extra/framing rejection, immutable operation/revision chain, generated decoder identity source hook, grant-checked auxiliary spool/refinement, durable physical/queued/accepted witnesses, selected bridge, owner-proof-first optional publication and contracts/fixtures. No fingerprint generation ran.

The only textual conflict was `registry/codex/app.rs::selected`: the coherent P1 `scope_consistent` check already contains and strengthens all incoming inline scope checks. It remains the single selector guard, followed by the existing registered history-session affirmation. The selected SessionLine now additionally carries the actual frame's private journal witness. This preserves conflicting thread/turn/image/terminal refusal while retaining the complete early-frame origin bridge; no raw fallback or weaker scope check replaces it.

All App constructors, NativeFrame constructors, SessionLine construction and selected archive writer call sites use the cumulative source shapes. The bridge retains immutable selected caller runtime/format/source/binding/nullable-native identity, requires all selected contributors in one physical frame, and anchors projection only through accepted scrubbed coverage. Requested versus acknowledged conditions, unknown/null/pending outcomes and native authority remain distinct. The wire retains its 15 keys; auxiliary uploads supply no native session/turn/media/control authority.

DashQ sink `83eb2ea62c88da1caf8ae0121289be6d73733741` uses the unchanged public lineage API from `69adf8d39c68d036958cae936891b5207ebc0f53`; report `/tmp/dashq-lineage-source/report.md`. No sink code or intermediate dependency pin changed here. Its decoder equality must be evaluated using the final compiled runtime, not a digest supplied before the authorized build.

Remaining finite source gaps: locator correction `ctx_e1b3c0850502` is still being implemented; partial Claude fork `acd030ff8f35d4ac3585468db8f2a262ecbfb4f3` remains unintegrated and functionally incomplete; affirmative background positive correlation is absent in the existing backend; central still owns final TaskQ cold-caller/API alignment and DashQ final runtime pin. No global completion, acceptance or native success is claimed. All lineage source fixtures and existing guard fixtures remain unexecuted, and all earlier immutable archives/refs remain preserved.

## Resumed background locator continuation

The exact `0de460d792e2c0de01cbb44c953b9b668c93ab41` to `49580ae2ccba024927284d646ea19d6cf314a9e3` delta is integrated onto `f48170539e57f9bf12ff02a5631ff5dee5a05cf4`. Author report: `/tmp/agentq-resumed-background-source-49580ae/report.md`. All four changed files (contract, Adapter, NativeTurn and synthetic regression source) joined without textual conflict. Lineage journal witnesses, selected-frame source binding and the P1 scope_consistent guard remain in the coherent source.

`BackgroundLocator.process: Option<NativeProcess>` carries the producer-pinned incarnation. A successful original exchange token, unique validated original-token job, requested scope, unchanged physical source and original live process are required for resumed cache/access. Metadata-only fresh observations can retain process None; they grant no import/control authority. Recovered labels restore neither the launch token nor process witness. NativeTurn::bound_file reuses existing physical identity/prefix checks without moving the cursor. No synthetic exit code or consumer native parser was introduced.

Locator correction source is now integrated; TaskQ owns final caller SOURCE alignment against this continuation. Remaining source gaps are mandatory Claude fork completion under `ctx_8d4de7e72397`, affirmative background BeforeInput correlation absent from the existing exchange backend, and central final TaskQ/DashQ caller/pin convergence. The partial `acd030f` source remains preserved and unintegrated; no fork capability or successful child admission is claimed. All regression outcomes remain unknown; no builds, tests, checks, lint, probes, hash generation/verification or review ran.

## Claude partial/backend continuation

Actual ancestry delta `f48170539e57f9bf12ff02a5631ff5dee5a05cf4` to `a8c696ff4ca4cb40f461d480da1b8e3c697b8bf7` is integrated onto coherent `4e4034418c5da122275a72b2e882fc5921571f51`. The author branch had already merged f481705; no older whole runtime was reimported. Six functional source/contract/fixture files joined without textual conflict. The locator49580ae `BackgroundLocator.process`, original exchange/process/source pins and recovered-label refusal remain intact alongside lineage bridge and P1 scope guards. Author report: `/tmp/agentq-300-claude-convergence-source-handoff.md`.

This imports only actual partial functionality: original native physical generation/whole-turn/UUID-chain witness, separated existing controlled boot/input transport and concrete private PreparedFork with measured selected Step, physical source/incarnation, parent/owner and requested_conditions. Preparation neither creates a child nor publishes applied lineage. Claude fork capability remains absent and public fork-unsupported remains required. `docs/spec/CLAUDE-FORK-BACKEND.md` records the unavailable legitimate backend edit/build boundary and required causal native startup hold, authoritative correlated initialize receipt and atomic first-input consumption on the existing transport.

### Current unresolved source list

1. Mandatory external Claude native startup hold, actual child/loaded-prefix/effective-condition receipt and atomic first typed input are unavailable in a legitimate editable backend tree or supported release. AgentQ cannot manufacture these guarantees through a parser, installed-binary patch, alternate transport or speculative field.
2. After that backend exists, AgentQ still needs real startup/receipt parsing, normal child preflight/process/lifetime launch, durable parent/child admission, actual distinct child binding, full retained-prefix/effective-condition comparison, atomic input, unknown-fence reconciliation and verified finish/publication. Current private PreparedFork is not that implementation.
3. Claude native receipt-specific archive publication and an actual accepted JSONL-selection/stream physical capture bridge remain absent; no Codex thread.id shim is permitted. Tool/carrier/attachment/compaction mapping and Windows alias mapping remain explicit gaps.
4. Affirmative background BeforeInput correlation remains absent from the existing launcher exchange; locator observation is not rejection proof and original input/occupancy remain uncertain without replay.
5. Consumer snapshots TaskQ `1cf62612`, DashQ `83eb2ea` and AssetQ `473789` remain unverified. Central owns final immutable source/pin/caller convergence and later authorized qualification.

Global functionality is NOT complete while the mandatory external Claude prerequisite and dependent AgentQ implementation remain absent. This continuation is successful source integration of partial work only, not successful Claude fork delivery. No tests/build/check/lint/probe/hash-verifier/review or native execution ran; no prior source archive/ref/evidence was rewritten.

## Reachable Claude adapter continuation

The exact cumulative `a8c696ff4ca4cb40f461d480da1b8e3c697b8bf7` to `6933fc5dc1ad4f00fcee8aa54b4c9990e95e0ec2` delta (5d57/999131/cd8e/6933) is integrated onto `182b4d29004d62f662f89ca0557c6e53892c39a0`. Nine contract/implementation/fixture files joined without textual conflict. Author reports: `/tmp/agentq-300-claude-reachable-fork-source.md` and `/tmp/agentq-300-claude-child-lifecycle-source.md`. Locator49580ae, typed errors/history/no-Intent, lineage selected journal and P1 scope_consistent remain present.

This continuation supersedes earlier handoff statements that a blanket external startup hold is mandatory or Claude capability must remain absent. The corrected contract scopes the extra no-additional-turn clause to Codex. Claude now declares an actual owned adapter path using ordinary inspected native fork argv, normal preflight, requested-ID ownership, process/lifetime hooks, durable parent/child lineage and initialize before explicit input. Descriptor path availability is not evidence of native success.

The exact expected child path must be absent before spawn. The same owned process must supply matching initialize PID/incarnation and actual newly persisted child conversation identity/prefix. Actual History generation, original source generation, native child identity and independent settings/rules must agree before binding/finish and exactly one explicit input. Unexpected native work, missing/foreign/partial/changed persistence or failed readback retain unknown fences and submit no caller prompt. No transcript copying, new vendor flag, fake event/thread.id, backend hold token or alternate provider is introduced. Recovery uses ordinary same-ID scope rather than replaying fork argv. The private archive hook accepts actual Claude persistence/PID/settings evidence and retains grant/capture-based unmapped/pending outcomes.

### Remaining independent scope after this continuation

- Actual Claude2.1.284 may not materialize pre-input JSONL; this source path exists but native success and persistence timing remain unknown. No successful fork functionality proof is claimed.
- The finite source implementation still refuses unsupported original tool/carrier/attachment/compaction/side-chain and Windows alias domains; their mapping remains independent functional scope if required by full #300 coverage. No accepted physical Claude JSONL-selection/stream bridge is fabricated; absent mapping remains unmapped/pending.
- #301 legacy removal remains an independent unfinished source scope; this Claude delta does not remove legacy model/hook/export/lifecycle callers. No exact #301 final removal delta was supplied or implemented in this dispatch, and the task is not declared complete by runtime convergence.
- Affirmative background BeforeInput correlation remains unavailable in the existing exchange backend; observation tokens/process locators do not authorize no-start/replay.
- Central still owns final TaskQ/DashQ/AssetQ immutable source/pin/caller alignment; their named snapshots remain unverified. This source freeze does not declare GLOBAL functional completion.

No builds, checks, tests, lint, probes, hash generation/verifiers, review or native execution ran. The nine-file cumulative delta and regression SOURCE are uncompiled/unexecuted; prior refs, archives and historical reports remain unchanged.
