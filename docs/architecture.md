# Spick Architecture

## Purpose

Spick is a cross-platform desktop dictation utility. A user holds a configurable shortcut, speaks, sees immediate listening feedback, and receives cleaned text at the caret in the application that was focused when dictation began.

The first supported platform is macOS. Windows and Linux reuse the product, audio, transcription, and storage layers while supplying their own shortcut, permission, window, and text-insertion implementations.

## Design principles

- Keep the time from shortcut press to visible feedback short and make every processing state explicit.
- Preserve the user's focused application; the dictation widget must not take keyboard focus.
- Separate speech transcription, text cleanup, language policy, and text insertion so each can evolve independently.
- Make local processing a complete mode, not merely a fallback for cloud processing.
- Send audio or text to a cloud provider only when the user's routing policy permits it.
- Keep provider-specific behavior behind capability-aware adapters.
- Store the least sensitive data needed for the selected features.

## System layers

### Tauri desktop shell

Tauri owns the application lifecycle and connects the shared interface to native capabilities. It coordinates the dashboard window, tray/menu-bar presence, floating widget, global shortcut registration, permission flows, and frontend-to-core commands and events.

The shell exposes stable application-level operations. Platform-specific details remain behind native seams rather than leaking into the React interface.

### React and TypeScript interface

React renders onboarding, the non-focusable dictation widget, settings, model management, provider setup, and usage statistics. It consumes state and progress events from the core and submits user intent back to it.

The interface does not directly capture audio, call speech providers, persist API keys, or inject text. It uses one restrained visual system across every surface: neutral surfaces, a single accent, common pill and control geometry, and consistent listening, processing, success, and error states.

### Rust application core

Rust coordinates shortcut sessions, microphone capture, voice activity detection, transcription, cleanup, language routing, model management, statistics, and text insertion. It also owns sensitive native integrations and the provider boundary.

The core is divided by responsibility rather than provider. Transcription and cleanup are separate engine categories, allowing combinations such as local transcription with local cleanup, local transcription with cloud cleanup, or cloud transcription with deterministic cleanup.

### Platform integrations

Each operating system supplies implementations for:

- global shortcut registration and press/release events;
- focused control discovery and focus validation;
- secure-field detection where the platform exposes it;
- caret or selection text insertion;
- microphone and accessibility permission checks;
- non-activating widget behavior; and
- native credential storage.

These integrations share application-level contracts but are tested independently on each platform.

## Dictation pipeline

One push-to-talk session follows this sequence:

1. On shortcut press, record the focused application/control identity and start microphone capture.
2. Show the listening widget without changing focus.
3. Apply voice activity detection and stream audio to the selected transcription engine when that engine supports streaming.
4. Publish partial transcripts for feedback without inserting unstable text into the target control.
5. On shortcut release, finalize transcription and apply the selected language policy.
6. Run deterministic normalization, followed by optional model-based cleanup according to the selected cleanup mode.
7. Confirm that the intended target is still valid and is not a secure field.
8. Insert the final text at the caret or replace the original selection.
9. Record non-sensitive session measurements and expose a recoverable error or copy action if insertion fails.

The user-visible session state is limited to idle, listening, transcribing, cleaning, inserting, success, and error. Cancellation can occur before insertion without modifying the target application.

## Transcription and cleanup engines

### Local engines

`whisper.cpp` is the initial local speech-to-text runtime. Multilingual behavior comes from the downloaded Whisper model: multilingual models support language detection or a fixed language, while `.en` variants are English-only. Model metadata must state language coverage, format, size, memory expectations, license, source, and checksum.

A local cleanup engine may be added independently. Deterministic cleanup remains available when no cleanup model is installed.

### Cloud engines

Cloud speech and cleanup services are integrated through adapters for configured providers such as OpenAI, Gemini, and xAI. A provider is selectable only for a role and language combination its adapter reports as supported.

Cloud fallback is never implicit in local-only mode. When fallback is enabled, the routing decision and the data that will leave the device must be clear in settings.

### Capability-aware adapters

Every adapter reports capabilities instead of being forced into a false common denominator. The core uses these declarations to validate settings and choose a valid pipeline.

Relevant capabilities include:

- batch and streaming transcription;
- partial transcript delivery;
- automatic language detection and explicit language hints;
- published language and script support;
- code-switching support;
- speech translation;
- vocabulary or prompt hints;
- cleanup or text-generation support;
- audio formats and session limits; and
- offline availability.

Provider results are normalized into shared transcript segments with timing, language, confidence when supplied, and final/partial status. Missing provider metadata remains unknown rather than being invented.

## Language policy

Spick keeps four concepts separate: spoken language, writing system, locale, and output language. This prevents transcription choices from being conflated with translation or formatting choices.

The supported policies are:

- **Auto:** detect the dominant language for the current dictation session.
- **Fixed:** pass one selected language to engines that accept a language hint.
- **Preferred:** constrain selection to a user-maintained set when the engine supports that behavior.
- **Mixed:** evaluate language at voice-activity phrase boundaries and preserve code-switched spans where the selected engine can do so reliably.
- **Translate:** transcribe the source and produce a chosen output language as a separate, explicit step.

Mixed-language routing uses stable phrase boundaries and confidence data when available. It does not promise word-level switching when an engine only identifies a dominant language. Cleanup instructions preserve language and script unless translation or transliteration was explicitly selected.

## Text-insertion seams

Text insertion is a platform capability with an ordered set of strategies and a shared safety contract.

- **macOS:** use Accessibility APIs when the focused control permits direct editing. Use a clipboard-and-paste fallback only after preserving clipboard contents and validating focus before insertion and restoration.
- **Windows:** use UI Automation for compatible controls, with a clipboard-and-paste fallback where required.
- **Linux:** use AT-SPI for accessible controls. X11 and Wayland behavior must be validated separately because desktop environments differ in global shortcut, window positioning, and synthetic input support.

The insertion layer refuses known secure/password controls, preserves an existing selection until commit, reports unsupported targets, and avoids inserting when focus has moved unexpectedly. Clipboard fallback must restore the previous clipboard without overwriting a newer user change.

## Privacy and security

- Raw audio is ephemeral by default and is not written to disk.
- Transcript history is off by default and requires explicit opt-in.
- Local-only mode blocks every cloud transcription, cleanup, translation, and fallback route.
- API keys are stored in the operating system credential store and are never exposed to the webview or written to the application database.
- Logs omit API keys, raw audio, clipboard contents, and transcript text by default.
- Provider requests contain only the data required for the selected operation.
- Secure fields are excluded from dictation and insertion.
- Model downloads use curated metadata and are verified against a published checksum before activation.
- Permission onboarding explains microphone and accessibility access separately and provides a path to revisit denied permissions.

## Data storage boundaries

| Data                                                                       | Storage                                             | Default lifetime                           |
| -------------------------------------------------------------------------- | --------------------------------------------------- | ------------------------------------------ |
| Preferences, language policies, provider selections, and hotkey            | Application settings store                          | Until changed or reset                     |
| Usage counts, voiced duration, WPM inputs, latency, and engine identifiers | Local SQLite database                               | Until history is cleared                   |
| Transcript history                                                         | Local SQLite database only when enabled             | User-configurable                          |
| API keys and provider secrets                                              | OS credential store                                 | Until removed                              |
| Downloaded model files and manifests                                       | Application model directory                         | Until removed                              |
| Raw microphone audio                                                       | Memory during the active session                    | Discarded after completion or cancellation |
| Clipboard snapshot used by fallback insertion                              | Memory during insertion                             | Discarded after restoration                |
| Diagnostic logs                                                            | Local log directory, with sensitive fields excluded | Bounded retention                          |

Statistics are derived locally. Speaking speed uses words and voiced duration rather than total wall-clock time; language-aware alternatives can be shown when whitespace-delimited word counts are not meaningful.

## Reliability boundaries

The core treats transcription success and insertion success as distinct outcomes. A valid transcript remains available to copy when the target application rejects insertion. Provider, model, microphone, permission, focus, and insertion errors have separate user-facing states so recovery does not require repeating a successful earlier stage unnecessarily.

Compatibility is verified against a maintained application matrix. Passing on one accessible text control does not imply support for all controls in the same application.
