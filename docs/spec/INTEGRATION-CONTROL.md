# Integration control

Binding detail of [agentq.md § 4 Temporary controls](../../agentq.md#temporary-controls): `agentq disable` and `agentq enable`. The contract holds the mandatory rules; this file holds lifecycle state, transition and status detail. A change here is a decision change ([spec first](../../agentq.md#principles)). Implementation plan and qualification record: [INTEGRATION-CONTROL-PLAN.md](INTEGRATION-CONTROL-PLAN.md).

## Retired fields

Optimization, routing, document MCP and wrapping hooks are retired ([decision](OPTIMIZATION-RETIRED.md)). Service startup cleans unchanged owned integrations, including old receipts without agent metadata, before export or reconciliation. Hermes owned source follows the record-only release; no activation is added. Saved disabled-profile receipts filter retired fields during scope validation and enable, preserving record hooks, plugin and disable intent.

## Lifecycle state

- `data/integration.json` is bounded, versioned owner-only local lifecycle state under the existing lifecycle lock: generation, host-disable intent, saved service state and selected profiles with supported owned fields, transition stage and the journal metadata needed to recover an interrupted write.
- Record before writing; do not overwrite the first disable baseline on repetition. Resolved rollback metadata is discarded; unresolved transitions are retained.
- Missing state means no disable intent. Never export its paths or values.

## Detach and enable

- Detach evaluates journal history against the simulated file after each receipt. Repeated legacy receipts for one owned registration are idempotent; later foreign values still block the whole affected file before writes.
- Detach reuses scoped journal restore and forgets successfully restored active entries.
- Enable reuses current canonical wants filtered to the saved fields, retaining absent optional components and native settings.
- Enable does not call onboarding or uninstall, replay obsolete registration bytes, or overwrite later foreign values. Retired fields never return.
- Partial writes reconcile by recorded stages and actual readback, preserving conflicts; ambiguous operations are not blindly reapplied.

## Locks, dry run and roots

- Hold the lifecycle lock only for durable intent/configuration/journal writes. Release it before client waits and OS service operations, then revalidate generation before finalizing.
- Dry-run writes nothing and previews selected aliases, changes, conflicts and reload impact.
- Explicit native configuration roots are authoritative; do not discover sibling installations or promise a switch for one worker.

## Running clients

- There is no forced-stop option and no elapsed-time proof. A conservative native process/session inventory must establish that dependencies ended or reloaded. Unknown or unsupported inventory remains pending; no proxy transport remains.
- A repeat finishes after dependencies clear; reload or reopen affected clients to use detached configuration. Never infer verified detach from removed hooks or TCP observations.

## Service during disable

- While a host disable is pending, Langfuse events (heartbeat, resource samples, model calls), revoke retries and operations stay off. After a safe stop no second daemon or scheduler runs them.
- Per-agent disable preserves service for enabled siblings.
- Save service registration/state for enable; honor disable intent after logon, reboot and service restart.
- Service ownership requires the saved unit recipe and the executable/arguments of any live OS registration. An owned file cannot grant authority over a same-name registration loaded by another installation; unreadable registration evidence blocks service mutation.

## Status and verification

- Status separates desired integration, observed configuration, runtime evidence and service health.
- A configured record-hook adapter verifies only after a new SessionStart or Stop newer than its apply watermark; historical hook failures remain visible.
- The newest applied operation follows a private monotonic journal sequence under the lifecycle lock, so same-second writes or a wall-clock rollback cannot select an earlier revision; legacy records without a sequence retain timestamp ordering.
- Each applied readback also captures the latest existing hook timestamp as a verification watermark. Only a strictly newer event can verify it, including after a wall-clock rollback; legacy operations without that watermark stay unverified.
- Intent `disabled` with detached readback is not accidental routing drift in doctor. Local status does not turn a stale heartbeat into running/verified.
- Service status uses its own process, heartbeat and unit observations; an unknown client dependency inventory keeps profile runtime evidence unknown without invalidating independently observed running or intentionally stopped service state.
- Snapshots add only bounded optional lifecycle fields; existing remote mutation envelopes and grants gain no disable/enable value.

## Configuration identity and project overrides

- Configuration revisions cover integration lifecycle intent and the existing owned/native inputs.
- Project overrides block fields editing native client files.
