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

1. On shortcut press, capture the focused application/control identity and reject unsupported or secure targets before starting microphone capture.
2. Show the listening widget without changing focus.
3. Apply voice activity detection and stream audio to the selected transcription engine when that engine supports streaming.
4. Publish partial transcripts for feedback without inserting unstable text into the target control.
5. On shortcut release, finalize transcription and apply the selected language policy.
6. Apply the cleanup choice captured when the session began. The current opt-in local cleaner only removes a few pause-marked English hesitation words when punctuation repair is unambiguous; choosing as-transcribed output leaves the recognizer result untouched.
7. Atomically claim the session for delivery, then confirm that the same target, focus, and selection are still valid and non-secure and that no observed change invalidated the target.
8. Insert the final text at the original caret or replace the original selection. Never retry automatically after a write may have occurred.
9. Record non-sensitive session measurements and expose a recoverable error or copy action if insertion fails.

The user-visible session state is limited to idle, listening, transcribing, cleaning, inserting, success, and error. Cancellation can win before the insertion claim. Once a future insertion begins, the result must be reported as inserted, failed, or indeterminate instead of claiming that nothing was typed.

The current checkpoint stops before step 8. It revalidates the captured target, retains the transcript in memory, and asks the user to copy it explicitly. Automatic mutation remains disabled until Spick has a native insertion primitive whose safety contract matches the claims above.

## Transcription and cleanup engines

### Local engines

`whisper.cpp` is the initial local speech-to-text runtime. Multilingual behavior comes from the downloaded Whisper model: multilingual models support language detection or a fixed language, while `.en` variants are English-only. Model metadata must state language coverage, format, size, memory expectations, license, source, and checksum.

The built-in `readable-v1` cleaner is local, deterministic, and opt-in. It removes pause-marked English “um”, “uh”, and “erm” outside quoted or explicitly referenced text, and only when adjacent punctuation can be repaired conservatively. Bare words and identifiers are treated as ambiguous and preserved. Unknown and non-English transcripts pass through byte-for-byte. Choosing no cleanup engine preserves the recognizer result, and changing the setting during a recording affects only the next session. When cleanup changes text, raw recognizer segments are discarded because their text and offsets no longer describe the delivered transcript.

Settings schema v2 records an explicit cleanup choice. The v1 migration turns cleanup off because older builds wrote `readable-v1` as a default before the cleaner was connected; that stored value cannot prove consent. The migration preserves the rest of the settings and keeps the original file as a backup.

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

- **macOS (current):** a private, one-use target token anchors the frontmost application, focused element, editable element, and original selection. Accessibility notifications permanently invalidate the token when focus, selection, value, application activity, or element lifetime changes. Capture accepts standard `AXTextField` and `AXTextArea` controls, blocks `AXSecureTextField` and `AXContainsProtectedContent`, and never retains the field’s contents. The checkpoint does not write to the target.
- **macOS (research):** prototype a bundled InputMethodKit client because `NSTextInputClient` exposes native insertion and replacement semantics. Whole-field `AXValue` replacement is rejected: Accessibility has no compare-and-set, so a concurrent keystroke could be overwritten.
- **macOS (experimental fallback):** a future clipboard-and-paste transaction must be serialized, local to the current host, and explicit about its weaker contract. Paste dispatch and clipboard restoration are not atomic; unconfirmed delivery is indeterminate and is never retried.
- **Windows:** use UI Automation for compatible controls, with a clipboard-and-paste fallback where required.
- **Linux:** use AT-SPI for accessible controls. X11 and Wayland behavior must be validated separately because desktop environments differ in global shortcut, window positioning, and synthetic input support.

The target layer refuses known secure/password/protected controls before audio capture, preserves the original selection until revalidation, reports unsupported targets, and detects focus or content movement. It does not retain field contents. Any later clipboard path may restore prior contents only while Spick can still prove ownership; it must never overwrite a newer user clipboard change.

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

The core treats transcription success and insertion success as distinct outcomes. A valid transcript remains available to copy when a non-secure target rejects insertion. Secure targets never produce a recoverable transcript. Provider, model, microphone, permission, focus, and insertion errors have separate user-facing states so recovery does not require repeating a successful earlier stage unnecessarily. An indeterminate result tells the user to inspect the field before copying and is never retried automatically.

Compatibility is verified against a maintained application matrix. Passing on one accessible text control does not imply support for all controls in the same application.
