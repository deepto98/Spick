# macOS input-method prototype

Spick needs to place text at a caret without borrowing the clipboard or rewriting a control's whole value. On macOS, a palette-style InputMethodKit helper is the most promising native seam. It receives the same text-input client macOS gives to an input method and can ask that client to replace its current selection.

This prototype is deliberately not part of the normal build yet.

## What is implemented

When the shortcut starts, the Rust target tracker still captures the frontmost application, focused Accessibility element, selection, and invalidation notifications. If the experimental feature is compiled and the helper is available, it also asks the helper to arm the exact active InputMethodKit controller and client.

The returned lease is random, short-lived, tied to that client and selection, and removed before an insertion attempt. A repeated request cannot type twice. At commit time both sides recheck the target. The helper also refuses marked text, secure event input, a changed application, a changed selection, expired work, and oversized or malformed messages.

After dispatch, the helper reads the inserted range back. Only an exact match is `Confirmed`. A timeout or exception after dispatch is `Dispatched`, which Spick treats as indeterminate and never retries automatically.

The protocol self-test covers arm, insert, disarm, Unicode text, truncated frames, oversized fields, stale identifiers, and response encoding. Rust has a matching big-endian codec and golden-frame tests.

## Why it is still gated

The development broker accepts only the current macOS user and checks the peer process bundle identifier. That keeps accidental clients out, but another process running as the same user can imitate a bundle identifier. Before release, the desktop app and helper must verify one another's audit identity and code signature before a transcript is sent.

The release bundle also needs one signing identity across both processes, notarization, and compatibility runs against real controls in TextEdit, browsers, VS Code, terminals, and Electron apps. InputMethodKit support is a control-level result; one working field does not prove an entire app works.

Until those checks are complete:

- the Cargo feature is off by default;
- the product reports automatic insertion as unavailable;
- transcripts remain available through the explicit copy recovery path; and
- no clipboard fallback runs silently.

## Development checks

Run the complete helper and feature-gated Rust check on macOS:

```sh
npm run check:input-method
```

That command builds universal `arm64` and `x86_64` helper binaries, runs the native protocol self-test, verifies the ad-hoc development signature, runs the feature tests, and runs strict Clippy.

The installer is separate because it changes the current user's macOS input sources:

```sh
./scripts/install-input-method.sh
```

Use it only for an intentional compatibility session. It refuses `sudo`, serializes builds and installs, snapshots the current source state, then deselects, disables, and asks any running development helper to stop before replacement. It stages and verifies the new bundle before moving it into `~/Library/Input Methods`, with a second state check immediately before the move. Once macOS registration begins, a failed new bundle is left in place rather than restoring files underneath a possibly running input-method process; the installer prints the preserved backup path for manual recovery. The generated bundle is for local development, not distribution.
