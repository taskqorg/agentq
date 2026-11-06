# Stopped-run observation

Native Mac proof used codex-cli 0.159.3 through `Runtime::launch` in a new temporary
Git repository. The prompt requested only a fixed reply and no tools or file edits.
After the native session receipt, `Runtime::cancel` killed the owned child (PID 36491).
`poll` observed termination; `stop_event(run, u64::MAX)` returned:

```text
cause=Signal exit=None outcome=unknown activity=true activity_unknown=None
```

The absence of an exit code remains explicit. No native turn-end, successful report,
permit release or retry is inferred from the stop observation. The probe did not
change client configuration, working profiles or any installed AgentQ artifact.
The initial attempt was refused by Codex's Git-directory check before any session
receipt; it is not counted as native signal proof. Creating a temporary Git repository
made the next native turn eligible without changing a working profile.

Portable runtime doubles cover zero exit without native end, killing after checkpoint
recovery, retained last activity and stable repeated IDs, for Claude, Codex and agy.
A lingering Claude double covers a native turn-end while its process is still live.
These are adapter/process tests, not native qualification of those other clients.
Windows native signal/recovered-owner proof remains for the owner's Windows host.

A first local agent-suite run hit the existing protected fixture's non-atomic JSON
read race (`runtime/tests/support/protected.rs:11`, EOF while parsing a value). The
shared fixture save helper now uses existing `coreq::write_atomic`, so readers cannot
observe its intermediate empty file. This changes only the fixture publication boundary,
not production behavior or authority checks.

The rebased reader derives stop activity using the same self-contained history fold,
restricted to the current turn boundary. A regression retains a shell command when
its last native record is only a result. The Intel CI failure (Running before EOF
delivery) was a separate asynchronous-reader test race: the fixture now waits for
the runtime state under its existing deadline after process exit, without adding a
timeout or changing production completion authority.

After rebasing main #309, a runtime cleanup after native end remains unknown exit
provenance without becoming LostOwner. The stopped-run regression covers both the
initial TurnEndProcessLive observation and the completed cleanup. Other Runtime
implementations explicitly refuse unsupported stop observations by default, keeping
the registration example and external trait implementations source-compatible.
