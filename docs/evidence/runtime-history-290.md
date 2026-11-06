# History source qualification (#290)

The Runtime trait and descriptor expose 24 history readers. The binding source and
support matrix is in RUNTIME.md, History sources. Execution enumeration is unchanged;
history-only readers refuse execution/control. Consumer code reads descriptor
capabilities, typed steps/live facts/media, source bytes and opaque checkpoints.
For snapshot pages it replaces the prior view when a newer generation has reset=true.
Consumers must not parse native records or switch on runtime names.

## Mac native evidence

Claude Code 2.1.319, codex-cli 0.159.3 and agy 1.5.2 native image/session reads are
recorded in runtime-history-291.md; no extra model turns were made for these readers.

Hermes Agent 0.21.6+509.gf925b01 (native version output dated 2026.9.24): seven sessions
were discovered from its current state.db in the final readback. One selected session returned 16 converged
steps: seven messages, one metadata, three shell, two tool and three aggregate usage steps. Unreported per-call usage stays unknown; native model aggregates remain aggregate
usage and tool parts do not invent model calls.

OpenCode 1.18.23: 62 native sessions were discovered from its current SQLite store.
One selected session returned 128 steps: 20 messages, 13 thinking, one shell, one
file change, four tool, three web, three image, 30 usage and 53 metadata steps.
Message aggregates are excluded from call accounting when step-finish usage exists.
Native DB, WAL and SHM SHA-256 values were identical before and after these reads.
Only temporary copies were opened with SQLite; native files were opened as bytes.

A new Hermes one-shot turn returned HISTORY_OK with no tools. Its native session
was then opened through the CLI history API: five steps (two messages, metadata
and two native aggregate usage records), exact assistant text,
checkpoint recovery with no duplicate steps, and an exact SQLite-header range read.
No working profile or configuration changed.

The fixtures retain 13 Hermes and 98 OpenCode native SQL row/cell shapes after
scrubbing content, IDs, paths including map keys, and numbers. They are labeled
projections, not SQLite file bytes. Attribution and other agent fixtures: NOTICE
and runtime/tests/fixtures/history/STEPS.md.

## Open native gaps

OpenCode's fresh default turn failed with APIError 404: no endpoint supports tool
use. A second launch disabled tools only through OPENCODE_CONFIG_CONTENT for that
process, without changing files, and failed with APIError 402 requiring a minimum
balance for image/video output. No balance, model, login or working configuration
was changed. The existing native-session read above is proven; a successful fresh
OpenCode turn remains unproved.

Grok 1.0.46 (2765805b9442) has no local transcript source. A single tiny no-tool proof
attempt failed before a turn with Not signed in. No login or credentials were changed.
It remains fixture-only.

OpenClaw 2026.4.14 (323493f) has no local agent session source. Its existing config is
rejected for plugin hook fields allowConversationAccess and timeoutMs, so no native
turn was launched and no config was edited. It remains fixture-only.

The other 17 additional readers have no installed CLI/native proof on this Mac and
remain fixture-only. Passing format fixtures never qualifies another native version.
Windows/WSL native proof remains for the owner: discover and open a native session
for each installed client, read a page, recover its checkpoint, read raw ranges and
media, and compare original source bytes before/after. Fixture CI is not that proof.

## Regression scope

history_sources.rs exercises the source registry, native layout discovery and opaque
IDs, all 21 additional agents, JSON/JSONL source bytes, snapshot replacement and
stable identities, SQLite/WAL read-only behavior, evolving OpenCode 2 ordering,
parent-session links, complete shell results and usage deduplication. Existing
history_steps/history_live/history_media cover the three execution runtimes and
shared derived facts. No archive or viewer is implemented here; the media/archive
boundary remains the one documented for #291.

Final stack rebase preserves main's registered native parsers, copied-session alias
binding and required per-runtime quota method. The additional readers explicitly
report quota/readiness as unsupported or unknown. JSONL separators are skipped, but
malformed nonempty JSON refuses without changing the checkpoint. Final local
qualification passed 25 focused source/step/live/media tests, offline selftest, pinned
lint and diff checks; full suites run in CI. Fresh CLI reads of Hermes and OpenCode
recovered without duplicate steps and left native DB/WAL/SHM digests unchanged.

Custom XDG data homes resolve to the same client root through Runtime::home and
session discovery; the focused source regression verifies that shared binding.

Raw SQLite source parts contain the complete DB/WAL files, potentially including
other sessions. Session-scoped projections do not authorize exporting those complete
files; archive consumers retain their separate content authority checks.
