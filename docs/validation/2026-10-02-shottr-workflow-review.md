# Shottr workflow review - 2026-10-02

## Scope and benchmark

Review base: `f9e0ca95658228400bf47b586a9e2ccf44d03820`. Scope includes all
tracked and new changes in this worktree. The user requested a complete app
review and implementation focused on quickly seeing freshly captured images
and dragging them into another application.

Shottr is a macOS benchmark. Its [start guide](https://shottr.cc/kb/startguide)
describes a preview with quick actions, reopening, and a draggable file icon
for sharing. Its [changelog](https://shottr.cc/newversion.html) documents keeping
previews until use/close and retaining dragged files after another capture for
shelf applications. The relevant target is low-friction capture, recognition,
and reuse. This review does not claim complete Shottr feature parity.

## Comparison and changes

| Workflow                      | PixelGrab at review base                                                                | Result of this change                                                                                               |
| ----------------------------- | --------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------- |
| Simple region screenshot      | Selection always enters editing and needs Done/Enter                                    | Release captures to clipboard and shelf; Ctrl at release or Edit before sharing keeps the editor open               |
| Full-screen screenshot        | Opens an overlay without selecting the whole screen                                     | Native operation captures the cursor's monitor and delivers immediately                                             |
| Recognise a fresh capture     | Small square preview crops the image; main window shows diagnostics                     | Complete image previews, dimensions, recent screenshots gallery, newest first                                       |
| Drag into another app         | Accepted drop can dismiss and delete the source file                                    | Capture remains reusable; native drag holds its file lock before reading assets and pauses expiry                   |
| Keep a screenshot available   | Timer enabled by default; disabled timer still expires                                  | New defaults retain until dismissed; disabled timers survive hover and long idle                                    |
| Open and edit                 | Metadata form; no visual editor; saved scene may be flattened twice                     | Actual canvas and annotation tools; immutable crop plus editable scene; new revision leaves original unchanged      |
| Failure and retry             | Revision failure can leave a held editor lock with an Idle session                      | Editor stays retryable; reveal failure releases resources; subsequent capture succeeds                              |
| Repeated captures and restart | Startup snapshots can overwrite new events; canvas can retain old crop                  | Subscribe before snapshot, preserve newer events, remount canvas per capture                                        |
| Display scaling               | Physical window sizes are smaller than logical cards at higher DPI; older captures clip | Scale card bounds, include overflow, reduce visible count to fit work area                                          |
| Window lifecycle              | Closing reusable windows can destroy later entrypoints                                  | Keep windows allocated; closing the editor releases its session                                                     |
| Keyboard use                  | Card shortcuts exist, but image opening and editing are awkward                         | Image button opens editor; focused card supports Enter/E, C, Ctrl+S, P, Delete; editor shortcuts ignore text inputs |

The existing shared flatten/export pipeline, clipboard, Save As, pins, tray,
hotkeys, and preference persistence remain the basis of the app. OCR, scrolling
capture, Shottr's measurement tools, and its broader editing features are
absent or outside this implementation. Native drops must still be checked in
the applications the user actually uses.

## Ownership and regression review

Standards and behavior were reviewed sequentially against CLAUDE.md, the
affected ADRs, and the user's capture-to-sharing requirement. Corrections
include native ownership of full-screen delivery and revision reveal,
shared native dispatch for tray/hotkey/secondary capture without a main WebView,
immutable source publication before the cache manifest, independent cache
consumer locks, retryable terminal states, and logical-to-physical shelf
placement. See [ADR-0012](../adr/0012-quick-capture-and-editable-sources.md).

The disabled timer regression failed before the fix. Native integration tests
exercise the actual capture, commit, revision, and drag owners with synthetic
pixels, isolated caches, and a mock window runtime. They cover failure after
resource acquisition, recovery, original-image preservation, repeated captures,
drag source retention, and safe editing when revision metadata is unsupported.
The annotation store test verifies restored IDs, independent objects, and undo.
Frontend entrypoint tests cover automatic commit, edit modifier, delayed startup
snapshots, accepted drop retention, and drag connection failure followed by retry.

Windows native-owner tests initially exited before main with
`STATUS_ENTRYPOINT_NOT_FOUND`: their Common Controls v6 imports lacked the
application's manifest. Embedding that dependency for Cargo test executables
allows these tests and the full workspace suite to run.

## Validation

| Check                                            | Result                                         | Evidence boundary                                                              |
| ------------------------------------------------ | ---------------------------------------------- | ------------------------------------------------------------------------------ |
| `pnpm install --frozen-lockfile`                 | Passed                                         | Existing locked dependencies installed; no dependency changes                  |
| `pnpm ci:check`                                  | Passed; 20 frontend files, 214 tests           | Agent infrastructure, format, lint, Svelte/TypeScript, component tests         |
| `pnpm ci:rust`                                   | Passed; 513 tests across workspace             | Rust format, clippy with `-D warnings`, synthetic unit/integration suites      |
| Native workflow integration                      | 9 tests passed                                 | Real native owners, synthetic capture, isolated cache, mocked window/event I/O |
| `pnpm licenses:check`                            | Passed; 30 dependencies                        | Repository license policy                                                      |
| `pnpm tauri:build`                               | Passed; production executable, MSI, NSIS       | Real Windows backend selected, latest source packaged                          |
| Production startup smoke                         | Passed; isolated profile, no capture requested | Process survives startup; no capture or drag acceptance claim                  |
| Browser workflow on production frontend assets   | Passed                                         | Real DOM/canvas gestures with synthetic PNG and mocked IPC                     |
| Final agent/format checks and `git diff --check` | Passed                                         | Documentation links, source formatting, whitespace                             |

Browser acceptance covered one drag invocation per gesture, retained images
after an accepted response, click-to-edit, drawing a rectangle, undo/redo and
exporting that scene, release-to-capture, Ctrl-release-to-edit, visible Done at
840 x 680, and no horizontal overflow at the minimum 320px width. A static
preview's missing favicon produced a 404; it did not affect these interactions.
These checks do not replace native target-app acceptance.

A parallel Rust build hit Windows paging-file error 1455 during linking.
Closing the owned browser/preview processes and rerunning with
`CARGO_BUILD_JOBS=1` passed the full source suite and final production build.
Lint retains six pre-existing console warnings in utility scripts; there are
no lint errors. Stable rustfmt reports unsupported nightly formatting options;
format checks and warning-free clippy pass.

All screenshots used for automated work contain generated fixture data, not
desktop or clipboard contents. No installation or release publication was
performed as part of validation.

## Remaining acceptance limits

- Packaged selection-to-drop acceptance in Explorer, Chromium/Electron, and
  the user's target applications is not established by synthetic tests.
- Real negative-origin and mixed-DPI capture, pin interaction, Windows text
  scaling, and screen-reader behavior require controlled hardware acceptance.
- Production compilation and startup do not prove capture or native drag.
- Retained shelf entries hold cache locks. Users must dismiss retained images
  to make them eligible for cache eviction; this is not an unlimited archive.
- Existing saved preferences are respected. Users with auto-dismiss enabled
  can turn it off in Settings to retain new and currently visible captures.

This review advances the capture/revision source implementation from the
[September review](2026-09-01-v1-workflow-review.md). It does not reinstate the
withdrawn v1 release sign-off or claim full Shottr parity.
