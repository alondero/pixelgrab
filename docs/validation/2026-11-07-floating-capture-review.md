# Floating capture workflow review — 2026-11-07

Review base: `2d5d3a8e6ca7a238bf53df6d644ade9c192e1b41`, including all local
tracked changes and the new native acceptance harness. The requested workflow
is capture → small desktop thumbnail → optional edit → drag into an agent.
Shottr's [start guide](https://shottr.cc/kb/startguide) describes preview quick
actions without forcing an editing session, and floating pinned references.
This change targets that interaction, not complete feature parity.

## Findings and changes

- Reproduced the production error `no frozen frame available for the requested
capture_id` after leaving an area capture open through the three-second
  display watcher interval. Layout invalidation discarded the frozen frame.
  Frozen pixels now survive layout polls and repeated non-terminal exports;
  the session releases them on completion, cancellation, or failed capture.
- Save As also consumed the frozen frame. Cancelling the dialog or saving and
  then committing now retains the source. Native dialogs have an owning
  window. Revision Save As uses the original crop and the current scene.
- The Windows clipboard adapter swapped red and blue before handing RGBA to
  arboard, which already performs the platform conversion. Removed that swap.
- The floating queue now gives more space to complete image previews and less
  to metadata. Editing gains Save As, Fit/100%/zoom controls, and wrapping tool
  groups. Completing or cancelling a revision closes the companion window.
- Loading/error states and Escape recovery replace an unresponsive capture
  canvas. Capture failure returns the session to Idle. Overlay reveal failure
  hides the partially revealed window before cancellation.
- Pins fill their actual viewport at Windows scaling and have the event
  capability required by dynamic pin windows.

## Verification completed

| Check                 | Result                                                                               |
| --------------------- | ------------------------------------------------------------------------------------ |
| `pnpm ci:check`       | Passed formatting, lint, Svelte/TypeScript checks, 216 tests                         |
| `pnpm ci:rust`        | Passed format, clippy, workspace tests and doc tests with the synthetic adapter      |
| `pnpm licenses:check` | Passed                                                                               |
| `pnpm tauri:build`    | Passed production executable and MSI/NSIS bundles                                    |
| Source review         | Independent standards and behavior reviews; actionable production findings corrected |

Regression tests failed before their fixes for frozen-frame retention through
layout invalidation/repeated exports, failed capture returning to Idle, and
canvas image error recovery. The clipboard regression verifies the actual
RGBA adapter contract without touching the operating-system clipboard in CI.

Native runs used the production executable, isolated profiles, real WebView2
IPC, a controlled coral desktop fixture, and a 3840 × 2160 display at 150%.
They exercised delayed selection, rectangle annotation, undo/redo, native
Save As cancellation and saving, commit, and visible floating delivery.
These actions progressed, but the final pixel assertion failed: the captured
background was entirely transparent. An independent xcap probe also returned
all-zero pixels, and Windows `Graphics.CopyFromScreen` returned black for the
same controlled fixture. This identifies an environmental capture problem;
its precise cause is not yet established. Display state was requested from
the user. No successful end-to-end acceptance is claimed.

## Outstanding native acceptance

The opt-in `tests/native/acceptance.mjs` now rejects an unreadable controlled
desktop before starting capture. When the display is available, run:

```powershell
node tests/native/acceptance.mjs F:/src/pixelgrab/target/release/pixelgrab.exe --real-desktop
```

The remaining assertions cover known captured pixels, exact clipboard/export
agreement, non-destructive revision editing, Save As in the editor, narrow
editor layout, subsequent hotkey/full-screen captures, native pin dismissal,
and a real OLE file drop into an external Windows receiver. These later stages
have not yet passed in this session. Specific agent applications, mixed-DPI
multi-monitor hardware, installer execution, and screen readers are also
unverified. The harness is not run in CI and does not mock production IPC.

Local pixel artifacts and logs remain ignored under `fixtures/local`. Test
processes are closed on failure and the desktop fixture has a timeout.
