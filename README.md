# Spick

Spick is a macOS-first desktop dictation utility for speaking into any text field. It is designed as a quiet utility: hold a shortcut, receive immediate visual feedback, and return clean text without leaving the application you are using.

The project has completed its foundation milestone and is entering the macOS offline vertical slice. It combines a Tauri 2 shell, a Rust application core, and a React/TypeScript interface. The native core now owns the global shortcut lifecycle and ephemeral microphone capture, while the HUD displays live audio level feedback. Dashboard statistics and model-management surfaces still use clearly labeled development data.

> Spick records microphone audio in memory during an active session, but it does not yet transcribe speech, call cloud providers, run `whisper.cpp`, or insert text into other applications. Do not treat the current build as a working dictation tool yet.

## Prerequisites

Development currently targets macOS. Install:

- Node.js 22 (the repository includes `.nvmrc`)
- Rust stable, including Cargo
- Xcode Command Line Tools (`xcode-select --install`)

## Run locally

```sh
npm install
npm run tauri dev
```

For frontend-only work, use `npm run dev`. The browser build cannot exercise Tauri commands or native window behavior.

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

- Usage data and transcript content are development scaffolding; native session states now reflect live capture.
- Microphone capture and a live level meter are implemented; voice activity detection, transcription, filler-word cleanup, and translation are not.
- Local model download and execution, cloud provider adapters, and API-key storage are not implemented.
- Cross-application focus tracking and text insertion are not implemented.
- macOS is the first validation target; Windows and Linux native integrations come later.
- The transparent HUD currently uses Tauri's macOS private API and therefore targets direct signed/notarized distribution; an App Store build would need an App-Store-safe window treatment.
- Packaging, signing, notarization, updates, and production accessibility review remain future work.

The next vertical slice connects the captured 16 kHz mono buffer to a local multilingual Whisper model, then adds safe text insertion into a measured set of macOS applications.
