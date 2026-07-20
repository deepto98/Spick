# macOS input-method prototype

Spick needs to place text at a caret without borrowing the clipboard or rewriting a control's whole value. On macOS, a palette-style InputMethodKit helper is the most promising native seam. It receives the same text-input client macOS gives to an input method and can ask that client to replace its current selection.

This prototype is deliberately not part of the normal build yet.

## What is implemented

When the shortcut starts, the Rust target tracker still captures the frontmost application, focused Accessibility element, selection, and invalidation notifications. If the experimental feature is compiled and the helper is available, it also asks the helper to arm the exact active InputMethodKit controller and client.

The returned lease is random, short-lived, tied to that client and selection, and removed before an insertion attempt. A repeated request cannot type twice. At commit time both sides recheck the target. The helper also refuses marked text, secure event input, a changed application, a changed selection, expired work, and oversized or malformed messages.

After dispatch, the helper reads the inserted range back. Only an exact match is `Confirmed`. A timeout or exception after dispatch is `Dispatched`, which Spick treats as indeterminate and never retries automatically.

The protocol self-test covers arm, insert, disarm, Unicode text, truncated frames, oversized fields, stale identifiers, and response encoding. Rust has a matching big-endian codec and golden-frame tests.

Every broker connection is mutually authenticated before a request header or transcript byte crosses it. Each side reads the Unix socket's immutable audit token and asks Security.framework for the live peer code object. Secure mode then requires the exact desktop/helper signing identifier, an Apple-anchored signature from the same Developer Team, hardened runtime, and no debugging or dynamic-loader entitlements that permit same-user code injection. It rejects ad-hoc code and never falls back to a PID or bundle-name check.

## Why it is still gated

The release bundle still needs notarization, a prebuilt helper payload nested into the desktop bundle, and compatibility runs against real controls in Notes, TextEdit, browsers, ChatGPT, VS Code, and other Electron apps. InputMethodKit support is a control-level result; one working field does not prove an entire app works.

## Real-control compatibility harness

The compatibility harness is a separate debug-only app mode. It is not a dictation build: its Tauri context is stripped of configured windows before build, model preloading is skipped, every shortcut event is consumed by the one-shot probe, and the microphone, Whisper, clipboard, settings, transcript store, and HUD are never initialized. The case catalog fixes the target bundle identifier, required caret/selection shape, fixture text, and expected result. There is no argument for arbitrary text or an arbitrary target.

Build a signed development harness with the same Apple Development identity and Team ID as the helper:

```sh
npm run build:desktop:input-method:compatibility:development
```

On a machine without a development identity, the explicitly unsafe local pair remains available for disposable testing only:

```sh
npm run build:desktop:input-method:compatibility:unsafe-adhoc
```

Before preflighting or running that ad-hoc pair, acknowledge that its signatures provide integrity but not trusted provenance:

```sh
export SPICK_INPUT_ALLOW_UNSAFE_ADHOC_COMPATIBILITY=YES
```

The preflight verifies all static signatures, identifiers, runtime flags, Team requirements, and signed capability markers before executing any inspection command. It never executes ad-hoc artifacts without that explicit acknowledgement.

Building the app does not install or select the input source. Once the matching helper has been installed and selected through the separately guarded installer, inspect readiness without changing any input-source state:

```sh
npm run preflight:input-method:compatibility
```

List the compiled cases, then run exactly one in an interactive terminal:

```sh
src-tauri/target/debug/bundle/macos/Spick.app/Contents/MacOS/spick-desktop \
  --list-input-method-compatibility-cases
npm run run:input-method:compatibility -- \
  --case macos.chrome.input.caretAscii --profile cold
```

The runner never opens or focuses a target app and never installs, registers, enables, disables, selects, or deselects an input source. It creates a private `target/input-method-compat` directory, launches one fixed-fixture probe, then records a separate closed-enum process-lifecycle result and visual review. Attempt, lifecycle, and review files are create-new and bind the exact attempt bytes by SHA-256. A small append-only journal is synced before capture so an evidence-storage failure aborts before any input-method lease or insertion. A probe that does not exit cleanly cannot qualify later, even if it sealed an attempt first.

Evidence deliberately excludes fixture text, audio, clipboard data, field contents, selections, PIDs, audit tokens, window/document titles, URLs, usernames, hostnames, file paths, screenshots, and free-form notes. Positive insertion cases require a `Confirmed` helper result, while protected-field cases require the catalog's exact pre-insertion refusal. An indeterminate delivery is never retried or promoted by visual appearance. A confirmed record binds the live audit-token-authenticated helper CDHash to the separately inspected installed helper, and target versions come from the exact process captured by Accessibility.

A case needs three reviewed passes—cold, warm, warm—under the same clean revision, macOS build, architecture, target version, signing policy, and secure peer authentication before it can be labelled compatible. “Cold” means the target app was fully quit and reopened and this is the first compatibility attempt in that launch. “Warm” means the same target-app launch remains open after a completed matching attempt, with the disposable field or document reset. The runner requires a typed profile attestation and stores that closed profile in the attempt. Unsafe ad-hoc results remain development-only. Compatibility is claimed per control, never for an entire application or a newer version by inference.

Until those checks are complete:

- the Cargo feature is off by default;
- release builds report InputMethodKit insertion as unavailable;
- ordinary debug builds first use verified `AXSelectedText` replacement and
  use a guarded paste fallback for web, Electron, and other custom controls;
- the fallback refuses a transcript containing a carriage return or line feed
  before any clipboard access, so it is not a terminal or multiline insertion
  path; direct Accessibility and InputMethodKit insertion are unaffected;
- the fallback retains no more than 64 MiB of snapshot data, but AppKit
  materializes lazy pasteboard representations before that cap can be checked;
  the cap is not a streaming or pre-allocation bound;
- after taking the in-memory snapshot, the fallback
  revalidates the exact original target immediately before one PID-scoped
  `Cmd-V`, and attempts to restore the snapshot only while its change-count
  and ownership marker still match; if ownership is already lost, Spick leaves
  the current clipboard alone;
- the ownership check and restore are best-effort and non-atomic because the
  public pasteboard API has no compare-and-swap operation; a microscopic race
  with a concurrent clipboard change remains;
- a selectionless custom field cannot prove its original caret or selection,
  so its dispatch is indeterminate and is never retried;
- any other unconfirmed paste dispatch is also indeterminate and is never retried;
- macOS can deny or prompt for clipboard access under its privacy policy;
- secure and protected controls are refused before recording; and
- transcripts remain available through the explicit copy recovery path when a control rejects insertion.

This best-effort fallback belongs only to the ordinary debug development path.
It does not replace or relax the separately authenticated InputMethodKit
release design.

## Development checks

Run the complete helper and feature-gated Rust check on macOS:

```sh
npm run check:input-method
```

That command exercises both peer policies, builds universal `arm64` and `x86_64` helper binaries, verifies exact signing identifiers, runs live audit-token and signature-policy self-tests, runs both feature sets, and runs strict Clippy. It finishes by leaving a non-installable `check` artifact whose broker uses the secure policy and therefore rejects its own ad-hoc signature.

## Signing modes

`build-input-method.sh` has four closed, non-fallback modes:

- `check` is the default. It is ad-hoc only so ordinary source checks work without a certificate; its helper still compiles the secure peer policy and cannot be installed by the installer.
- `development` requires `APPLE_SIGNING_IDENTITY` as the 40-character SHA-1 hash of an Apple Development certificate plus its 10-character `SPICK_INPUT_TEAM_ID`. Both artifacts use hardened runtime.
- `release` requires a Developer ID Application identity, hardened runtime, and a timestamp. Building this payload is not notarization, and the development installer intentionally refuses it.
- `unsafe-adhoc` compiles an explicit same-user development escape hatch. Both peers still need valid ad-hoc signatures with Spick's exact signing identifiers, but any process in the account can forge those; release builds forbid this Cargo feature.

The source-management tool is independently signed as `app.spick.desktop.input-source-tool`. The build and installer verify both artifacts, their expected authentication mode, and—where applicable—their shared Team ID before executing either one.

## Signed compatibility build

Use an actual `.app` bundle for compatibility work. A raw `cargo run` binary has a hash-derived signing identifier and is intentionally rejected.

With an Apple Development identity available:

```sh
export APPLE_SIGNING_IDENTITY=0123456789ABCDEF0123456789ABCDEF01234567
export SPICK_INPUT_TEAM_ID=A1B2C3D4E5
./scripts/install-input-method.sh --development
npm run build:desktop:input-method:development
open src-tauri/target/debug/bundle/macos/Spick.app
```

Replace the example identity and Team ID with the certificate values from your keychain. The installer changes the current user's input-source state, so run it only when starting an intentional hands-on session.

When no Apple Development identity is available, the deliberately unsafe local path requires two explicit acknowledgements:

```sh
SPICK_INPUT_ALLOW_UNSAFE_ADHOC_INSTALL=YES \
  ./scripts/install-input-method.sh --unsafe-adhoc
npm run build:desktop:input-method:unsafe-adhoc
open src-tauri/target/debug/bundle/macos/Spick.app
```

That path is for disposable local compatibility work only. Quit the app and disable/remove the development input source after the session.

The installer refuses `sudo`, serializes builds and installs, authenticates both artifacts before its first tool execution, snapshots the current source state, then deselects, disables, and asks any running development helper to stop before replacement. It reverifies after staging and again after the final move into `~/Library/Input Methods`, with a second state check immediately before replacement. Once macOS registration begins, a failed new bundle is left in place rather than restoring files underneath a possibly running input-method process; the installer prints the preserved backup path for manual recovery.
