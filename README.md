# Spick

Spick is a macOS-first desktop dictation app in active development. Tap Option once to start and once more to stop, or hold Option while speaking. The current development build can transcribe with a downloaded local model or a configured cloud provider, offers opt-in cleanup, tracks the field where dictation began, and inserts the result into compatible controls.

The app combines a Tauri 2 shell, a Rust application core, and a React/TypeScript interface. The native core owns the global shortcut, ephemeral microphone capture, verified model downloads, in-process `whisper.cpp` transcription, cloud requests and credentials, guarded macOS field tracking, and local usage/history storage.

macOS builds use a narrow Accessibility selection replacement for compatible native controls. Web, Electron, and other custom editors can use a guarded paste fallback for transcripts without carriage returns or line feeds. The fallback refuses multiline text before touching the clipboard, so it is not a terminal insertion path; direct Accessibility and the optional InputMethodKit prototype do not have this restriction. Spick takes an in-memory snapshot of the general pasteboard, revalidates the exact original target, and sends one `Cmd-V` to that application. It attempts to restore the snapshot only while its change-count and ownership marker still match; if ownership is already lost, it leaves the current clipboard alone. That check and the following restore are not atomic, so a microscopic race with a concurrent clipboard change remains. macOS may also deny or prompt for clipboard access under its own privacy policy. A selectionless custom field cannot prove that its original caret or selection survived, so its dispatch remains indeterminate. Indeterminate delivery is never retried and leaves the transcript available for explicit copy. Secure and protected fields are refused before recording.

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

Open **Engines** in the desktop build to download, import, and select a local model, or save a provider key and select OpenAI, xAI, or Gemini. Downloads can be cancelled and are written to app-local data only after the declared byte length and SHA-256 both match. **Import model** accepts a trusted whisper.cpp GGML `.bin` file selected through the native file picker. Spick copies it into app-local storage, checks it with the bundled runtime, and never sends or retains its original path.

After a dictation attempt, **Today → Last handoff** shows coarse startup and processing timings. Startup milestones cover target capture, microphone setup, state-event delivery, the native widget show call succeeding, and the microphone stream becoming ready. The widget measurement does not claim that a WebView frame was painted. Diagnostics live only in memory until Spick quits and never include the recording, transcript, target app, device, language, model/provider identity, or error text. Failed and cancelled starts keep unreached stages blank.

### Local development data

`npm run tauri dev` uses the same OS application directories as other builds with the `app.spick.desktop` identifier. On macOS, the files are in:

```text
~/Library/Application Support/app.spick.desktop/settings.json
~/Library/Application Support/app.spick.desktop/spick.sqlite3
~/Library/Application Support/app.spick.desktop/models/
```

SQLite may also create `spick.sqlite3-wal` and `spick.sqlite3-shm` while Spick is running. Quit Spick before resetting it, then remove `settings.json` (and its `.bak`) plus all three `spick.sqlite3*` files. Cloud API keys are stored separately as `cloud-credentials.json` in the same app-local data directory; remove that file too for a complete reset. Remove `models/` only if downloaded model weights should be reset too. To revisit onboarding without deleting data, use **Settings → General → Restart setup**. To erase its WebView `localStorage` marker as part of a full manual reset, quit Spick and also remove `~/Library/WebKit/spick-desktop/`. Windows keeps the database and credential file under the app-local data directory (`%LOCALAPPDATA%`), while Linux uses the platform local-data/XDG directory; settings follow each platform's config directory.

The Option gesture needs macOS **Input Monitoring** in addition to microphone and Accessibility access. Spick shows a temporary `⌘ ⇧ Space` fallback until Input Monitoring is allowed; returning to Spick activates Option without requiring a rebuild. Option-letter, Option-click, dual-Option, and other chords are passed through. Chords seen before the hold threshold prevent dictation, and external pointer input during an active hold cancels it before delivery. Pointer input proven to target the nonactivating Spick HUD remains available so its move grip works while speaking.

Onboarding checks microphone access before shortcut practice. The permission prompt is shown only from an explicit button inside Spick; a shortcut never opens a system dialog after another app’s field has been captured. If access was denied, the same button opens the macOS Microphone privacy pane.

The widget says **Opening microphone** while the selected input is being prepared. It changes to **Listening** only after the native stream has started; cancelling or releasing the shortcut during startup discards that attempt without transcription. An input that does not become ready within eight seconds fails cleanly and releases its field, HUD lease, and audio owner instead of leaving the app stuck.

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
- API keys are sent directly from the transient entry field to a private app-local credential file, cleared from the field immediately, never returned to the frontend, and never written to ordinary settings or SQLite.
- The credential file is restricted to the current user on Unix systems. This avoids Keychain prompts in development but provides weaker same-user process isolation than the operating-system credential store.
- Logs and statistics omit dictated text, raw audio, clipboard contents, and credentials by default.
- Secure fields and stale focus targets must fail safely instead of receiving inserted text.

The macOS development and release paths enforce these rules for local and cloud transcription, target tracking, and guarded insertion.

## Current limitations

- Usage totals, Unicode word counts, capture-time WPM, language groups, optional transcript history, and vocabulary entries are stored in native-owned SQLite. Transcript text is written only when **Keep transcript history** is enabled; aggregate receipts never contain transcript or target-app text. The latest recoverable transcript also remains in memory for explicit copy.
- Enabled vocabulary phrases bias the next local or cloud session through a bounded prompt/keyterm snapshot. This means enabled phrases can leave the Mac when a cloud provider runs. Pronunciation/replacement metadata is saved for future adapters but does not rewrite dictated text yet.
- Microphone capture, selectable input devices, a live level meter, verified local model downloads/imports, cancellation, and local `whisper.cpp` batch transcription are implemented. Imported files must be trusted because whisper.cpp parses model data in-process. The linked speech-model loader accepts whisper.cpp's custom GGML `.bin` format, not general GGUF or LLM files. A buffer overrun aborts before transcription so discontinuous audio can never become inserted text. Cleanup is off by default. Its local option removes a reviewed set of standalone hesitation sounds in English, Spanish, French, German, Hindi, Italian, Russian, Japanese, and Chinese while preserving quoted uses, obvious word or code references, unknown languages, and unreviewed languages. Voice activity detection and translation are not connected yet.
- OpenAI `gpt-4o-transcribe`, xAI Speech to Text, and experimental Gemini 3.5 Flash batch audio are connected. Requests use fixed HTTPS endpoints and bounded bodies/responses; Gemini requests explicitly set `store: false`. Selecting a cloud engine uploads each completed recording to that provider. Optional fallback tries only the first configured provider compatible with the recording’s language (OpenAI, then xAI, then Gemini) after an eligible local runtime failure and never fans one recording out to several services. Cancelling prevents delivery but cannot recall an upload that already began.
- Provider request builders and parsers are covered without live billable API calls. A real key, network connection, provider account, and current provider availability are therefore runtime dependencies rather than test fixtures.
- Shortcut dictation captures an accessible Mac text field or text area before recording and checks the exact application, element, selection, protection state, and change notifications again after transcription.
- Microphone authorization is checked before that field capture. Onboarding cannot finish permanently until microphone and field permissions have a usable path and the selected local or cloud transcription engine is ready.
- Password fields and controls marked as protected content are rejected before the microphone starts. Accessibility permission is required for shortcut-driven field tracking.
- macOS builds first attempt selection-only Accessibility insertion and confirm the exact inserted range or, where content readback is unsupported, synchronous setter success plus the exact new caret. Web, Electron, and other custom controls can use the guarded paste fallback described above only when the transcript has no carriage return or line feed. That refusal happens before any clipboard access and does not affect direct Accessibility or InputMethodKit insertion. The fallback retains at most 64 MiB of snapshot data, but AppKit materializes lazy pasteboard representations before Spick can apply that cap; it is not a streaming or pre-allocation limit. After checking the exact target again, Spick sends one PID-scoped paste and attempts restoration only if its change-count and ownership marker still match. If ownership is already lost it leaves the current clipboard alone, but macOS provides no atomic compare-and-swap for the final check and replacement, so a very small race remains. Selectionless controls cannot prove the original caret or selection; their dispatch is indeterminate and is never retried. Clipboard access can be denied or can prompt under macOS policy. A recoverable transcript remains in memory on Stats when a control is unsupported.
- macOS is the first validation target; Windows and Linux native integrations come later.
- The HUD is a compact-only `38×76` live-wave bar, vertical at the middle of either screen edge or horizontal above the Dock. Free drags snap to one of those three positions. On macOS it is converted once to a true nonactivating `NSPanel`; the pinned community integration and fail-closed fallback are documented in [docs/macos-hud.md](docs/macos-hud.md).
- `npm run build:macos:local` creates an ad-hoc signed `.app` and `.dmg` for testing on this Mac. `npm run build:macos:release` requires an installed Developer ID Application identity and notarization credentials, then produces and verifies the public artifacts.

The optional input-method design and development commands are documented in [docs/input-method.md](docs/input-method.md). The guarded paste fallback is best-effort because paste dispatch and clipboard restoration are not atomic on macOS; InputMethodKit compatibility work remains a separate experimental path.
