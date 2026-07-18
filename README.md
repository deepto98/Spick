# Spick

Spick is a macOS-first desktop dictation app in active development. The goal is simple: hold a shortcut, speak, and put the result into the field you were already using. The current build records and transcribes locally; field insertion is next.

The project has completed its foundation milestone and is building the macOS offline vertical slice. It combines a Tauri 2 shell, a Rust application core, and a React/TypeScript interface. The native core owns the global shortcut, ephemeral microphone capture, verified model downloads, and in-process `whisper.cpp` transcription. Dashboard statistics still use clearly labelled development data.

> Spick can now record and transcribe with an installed local model. It does not yet insert the result into another application, apply the cleanup setting, or call a cloud provider, so it is not a complete dictation tool yet.

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

Read [the architecture](docs/architecture.md) for the planned pipeline, native seams, language policy, storage boundaries, and provider model. Delivery order and exit criteria live in [the milestone plan](docs/milestones.md).

## Privacy principles

- Local-only mode must never silently fall back to a cloud provider.
- Raw audio is ephemeral by default and transcript history is opt-in.
- API keys belong in the operating system credential store, never frontend state or ordinary settings files.
- Logs and statistics omit dictated text, raw audio, clipboard contents, and credentials by default.
- Secure fields and stale focus targets must fail safely instead of receiving inserted text.

These are architectural requirements. Features that enforce them will be verified as their milestones are implemented.

## Current limitations

- Usage statistics and saved transcript rows are development scaffolding. The latest real transcript is kept in memory only and can be copied from Today.
- Microphone capture, a live level meter, verified local model downloads, cancellation, and local `whisper.cpp` batch transcription are implemented. Voice activity detection, filler-word cleanup, and translation are not connected to the session pipeline yet.
- Cloud provider adapters and API-key storage are not implemented.
- Cross-application focus tracking and text insertion are not implemented.
- macOS is the first validation target; Windows and Linux native integrations come later.
- The transparent HUD currently uses Tauri's macOS private API and therefore targets direct signed/notarized distribution; an App Store build would need an App-Store-safe window treatment.
- Packaging, signing, notarization, updates, and production accessibility review remain future work.

The next vertical slice captures the target text field before recording, then inserts a successful local transcript through macOS Accessibility APIs with a guarded clipboard fallback.
