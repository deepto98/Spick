# Spick

Spick is a macOS-first desktop dictation app in active development. Tap Option once to start and once more to stop, or hold Option while speaking. The current development build can transcribe with a downloaded local model or a configured cloud provider, offers opt-in cleanup, tracks the field where dictation began, and inserts the result into compatible controls.

The app combines a Tauri 2 shell, a Rust application core, and a React/TypeScript interface. The native core owns the global shortcut, ephemeral microphone capture, verified model downloads, in-process `whisper.cpp` transcription, cloud requests and credentials, guarded macOS field tracking, and local usage/history storage.

> Development builds use a narrow Accessibility operation that replaces only the captured selection and then reads the inserted UTF-16 range back. They never replace a field’s whole value and never retry after a write may have occurred. Unsupported controls keep the transcript available for an explicit copy.

## Prerequisites

Development currently targets macOS. Install:

- Node.js 22 (the repository includes `.nvmrc`)
- Rust stable, including Cargo
- Xcode Command Line Tools (`xcode-select --install`)
- CMake 3.20 or newer (`brew install cmake`)

## Run locally

```sh
npm install
npm run tauri dev
```

For frontend-only work, use `npm run dev`. The browser build cannot exercise Tauri commands or native window behavior.

Open **Engines** in the desktop build to download and select a local model, or save a provider key and select OpenAI, xAI, or Gemini. Downloads can be cancelled and are written to app-local data only after the declared byte length and SHA-256 both match.

### Local development data

`npm run tauri dev` uses the same OS application directories as other builds with the `app.spick.desktop` identifier. On macOS, the files are in:

```text
~/Library/Application Support/app.spick.desktop/settings.json
~/Library/Application Support/app.spick.desktop/spick.sqlite3
~/Library/Application Support/app.spick.desktop/models/
```

SQLite may also create `spick.sqlite3-wal` and `spick.sqlite3-shm` while Spick is running. Quit the development app before resetting it, then remove `settings.json` (and its `.bak`) plus all three `spick.sqlite3*` files. Remove `models/` only if downloaded model weights should be reset too. Provider keys are separate Keychain items under the `app.spick.desktop` service; remove them in **Engines** if those should also be reset. Windows keeps the database under the app-local data directory (`%LOCALAPPDATA%`), while Linux uses the platform local-data/XDG directory; settings follow each platform's config directory.

The Option gesture needs macOS **Input Monitoring** in addition to microphone and Accessibility access. Spick shows a temporary `⌘ ⇧ Space` fallback until Input Monitoring is allowed; returning to Spick activates Option without requiring a rebuild. Option-letter, Option-click, dual-Option, and other chords are passed through. Chords seen before the hold threshold prevent dictation, and external pointer input during an active hold cancels it before delivery. Pointer input proven to target the nonactivating Spick HUD remains available so its move grip works while speaking.

Run the project checks before committing:

```sh
npm run check:all
```

`npm run check:all` verifies formatting, linting, frontend tests, TypeScript, the production web build, Rust tests, strict Clippy, and the native compile check. Use `npm run check` when you only need the frontend gate.

The macOS input-method prototype has its own opt-in check because it builds a universal Objective-C bundle and an experimental Rust feature:

```sh
npm run check:input-method
```

Real-control testing uses a separate fixed-fixture diagnostic build. It does not listen to the microphone or accept arbitrary text/targets, and its preflight never changes the selected input source:

```sh
npm run build:desktop:input-method:compatibility:development
npm run preflight:input-method:compatibility
npm run run:input-method:compatibility -- \
  --case macos.chrome.input.caretAscii --profile cold
```

## Architecture

Spick keeps audio capture, transcription, cleanup, language policy, and text insertion as separate responsibilities. That boundary is intended to support local `whisper.cpp` models, capability-aware cloud providers, and platform-specific insertion without coupling the product interface to one engine.

Read [the architecture](docs/architecture.md) for the pipeline, native seams, language policy, storage boundaries, and provider model. Delivery order and exit criteria live in [the milestone plan](docs/milestones.md).

## Privacy principles

- Local-only mode must never silently fall back to a cloud provider.
- Raw audio is ephemeral by default and transcript history is opt-in.
- API keys are sent directly from the transient entry field to the operating system credential store, cleared from the field immediately, never returned to the frontend, and never written to ordinary settings or SQLite.
- Logs and statistics omit dictated text, raw audio, clipboard contents, and credentials by default.
- Secure fields and stale focus targets must fail safely instead of receiving inserted text.

The development path enforces these rules for local and cloud transcription, target tracking, and selection-only Accessibility insertion. Release insertion remains separately gated.

## Current limitations

- Usage totals, Unicode word counts, capture-time WPM, language groups, optional transcript history, and vocabulary entries are stored in native-owned SQLite. Transcript text is written only when **Keep transcript history** is enabled; aggregate receipts never contain transcript or target-app text. The latest recoverable transcript also remains in memory for explicit copy.
- Enabled vocabulary phrases bias the next local or cloud session through a bounded prompt/keyterm snapshot. This means enabled phrases can leave the Mac when a cloud provider runs. Pronunciation/replacement metadata is saved for future adapters but does not rewrite dictated text yet.
- Microphone capture, a live level meter, verified local model downloads, cancellation, and local `whisper.cpp` batch transcription are implemented. Cleanup is off by default. Its local English option removes pause-marked “um”, “uh”, and “erm” only when punctuation can be repaired safely; bare words, identifiers, quoted uses, explicit references, unknown languages, and non-English transcripts are left alone. Voice activity detection and translation are not connected yet.
- OpenAI `gpt-4o-transcribe`, xAI Speech to Text, and experimental Gemini 3.5 Flash batch audio are connected. Requests use fixed HTTPS endpoints and bounded bodies/responses; Gemini requests explicitly set `store: false`. Selecting a cloud engine uploads each completed recording to that provider. Optional fallback tries only the first configured provider compatible with the recording’s language (OpenAI, then xAI, then Gemini) after an eligible local runtime failure and never fans one recording out to several services. Cancelling prevents delivery but cannot recall an upload that already began.
- Provider request builders and parsers are covered without live billable API calls. A real key, network connection, provider account, and current provider availability are therefore runtime dependencies rather than test fixtures.
- Shortcut dictation captures an accessible Mac text field or text area before recording and checks the exact application, element, selection, protection state, and change notifications again after transcription.
- Password fields and controls marked as protected content are rejected before the microphone starts. Accessibility permission is required for shortcut-driven field tracking.
- Debug builds attempt selection-only Accessibility insertion and confirm the exact inserted range before reporting success. If a setter may have written but cannot be confirmed, delivery is indeterminate and is never retried. Release builds still keep automatic insertion gated behind the authenticated InputMethodKit prototype and compatibility work. A recoverable transcript remains in memory on Today when a control is unsupported.
- macOS is the first validation target; Windows and Linux native integrations come later.
- The HUD can collapse to a `56×116` vertical live-wave bar. Its physical position and presentation persist and are clamped when monitors or DPI layouts change. On macOS it is converted once to a true nonactivating `NSPanel`; the pinned community integration, fail-closed fallback, and release risks are documented in [docs/macos-hud.md](docs/macos-hud.md).
- Development/release signing gates for the input-method pair are in place. Final nested packaging, notarization, updates, and production accessibility review remain future work.

The input-method design, development commands, and remaining release gates are documented in [docs/input-method.md](docs/input-method.md). A guarded paste experiment may follow, but it will stay clearly labelled best-effort: neither event delivery nor clipboard restoration is atomic on macOS.
