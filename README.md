# Spick

Spick is a macOS-first desktop dictation app in active development. The goal is simple: hold a shortcut, speak, and put the result into the field you were already using. The current build records and transcribes locally, tracks the field where the shortcut began, and keeps the result ready for an explicit copy.

The app combines a Tauri 2 shell, a Rust application core, and a React/TypeScript interface. The native core owns the global shortcut, ephemeral microphone capture, verified model downloads, in-process `whisper.cpp` transcription, and guarded macOS field tracking. Dashboard statistics still use clearly labelled development data.

> The shortcut path now works through local transcription and copy recovery for accessible macOS text fields and text areas. Automatic cross-app paste is deliberately off: macOS Accessibility does not offer an atomic replace-selection operation, and replacing a field’s whole value could erase a keystroke that arrives during the handoff.

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

Open **Engines** in the desktop build to download and select a local model. Downloads can be cancelled and are written to app-local data only after the declared byte length and SHA-256 both match.

Run the project checks before committing:

```sh
npm run check:all
```

`npm run check:all` verifies formatting, linting, frontend tests, TypeScript, the production web build, Rust tests, strict Clippy, and the native compile check. Use `npm run check` when you only need the frontend gate.

## Architecture

Spick keeps audio capture, transcription, cleanup, language policy, and text insertion as separate responsibilities. That boundary is intended to support local `whisper.cpp` models, capability-aware cloud providers, and platform-specific insertion without coupling the product interface to one engine.

Read [the architecture](docs/architecture.md) for the pipeline, native seams, language policy, storage boundaries, and provider model. Delivery order and exit criteria live in [the milestone plan](docs/milestones.md).

## Privacy principles

- Local-only mode must never silently fall back to a cloud provider.
- Raw audio is ephemeral by default and transcript history is opt-in.
- API keys belong in the operating system credential store, never frontend state or ordinary settings files.
- Logs and statistics omit dictated text, raw audio, clipboard contents, and credentials by default.
- Secure fields and stale focus targets must fail safely instead of receiving inserted text.

The current Mac path enforces these rules for local transcription and target tracking. It does not mutate another application yet. Future cloud and insertion paths must meet the same bar before they are enabled.

## Current limitations

- Usage statistics and saved transcript rows are development scaffolding. The latest real transcript is kept in memory only and can be copied from Today.
- Microphone capture, a live level meter, verified local model downloads, cancellation, and local `whisper.cpp` batch transcription are implemented. Voice activity detection, filler-word cleanup, and translation are not connected to the session pipeline yet.
- Cloud provider adapters and API-key storage are not implemented.
- Shortcut dictation captures an accessible Mac text field or text area before recording and checks the exact application, element, selection, protection state, and change notifications again after transcription.
- Password fields and controls marked as protected content are rejected before the microphone starts. Accessibility permission is required for shortcut-driven field tracking.
- Automatic insertion is not enabled. A valid transcript stays in memory on Today until the next recording, where the user can copy it explicitly. Custom controls that cannot be verified are rejected before recording.
- macOS is the first validation target; Windows and Linux native integrations come later.
- The transparent HUD currently uses Tauri's macOS private API and therefore targets direct signed/notarized distribution; an App Store build would need an App-Store-safe window treatment.
- Packaging, signing, notarization, updates, and production accessibility review remain future work.

The next insertion work starts with a native InputMethodKit proof of concept, because it exposes real text-input replacement semantics. A guarded paste experiment may follow, but it must be labelled best-effort: neither event delivery nor clipboard restoration is atomic on macOS.
