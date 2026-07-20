# Spick Delivery Milestones

The plan delivers one useful macOS dictation path first, then expands language, provider, and dashboard capabilities before porting the native seams to Windows and Linux. Each milestone ends with a runnable build, focused verification, and a short record of completed work and known limitations.

## Milestone 0: Project foundation — complete

Establish the Tauri, React/TypeScript, and Rust workspace with formatting, linting, tests, application configuration, and a shared visual foundation. Define the dictation session states and the boundaries between the interface, core, providers, storage, and platform integrations.

Exit criteria:

- The desktop shell opens a dashboard and a non-activating widget in development.
- React receives revisioned dictation lifecycle events and live microphone-level events from Rust.
- The project has repeatable local checks and documented development commands.
- No secrets or generated model files are tracked by Git.

## Milestone 1: macOS offline vertical slice — in progress

Deliver the smallest complete path: hold a configurable global shortcut, capture microphone audio, show listening feedback, transcribe through one supported local `whisper.cpp` model, and insert final text into the control that was focused when dictation began.

Begin compatibility testing with native and browser editors. Permission onboarding covers microphone, Accessibility, and Input Monitoring access.

Current checkpoint:

- A bare-Option gesture supports tap/tap and hold/release modes without consuming normal Option chords. A temporary accelerator fallback remains available until Input Monitoring is granted.
- The non-activating HUD, bounded in-memory microphone capture, and cancellation path are working. When enabled, the HUD remains on the desktop between sessions and has persisted expanded/compact presentation plus movable, monitor-clamped coordinates.
- Onboarding owns the first microphone prompt before any external field is captured. A denied permission routes back to the macOS privacy pane, and an eight-second device-start watchdog recovers the exact session without disturbing a newer attempt.
- Curated models can be downloaded or cancelled, size-checked, SHA-256 verified, selected, removed, and loaded through a cached Metal-enabled `whisper.cpp` runtime. Trusted compatible GGML `.bin` models can also be imported through the native picker, copied into content-addressed app-local storage, inspected by the bundled runtime, selected, and removed.
- Auto and fixed language settings are saved natively, and incompatible model/language combinations are rejected before recording.
- The cleanup choice is saved natively and captured per session. As-transcribed output is the safe default; the opt-in local cleaner removes a reviewed, language-tagged set of standalone hesitation sounds across nine languages while preserving quoted uses, obvious word or code references, and unreviewed languages.
- Focused-field capture retries transient app gaps within a bounded snapshot deadline, falls back through the focused UI element, and performs exact-target revalidation and secure/protected-field preflight. Debug builds prefer element-addressed `AXSelectedText` replacement with exact range/caret confirmation; a one-shot, explicitly weaker target-PID event covers Notes-style controls without a setter. Structured copy recovery remains available.
- A universal InputMethodKit palette helper and versioned local protocol exercise arm, one-use insert, disarm, expiry, exact readback, and ambiguous-delivery outcomes. The desktop and helper mutually authenticate live audit-token-bound code identities. It remains the release candidate while application compatibility and final packaging work continue; the ordinary debug build uses the narrower Accessibility development path.
- Final transcripts are kept in memory and shown on Today for an explicit copy. Indeterminate future writes require a separate check-before-copy acknowledgement.
- First-run setup is persisted only after the selected local model is usable or the selected cloud provider has a saved credential.
- A debug-only, fixed-fixture harness now measures exact controls without initializing audio, Whisper, settings, or the dashboard. Its offline browser bench, read-only preflight, exact target-app constraints, pre-capture evidence journal, and separately hashed visual review make runs repeatable without storing user content. The prototype still needs three signed hands-on passes per catalog case and a nested/notarized distribution path before it can become the default text-input primitive. Whole-field Accessibility replacement was rejected because it can race and overwrite a concurrent keystroke.

Exit criteria:

- Shortcut press starts capture and shortcut release finalizes the session.
- The widget never takes focus from the target application.
- Final local transcription can be inserted in every supported test target.
- Cancellation changes no target text and raw audio is not retained.
- Unsupported and secure controls fail safely with a copyable transcript when appropriate.

## Milestone 2: macOS insertion and session hardening

Make the vertical slice dependable across common control types. Prove a native InputMethodKit insertion path, then evaluate a separately gated best-effort paste fallback. Keep exact focus revalidation, cancellation/write linearization, at-most-once delivery, and explicit recovery. Instrument the stages needed to diagnose perceived latency without logging dictated content.

Expand the compatibility matrix to include a native editor, browser text fields and editors, VS Code, a terminal, and one Electron communication app.

Exit criteria:

- The selected native insertion primitive and any best-effort fallback are independently tested and honestly labelled.
- Focus changes, clipboard races, permission denial, microphone loss, and empty speech have defined outcomes.
- Stage timings are visible in diagnostics without exposing transcript or audio content.
- Compatibility results identify supported controls and known limitations rather than claiming application-wide support.

Current development checkpoint: the Today view can expand the last processing attempt into microphone handoff, transcription, and text-handoff timings. These measurements stay in process memory, use monotonic elapsed time, and omit speech, transcripts, target-app names, model/provider identifiers, errors, paths, and wall-clock timestamps.

## Milestone 3: multilingual local models

Expand the curated model manager introduced in Milestone 1. Add disk-space preflight, RAM and speed guidance, model migration, and measured recommendations while keeping the distinction between multilingual and English-only models clear.

Expose Auto and Fixed language policies first. Add Preferred and Mixed policies only after phrase-level behavior is measurable in representative language pairs. Keep translation and transliteration explicit.

Exit criteria:

- Users can install, select, verify, and remove a supported local model.
- Invalid, incomplete, or incompatible downloads cannot become active.
- Language settings prevent incompatible model selections.
- Test fixtures cover multiple scripts, punctuation, and at least one code-switching scenario.

## Milestone 4: cleanup and cloud providers

Build on the initial as-transcribed and deterministic English cleanup choices with separate engine selectors and, only when it has a clear job, optional model-based rewriting. Add cloud adapters one provider at a time, beginning with a single speech service, and store its key in the macOS credential store.

Adapters declare streaming, language, translation, vocabulary, and cleanup capabilities. Local-only mode and per-role routing are enforced by the core.

Exit criteria:

- A user can understand which engine handles transcription and which handles cleanup.
- Unsupported provider, language, and mode combinations cannot be selected.
- API keys never enter application settings, SQLite, or logs; the transient entry field clears immediately and the native side never returns a saved key to the frontend.
- Local-only mode is covered by tests that reject cloud routing and fallback.
- As-transcribed mode is byte-for-byte unchanged, and filler removal preserves quoted or explicitly referenced uses.

## Milestone 5: dashboard and personalization

Complete onboarding and the four primary product areas: Today, Engines, Vocabulary, and Settings. Add local statistics for words dictated, voiced minutes, speaking speed, time saved, languages, engine usage, and processing latency. Add custom vocabulary and per-application formatting profiles without coupling them to a single provider.

Exit criteria:

- Statistics are derived from documented local measurements and can be cleared.
- Transcript history remains optional and separate from aggregate statistics.
- Users can change shortcut, microphone, widget position, language policy, engines, privacy mode, and credentials.
- Empty, loading, downloading, offline, permission, and error states use the same visual language as the widget.

## Milestone 6: macOS beta readiness

Harden startup, updates, model migration, crash recovery, power behavior, and long-running tray operation. Complete accessibility review, privacy disclosures, signed distribution, and notarization. Validate supported macOS versions and hardware/model combinations rather than inferring compatibility.

Exit criteria:

- A clean machine can install, onboard, dictate, update, and uninstall without development tools.
- Failure recovery does not lose user settings, credentials, or verified models.
- Privacy behavior matches product copy and provider routing choices.
- The supported application and hardware matrix is published with known limitations.

## Milestone 7: Windows port

Reuse the shared interface, core pipeline, provider adapters, model manager, and storage schema. Implement Windows global shortcuts, microphone permissions, non-activating widget behavior, credential storage, UI Automation insertion, and the required fallback path.

Exit criteria:

- The offline vertical slice passes on supported Windows versions in native, Chromium, Electron, and code-editor controls.
- Secure fields, focus movement, selection replacement, and clipboard restoration pass Windows-specific tests.
- Packaging, signing, updates, and model paths follow Windows conventions.
- Platform differences are reflected in onboarding and the compatibility matrix.

## Milestone 8: Linux port

Implement Linux audio, shortcuts, credential storage, widget behavior, and AT-SPI insertion. Treat X11 and Wayland as separate compatibility targets and select supported desktop environments from measured results.

Exit criteria:

- The offline vertical slice works on the explicitly supported display server and desktop combinations.
- Installation, permissions, shortcuts, model storage, and credential behavior are documented for each supported package format.
- Unsupported Wayland or compositor behavior is detected and explained without unsafe input workarounds.
- The Linux compatibility matrix distinguishes native, browser, Electron, and code-editor controls.

## Milestone discipline

For every milestone:

- keep the main branch runnable;
- commit cohesive changes with tests or verification notes;
- avoid introducing a provider or platform abstraction before one real implementation exercises it;
- update the compatibility matrix when a target is added or behavior changes; and
- report what was completed, what was verified, and what remains intentionally out of scope.
