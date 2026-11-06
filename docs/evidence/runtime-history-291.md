# History media (#291)

The runtime exposes a read-scoped MediaCatalog with image/video/audio/document
kind, MIME, name, size, SHA-256, native source and explicit missing/unknown reasons.
Inline bytes stay in the catalog for range reads; local assets are hashed without
loading the whole file into memory. Every local range read hashes the same byte
stream from which the requested range is collected, so changed content is refused
before any bytes are returned. Arbitrary paths are refused. Native URLs remain
references and are never fetched. No extension or assistant prose establishes media.

Native shapes cover nested Claude image/document/tool-result blocks and image
source wrappers; Codex input_image data URLs, local_image, ImageView and generated
savedPath; Antigravity quoted Path arguments, native media file URIs and brain
artifacts. Session-scoped Codex generated_images directories are also read; a
portable test proves another session's assets never enter the catalog. Structured file-change and tool path references are signature-checked.
The same catalog provides Media steps linked to the raw source/parent step. Multiple
native references can share a hash; inline backing bytes are stored once by hash.

Event::Image now retains item identity, generated status, saved_path and the native
image item. Both imageGeneration and image_generation, savedPath and saved_path
survive app-server/exec transport normalization; rollout ImageView also survives.
This restores the evidence Pipe's image method 5 needs without client JSON parsing.

## Native Mac proof

On macOS aarch64, a locally generated 16x16 red PNG was actually received/read by:

- Claude Code 2.1.319, claude-fable-5[1m]: the successful interactive turn used for
  #299, with a completed Read and native image tool result. No additional Claude turn.
- codex-cli 0.159.3: one image-input turn completed and described the red image.
- agy 1.5.2: one sandboxed read_file image turn completed SUCCESS and described red
  (#FF0000); native stream reported ACTIVE then DONE for read_file. Its sidecar
  retained quoted Path and a separate image/png artifact URI in logs/media.

The built history CLI read all three original sources without edits. Every catalog
reported available image/png descriptors of 80 bytes. Byte-range reads by SHA-256
through recovered checkpoints returned the exact eight-byte PNG signature for all
three runtimes, exit 0. Native paths and identity are retained only in private
scratch evidence, not committed fixtures. The three *-media.jsonl fixtures scrub
identity, times, paths and unrelated prompt content; pixel.png is the generated test
asset. An earlier invalid PNG in the #299 print-mode probe is not counted.

## Checks and boundaries

Portable tests cover nested/duplicate inline references, hash reads, overflow/range
validation, external URL absence, bad base64, same-path mutation, missing files,
media signatures independent of extensions, fake .mp4 text rejection, typed
imageGeneration/savedPath evidence, and all three scrubbed native fixture shapes.

Archive storage (#283) and viewer playback belong to their existing owners. Callers
can copy available catalog bytes by hash once; this PR does not claim retention of
vanished local files or implement a DashQ viewer. Native video/audio playback and
Windows native image turns remain unproved. Portable Windows tests/CI are separate
from those native proofs. No profiles, client configs or deployment were changed.

Local verification: locked full tests, offline selftest, pinned contract lint and
diff checks passed; the added Codex asset-scope regression also passed separately.
Local file ranges rehash the asset to reject mutable-source changes, so large mutable
video assets cost a full streaming hash per read; no cache or size ceiling is added.

Windows CI caught raw string substitution corrupting JSON backslashes in the fixture
setup. Fixture paths are now substituted as parsed values; nested quoted native Path
arguments are JSON-encoded once more and artifact URIs use the platform form. The
native reader is unchanged by this fixture correction. Rebased main #309 also fixes
the separate cleanup-exit provenance failure observed on macOS.

## Source-authority review corrections

Media discovery now traverses recognized native containers and tool-result fields;
it does not decode assistant/user prose or arbitrary JSON subtrees into file
references. Each contributing physical source revision is hash-checked from one
validated native stream and decoded with its own logical step identity. An
ImageView completion without a path therefore retains the started revision's
reference and exact origin, without repeatedly reopening or hashing the transcript.

Implicit generated-asset discovery anchors the selected native directory, refuses
symlinked roots/ancestors, and rechecks that anchor before and after file hashing
and range reads. Replacing a catalogued session directory with a symlink refuses
rather than reading another session. Explicit native references still use the
same content hash and full caller-selected byte ranges; URLs remain unfetched.

Focused regressions reproduce the three review findings and additionally cover
symlinked ancestors, replacement after catalog creation, text-shaped JSON and
arbitrary nested objects. Eight media tests and 16 native event tests pass, plus
offline selftest, workflow-pinned lint and diff check. Accounting's five focused
regressions also pass after the stack rebase. No full local suite, paid/native turn
or working configuration change was made; Windows/WSL, native audio/video playback,
archive retention and viewer end-to-end behavior remain unproved.

The second media review now stops at quoted conversation and text/prose containers
inside structured tool results. Sidecar media/tool_calls require the actual
Antigravity sidecar format, native root position and recognized record schema;
fields in an ordinary Codex message cannot impersonate that container. A shared
Codex image classifier supplies events, steps and media, including ImageGeneration.
Direct structured tool-result paths/files and native arguments still authorize
signature-checked reads; existing physical origin and generated-anchor guards stay.

Final focused proof: 11 media tests, 12 step tests, 16 event tests and a read-only
probe of the three existing real Mac sources pass, plus offline selftest, pinned
lint and diff check. Each native source still returns PNG signature bytes. No paid
turn, full local suite or working configuration change was made; previous platform,
archive and viewer gaps remain.
