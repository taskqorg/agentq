# Inherited CoreQ context: task 260

The protected-child follow-up completes [task 260 A2](https://github.com/taskqorg/agentq/issues/260) under agentq.md M12.13. It builds on retained candidate `eac48ee2f64c8faced241f3ad81a5ede92b0a097`; the original result and crashed review remain historical evidence, not a passed review of this follow-up.

## Defect and correction

The protected host omitted `COREQ_TRACE_CONTEXT` from its launchd job environment and cleared it again before starting the native child. Ordinary runtime paths already carried the caller context.

The job and native launch now share their clean environment builder. CoreQ's bounded `TraceContext` decoder validates the inherited value before re-encoding it. A current context takes precedence. Missing or invalid values are omitted; arbitrary caller environment is excluded. The existing plist serializer and its escaping are reused.

The change adds no authority: permit authentication, preparation, native identity, input gates, descendant completion and retained recovery keep their existing checks (M12.8). Store identity, privacy, unknown tokens and idempotent appends keep their existing contracts (M12.13, M12.14).

## Before and after

On Windows, the existing environment/plist code was moved unchanged into a portable component before adding the regression check. The development command was:

```text
cargo test --locked -p agentq-runtime --lib protected_child_context_survives_clean_hops -- --nocapture
```

Before the fix, it exited 101: `protected hop scope lost or accepted invalid context`, with `left: Null` and the expected full v2 context on the right. After the fix, the same check passed.

The check covers two fresh processes using the production clean environment builder and the production launchd plist serializer:

- Full `project`, `task`, `attempt`, `actor` and `entry`; partial caller context; legacy trace-only context.
- Missing, malformed, oversized, wrongly typed and private-field context; unrelated environment exclusion.
- Nested current-span context and private-field exclusion.
- The child records through the consumer's CoreQ Store: one turn and one call after two append attempts, missing fields with null reasons, and unknown tokens with `not-reported`.

The merge-candidate commands and exact source are recorded on [PR 262](https://github.com/taskqorg/agentq/pull/262) and in the TaskQ result. CoreQ remains pinned to v0.23.0, immutable revision `34ae02cbe35065b08a5e0643d45881ec494276b6`; the consumer one-source guard is retained.

## Evidence limits

These are portable component and fake-client checks, not native macOS launchd or sandbox qualification (R12). No working profile, native input, grant, service, shipper or cursor was changed or replayed. Publication still requires merge-result CI and the exact-source runtime-tag workflow; local tests do not publish `runtime-v0.10.0`.
