# macOS frontend keeps native stdout: task 266

Scope: `agentq agent host` on macOS, under agentq.md § 12 M12.8. The live launch of digest `425c5924046338bf13167e5bf76eb827533435ed09e5f3cf336b6744d9bf2118` ended with `native stdout interrupted; retain scope` before `initialize`.

## Cause

The frontend sets its two scope listeners non-blocking so that `accept` stays bounded. Darwin's `accept` inherits `O_NONBLOCK` from the listener, and Linux's does not. The accepted stream was therefore non-blocking while `set_read_timeout(None)` asked for an open-ended read, so the first read of an idle native stream failed.

`protected_scope_stream_waits_for_idle_native_output` runs the frontend's own `scope::accept`, then the same `set_read_timeout(None)` and `io::copy`, with a peer that writes after 200 ms. On macOS 27.2 arm64, before the fix:

```text
assertion `left == right` failed
  left: Err(Some(35))
 right: Ok(6)
```

Errno 35 is `EAGAIN` on Darwin. This is a reproduction on the same code path. The live run recorded no errno, because the old build discarded it in its reason. The frontend now also logs the copy error to stderr, so a later live failure names its errno. The reason text is unchanged.

The same state made the binding read and the input and stderr streams depend on timing. They share the fix.

## Fix and guards

`scope::accept` clears `O_NONBLOCK` on the accepted stream before it sets the existing 5 s deadlines. The test passes after the fix.

No no-replay, fencing or retained-scope code or assertion changed: an interrupted copy still returns the same retained-scope error, and the `protected_` guards are unedited.

## Retained operation

No command in this task launches, cancels or reconciles through the Mac host. The retained failed operation stays in the Mac journal. Its SHA-256 is `ab4281c3aa8971a1ab965d68495364c8dcc5c9410de2dbe29f7884b2b77d96f6` at 9,862 bytes, read on the Mac before the change. The read-only task [#272](https://github.com/taskqorg/agentq/issues/272), pinned to the Mac host, reported the same size and digest at `b5431b118c68494bdd8c8075715cc74c5b6485df`. The hand-in attempt of this task read the journal again on the Mac and got the same size and digest. These reads printed only the size and digest and ran no host action.

## Signed adapter

The new final adapter replaces digest `425c5924046338bf13167e5bf76eb827533435ed09e5f3cf336b6744d9bf2118`.

- Binary: `agentq-0.0.0-dev-macos-aarch64`, 7,598,192 bytes, SHA-256 `711a6066519d95f625bbffd8531ed87658f90be993d9dafc91a1a21d111d32e3`.
- Build: artifact `13499089022` of [release run 42213746472](https://github.com/taskqorg/agentq/actions/runs/42213746472), at merge `5535edc1b7f0b73b36ba592d69b6917c9379d832` of fix `e34f88bed0389006413aedc266bc8c0fab056660` into `ede9e465b0caa7b567f3370f19367e5fdd6b51cd`.
- Public manifest: [adapter-macos-aarch64.json](runtime-266/adapter-macos-aarch64.json), public key `7f739abab8625718e2e5dbedf0deb5a98373260d3c22ecd20a1148f42bf6d584`.
- Signing: [#269](https://github.com/taskqorg/agentq/issues/269) signed on the host that already holds the publication key and merged the manifest in [#271](https://github.com/taskqorg/agentq/pull/271). The key did not move and was not printed. The #269 review accepted this signature.

This task rechecked the published result on Windows. A fresh download into a new empty directory has the size and digest above. `sign_adapter verify` printed `signed adapter verified` for the manifest at `0de008b13465fd68d4df008cc33c21343a019ce0`, and with a zeroed signature it refused with `protected signature invalid; authority unknown`. The downloaded binary was not executed.

The binary does not contain [#265](https://github.com/taskqorg/agentq/pull/265), which main merged after the build base. `git diff ede9e46 0de008b` is empty for `runtime/src/agent/protected.rs`, `runtime/src/agent/host.rs`, `runtime/src/agent/host/`, `runtime/examples/sign_adapter.rs` and `agentq/host.rs`. A consumer that needs #265 inside the adapter needs a new build and a new signature.

## Limits

These checks prove the socket mode, the stream behavior and the published signature. They do not prove a native launch or a TCC grant; assetq#52 A4 qualifies those. No policy, trust manifest, permit or grant changed, so no host trusts this adapter until an operator provisions it.
