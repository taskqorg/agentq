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

No command in this task launches, cancels or reconciles through the Mac host. The retained failed operation stays in the Mac journal. Its SHA-256 is `ab4281c3aa8971a1ab965d68495364c8dcc5c9410de2dbe29f7884b2b77d96f6` at 9,862 bytes, read before the change and again before hand-in.

## Limits

These checks prove the socket mode and the stream behavior. They do not prove a native launch, a TCC grant or the signed adapter; assetq#52 A4 qualifies those.
