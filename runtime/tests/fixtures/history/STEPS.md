# Native step fixtures

`codex-steps.jsonl` and `claude-steps.jsonl` are structural extracts from local
Mac native transcripts. Versions: Claude 2.1.293, Claude 2.1.319, Codex 0.159.3.
All content, paths (including map keys), signatures, identities and timestamps are scrubbed; non-time numbers,
booleans, nulls and native envelope/field shapes are retained. These prove decoding,
not the operation of another native version or platform.

Fork regression uses an invented minimal reproduction of observed nested session metadata.
Orca decoding reference and MIT attribution: `runtime/src/agent/history/NOTICE`.

`claude-statusline.json` is a projection of a native Claude Code 2.1.319 statusline
callback from a single image-read turn on macOS aarch64. Only model, context-window
and limit-window fields remain; session identity and reset timestamp are scrubbed.
Missing five-hour usage is native absence, not zero. No user settings were installed.
