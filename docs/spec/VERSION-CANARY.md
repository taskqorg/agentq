# Explicit native-version canary

`agentq canary` reads one JSON binding on stdin. It is never invoked by a hook, service, scheduler or default workflow. Only an operator writes `data/canary/opt-in.json`; no command creates or registers approval automatically.

## Authority

The receipt and request must match exactly, with these fields only: `installation`, `host`, `runtime`, `account`, `home`, `model`, `permissions`, `session_policy`, `scenario_revision`, `approved_operation`, `executable`. Identity fields are nonempty strings; home is an existing absolute canonical directory path. `model` is an explicit model code or null (native selection). `permissions` is the complete serialized Runtime Policy, with every field explicit. `session_policy` explicitly selects `lifetime` and `readiness`. `executable` binds an absolute canonical path and SHA-256 of its bytes. Revision is 1 and operation is `native-turn`. Installation and host must match independent local identity, and runtime must be registered.

Missing, malformed, unreadable or changed authority yields unknown before any executable. The receipt is authority for the bound operation only, never login, upgrade, configuration repair or policy changes. The receipt binds requested Launch parameters; it does not prove the native client applied them. Only that exact model, Policy, account home and session policy are requested through Runtime security/preflight guards, without repair, fallback or global configuration edits. The account label identifies the requested home, not observed native login identity.

## Scenario and evidence

Revision 1 is the exact scenario in [VERIFICATION](VERIFICATION.md#explicit-native-version-canary). The native runner requests only the exact approved parameters, preserves authentication/global settings, uses an isolated workspace and session, runs once, and supply independent terminal/events/history/hook-correlation/usage observations. No missing dimension becomes pass. Detected version and scenario result are separate. A failed turn is fail; recognized new shapes are drift; unavailable checks are unknown.

Only fixed allowlisted diagnostic schema keys, typed metadata and known discriminators survive projection. Arbitrary keys and strings become opaque hashes/counts, never copied names or content. Diagnostic fixtures and reports never retain raw diagnostics. Existing Runtime working logs, ownership receipts and optional archive bookkeeping use a private temporary workspace and an explicitly empty temporary installation, removed after confirmed terminal collection; native client-owned history stays under its existing retention. Uncertain live work is never deleted or cancelled merely because observation failed. Secret-pattern scrubbing alone is insufficient.

Reports and local findings contain opaque binding identities, normalized numeric versions, known runtime names, verdicts and projected shapes. Findings live in `data/canary/findings/<identity>.json`, keyed by host/runtime/version/scenario, with explicit `open` state. Repeated failures replace the same finding under the existing lifecycle lock and atomic private-state writer. Findings never change native work or exit codes.

## Qualification and delivery gates

Local evidence is not board delivery. Issue 298 requires one deduplicated board finding; delivery remains a separate operator-approved operation through an approved board API. Existing incident policy permits update and queue-overflow faults only; canaries do not silently extend that catalog or submit network messages.

Actual collection uses one Runtime launch, poll, report, typed records and canonical native history. Applied model, permissions, session policy and account are separate observed conditions; missing native proof remains unknown and observed mismatch fails. Requested equality is never observed proof. Version and executable bytes are revalidated before submission; changed or unavailable evidence refuses rather than substituting a client. Synthetic observations qualify source dataflow only, never native capability. Correlated hook proof requires the exact newly launched session record, absent before the run, from the existing SessionStart writer; timestamps or configured hooks alone do not qualify it. Runtimes without this evidence remain unknown. No native/paid execution or host opt-in is authorized by source qualification.

A protected receipt is a regular nonsymlink file owned by the effective local user, with no group/other permissions. Missing platform proof refuses. The workspace uses the existing Runtime wait cadence; no new timeout, retry or budget is added.

Version observations use `Runtime::version_for` with the already constructed exact Launch, including its approved executable, account home and isolated working directory. The same runtime environment cleaner removes inherited session/auth context as for the native turn. Authority and executable identity are checked before each explicit version observation; missing runtime support remains unknown without a bare-command fallback.

Known session metadata includes optional `cwd` and `forked_from_id` schema names. Their path/native-ID values are never diagnostic output, fixture evidence or paths to open; genuine unknown fields/types still produce hashed drift evidence.
