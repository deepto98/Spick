# Spick

Spick is a macOS-first desktop dictation utility for speaking into any text field. It is designed as a quiet utility: hold a shortcut, receive immediate visual feedback, and return clean text without leaving the application you are using.

The project is currently at the foundation milestone. It combines a Tauri 2 shell, a Rust application core, and a React/TypeScript interface. The current product surfaces and statistics use development data, while native commands model the dictation session lifecycle and floating HUD states. This makes the interaction and architecture testable before real audio and insertion are connected.

> Spick does not yet record or transcribe audio, call cloud providers, run `whisper.cpp`, or insert text into other applications. Do not treat the current build as a working dictation tool.

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
npm run check
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo test --manifest-path src-tauri/Cargo.toml
```

`npm run check` verifies formatting, linting, frontend tests, TypeScript, and the production web build.

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

- Dictation states and usage data are development scaffolding, not results from live speech.
- Microphone capture, voice activity detection, transcription, filler-word cleanup, and translation are not implemented.
- Local model download and execution, cloud provider adapters, and API-key storage are not implemented.
- Cross-application focus tracking and text insertion are not implemented.
- macOS is the first validation target; Windows and Linux native integrations come later.
- Packaging, signing, notarization, updates, and production accessibility review remain future work.

The next goal is the [macOS offline vertical slice](docs/milestones.md#milestone-1-macos-offline-vertical-slice): shortcut-to-audio capture, a local multilingual Whisper model, visible session progress, and safe text insertion into a measured set of applications.
