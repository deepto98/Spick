# macOS dictation HUD

The development HUD is a native nonactivating `NSPanel`, not a normal Tauri
`NSWindow`. This distinction matters: Apple only honors
`NSWindowStyleMaskNonactivatingPanel` for `NSPanel` subclasses. It lets the HUD
receive mouse input without activating Spick or invalidating the text target
captured in another application.

## Native integration

Tauri 2 does not expose `NSPanel` construction. Spick therefore uses the
community `tauri-nspanel` crate on macOS, pinned to commit
`a3122e894383aa068ec5365a42994e3ac94ba1b6`. It is deliberately a target-only
Git dependency rather than a floating branch.

At startup, `hud.rs` creates one hidden, undecorated Tauri webview window,
converts it once to `SpickHudPanel`, and then shows it at the saved position
when the floating-widget preference is enabled. The panel is configured as:

- borderless plus `NonactivatingPanel`;
- unable to become key or main;
- floating at level 4;
- visible on all Spaces and alongside full-screen windows;
- retained for the process lifetime.

All AppKit panel operations run on the macOS main thread. Show, hide, and resize
use the panel handle. Dragging calls AppKit's `performWindowDragWithEvent:` on
that panel. Tauri remains responsible for its tested cross-monitor coordinate
conversion and frame-position queries.

Do not close the HUD, convert it back to a Tauri window, maximize it, or call
Tauri's focusability setters after conversion. The plugin changes the native
Objective-C class at runtime; those lifecycle paths can depend on Tao's original
window subclass and are outside Spick's supported use.

## Failure behavior

Panel conversion is an enhancement, not permission to risk the captured target.
If conversion is unavailable, the normal HUD becomes pointer-through while a
live insertion target exists. It may still animate, resize, and follow its saved
position, but it cannot be clicked or dragged until the target is committed,
discarded, or the HUD is hidden. Idle and transcript-copy states restore mouse
interaction.

## Upgrade and manual checks

The pinned dependency has no crates.io release and must not be upgraded as a
routine dependency refresh. Before changing its revision or Tauri/Tao versions:

1. Run `npm run check:all` and the native all-features test/clippy checks.
2. In Notes and a browser text field, start dictation and confirm Spick never
   becomes the frontmost application when the HUD appears, is clicked, is
   collapsed or expanded, or is dragged.
3. Confirm the original caret remains active and receives the transcript.
4. Repeat on multiple Spaces, a full-screen app, and mixed-DPI monitors.
5. Exercise at least 100 dictate/settle cycles plus repeated visibility toggles.
   The HUD must remain reusable and must never be closed or converted back to a
   window.

This is development scope. Release readiness requires the same matrix on every
supported macOS version and a fresh audit of the pinned plugin's open lifecycle
issues.
