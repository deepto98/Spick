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
2. Show the widget in its opening-microphone state without changing focus.
3. Enter the listening state only after the selected native microphone stream reports that it started successfully.
4. Apply voice activity detection and stream audio to the selected transcription engine when that engine supports streaming.
5. Publish partial transcripts for feedback without inserting unstable text into the target control.
6. On shortcut release, finalize transcription and apply the selected language policy.
7. Apply the cleanup choice captured when the session began. The current opt-in local cleaner removes a reviewed language-specific list of standalone hesitation sounds while protecting quoted and explicitly referenced uses; choosing as-transcribed output leaves the recognizer result untouched.
8. Atomically claim the session for delivery, then confirm that the same target, focus, and selection are still valid and non-secure and that no observed change invalidated the target.
9. Insert the final text at the original caret or replace the original selection. Never retry automatically after a write may have occurred.
10. Record non-sensitive session measurements and expose a recoverable error or copy action if insertion fails.

The user-visible session state is limited to idle, opening the microphone, listening, transcribing, cleaning, inserting, success, and error. A session-bound readiness signal advances opening to listening only after the native stream starts; late signals cannot revive a cancelled or replaced session. Ready and failure messages share one ordered lifecycle channel handled away from the stream-owning thread, so UI work cannot stall audio draining. Cancellation can win before the insertion claim. Once a future insertion begins, the result must be reported as inserted, failed, or indeterminate instead of claiming that nothing was typed.

The processing worker also emits one optional, in-memory latency trace when it owns a completed or failed terminal transition. The trace uses monotonic elapsed durations for the stop-to-processing handoff, microphone finalization, the full transcription operation, and text delivery. It contains no speech or transcript content, target application, language, model/provider identity, path, error message, absolute timestamp, or audio samples, and it is never written to settings or SQLite. Cancelled and superseded workers emit nothing. The Today view listens for this event only in the main window and treats it as optional diagnostics rather than recorder state.

The development build now reaches step 9 through two debug-only paths. Controls with settable selected text receive an element-addressed selection replacement, followed by exact content readback or exact-caret confirmation when parameterized readback is unavailable. Controls such as some Notes versions can receive one target-PID Unicode event after a second exact target check. PID event delivery has an unavoidable focus micro-race because macOS does not bind it atomically to an Accessibility element; Spick therefore posts at most once and requires the original element to confirm the expected result. Release insertion remains gated while the InputMethodKit compatibility work continues.

## Transcription and cleanup engines

### Local engines

`whisper.cpp` is the initial local speech-to-text runtime. Multilingual behavior comes from the selected Whisper model: multilingual models support language detection or a fixed language, while `.en` variants are English-only. Curated downloads carry pinned source, license, size, and checksum metadata. A user import carries computed size/checksum and runtime-inspected language, family, and quantization metadata without inventing a source or license claim. Its original path is neither returned to the webview nor persisted. Imported weights are copied into content-addressed app-local storage before the bundled runtime checks them.

The built-in `readable-v1` cleaner is local, deterministic, and opt-in. It removes reviewed standalone hesitation sounds for English, Spanish, French, German, Hindi, Italian, Russian, Japanese, and Chinese outside quoted or explicitly referenced text. Obvious code, identifier, and word-reference uses remain intact. Missing, mismatched, unknown, and unreviewed language tags pass through byte-for-byte. Choosing no cleanup engine preserves the recognizer result, and changing the setting during a recording affects only the next session. When cleanup changes text, raw recognizer segments are discarded because their text and offsets no longer describe the delivered transcript.

Settings schema v5 records the explicit cleanup choice together with language routing, microphone selection, HUD presentation/position/visibility, privacy choices, and the shortcut. The v1 migration turns cleanup off because older builds wrote `readable-v1` as a default before the cleaner was connected; that stored value cannot prove consent. The v5 migration moves only an untouched bottom-center HUD preset to the bottom-right corner and preserves every dragged coordinate. Other migrations preserve explicit choices, add safe defaults for newly persisted controls, and keep the original file as a backup.

### Cloud engines

The development build has fixed batch adapters for OpenAI `gpt-4o-transcribe`, xAI Speech to Text, and experimental Gemini 3.5 Flash audio understanding. Credentials live in the OS credential store and are read only by the native transcription worker. Requests and raw provider responses are bounded and zeroed after use; errors omit provider bodies and credentials. Gemini uses the stable Interactions endpoint with server-side storage explicitly disabled.

Cloud fallback is never implicit in local-only mode. Its permission is captured when recording starts. After an eligible local runtime failure, the worker chooses the first configured provider compatible with the session’s language in the documented order and makes at most one upload; cancellation never triggers fallback. Settings disclose that enabled vocabulary hints may accompany audio and that an upload already in progress cannot be recalled.

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

- **macOS (current development build):** a private, one-use target token anchors the frontmost application, focused element, editable element, and original selection. Accessibility notifications invalidate stale targets when focus, selection, value, application activity, or element lifetime changes. Capture accepts standard text controls and custom controls with editable Accessibility capabilities, blocks `AXSecureTextField` and `AXContainsProtectedContent`, and never retains the field’s contents. Compatible controls receive an element-addressed selection write followed by exact range or caret confirmation. A debug-only, single-event PID fallback covers controls without that setter after immediate revalidation; it is explicitly weaker, never retried, and must confirm the original element after dispatch.
- **macOS (gated prototype):** a bundled palette input method can arm the exact `IMKTextInput` client active when recording begins, then consume that short-lived lease once. The helper rechecks the client identity, application, selection, marked-text state, and secure-input state before inserting. It reports success only when the caret moved as expected and the inserted range reads back exactly. The desktop and helper authenticate each live socket peer by audit token, exact signing ID, Apple Developer Team, hardened runtime, and a restrictive entitlement policy. The feature stays off in normal builds until control-level compatibility testing and final nested/notarized packaging are complete. Whole-field `AXValue` replacement remains rejected: Accessibility has no compare-and-set, so it could overwrite a concurrent keystroke.

The macOS compatibility executable is a distinct debug-only feature, not a runtime setting. It clears configured windows from the Tauri context before construction, omits all speech/model/settings state, consumes every shortcut event, and can submit only catalog fixtures to exact catalog bundle identifiers. The machine attempt, runner lifecycle, and visual review are separate immutable records; none contains target content or free-form text. This keeps measurements reproducible without turning the harness into a second arbitrary text-injection surface.

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

| Data                                                                                | Storage                                             | Default lifetime                           |
| ----------------------------------------------------------------------------------- | --------------------------------------------------- | ------------------------------------------ |
| Preferences, language policies, provider selections, and hotkey                     | Application settings store                          | Until changed or reset                     |
| Usage counts, capture duration, word-count inputs, language, and engine identifiers | Native-owned local SQLite database                  | Until usage and history are cleared        |
| Transcript history                                                                  | Local SQLite database only when enabled             | User-configurable                          |
| Vocabulary phrases and inactive pronunciation/category metadata                     | Native-owned local SQLite database                  | Until removed                              |
| API keys and provider secrets                                                       | OS credential store                                 | Until removed                              |
| Downloaded model files and manifests                                                | Application model directory                         | Until removed                              |
| Raw microphone audio                                                                | Memory during the active session                    | Discarded after completion or cancellation |
| Clipboard snapshot used by fallback insertion                                       | Memory during insertion                             | Discarded after restoration                |
| Diagnostic logs                                                                     | Local log directory, with sensitive fields excluded | Bounded retention                          |

Statistics are derived locally. Speaking speed uses Unicode word boundaries and microphone capture duration; the API labels that duration basis explicitly so a future voice-activity metric cannot be confused with it. Each completed session has one UUID receipt, making retries idempotent without storing transcript text in aggregate tables. Private-text deletion uses SQLite secure deletion and a checked WAL truncation after commit; if another reader prevents physical cleanup, the clear result carries an explicit warning so the same clear can be retried.

## Reliability boundaries

The core treats transcription success and insertion success as distinct outcomes. A valid transcript remains available to copy when a non-secure target rejects insertion. Secure targets never produce a recoverable transcript. Provider, model, microphone, permission, focus, and insertion errors have separate user-facing states so recovery does not require repeating a successful earlier stage unnecessarily. An indeterminate result tells the user to inspect the field before copying and is never retried automatically.

Compatibility is verified against a maintained application matrix. Passing on one accessible text control does not imply support for all controls in the same application.
