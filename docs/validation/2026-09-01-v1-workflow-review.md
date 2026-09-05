# Windows v1 workflow review — 2026-09-01

This review compares the current application with the product map in issue #1
and the v1 specification in issue #12. It follows the withdrawn sign-off in
[`v1-validation-record.md`](v1-validation-record.md) and the earlier
[`2026-08-22-v1-gap-review.md`](2026-08-22-v1-gap-review.md).

## Outcome

The reported capture-to-delivery path is now wired coherently:

1. Tray and secondary-launch capture intents leave PixelGrab's companion
   window hidden until the desktop has been frozen.
2. All eight crop handles are reachable, and Escape clears the frontend-owned
   crop before cancelling the session.
3. Enter commits to the clipboard and cache-backed shelf. Explicit Copy, Save,
   and Done controls expose the same keyboard actions to pointer users.
4. Commit failures are surfaced in the visible companion window instead of
   disappearing with the hidden overlay.
5. Shelf History shows the native shelf, and startup rehydration shows surviving
   cards without requiring a new capture.
6. Backend and frontend countdowns share an authoritative monotonic epoch.
7. Windows drag initializes OLE on the drag thread, advertises `CF_HDROP` with
   `TYMED_HGLOBAL`, advances and clones its format enumerator correctly, and
   marks its DIBV5 pixels as top-down with explicit sRGB metadata. COM object
   allocations now transfer to raw ownership and are released through their
   IUnknown vtables exactly once.
8. Focused shelf cards implement the documented Copy, Save As, Pin, and
   Dismiss shortcuts. The thumbnail labels its drag gesture and
   releases pointer capture on both completion and cancellation.
9. Tray and single-instance Shelf History actions call the native shelf
   presentation seam directly; the hidden companion WebView is no longer an
   intermediary for backend-to-backend work. Frontend expiry checks run once
   per second instead of allocating on every animation frame.
10. Re-cropping with Escape clears the prior annotation scene, and failed
    background captures report through the tray without stealing foreground
    focus.

Regression coverage was added at the Rust OLE seam and at the frontend shelf
clock, keyboard, and overlay IPC seams.

## Validation performed

| Gate                     | Result                                                                    |
| ------------------------ | ------------------------------------------------------------------------- |
| `pnpm ci:all`            | Green: Rust format, clippy, and tests; Svelte check; frontend tests; lint |
| Frontend suite           | 20 files, 206 tests passed                                                |
| Rust `pixelgrab` library | 199 tests passed, plus all workspace integration and contract suites      |
| `pnpm licenses:check`    | Green: 30 dependencies verified                                           |
| `pnpm tauri:build`       | Green: production executable plus MSI and NSIS bundles                    |

The build exercises the real Windows backend selection. Automated tests still
use the synthetic capture adapter and contain no real desktop pixels.

## Remaining v1 release blockers

Issue #12 should not be closed from this pass alone:

- The packaged WebDriver workflow still does not drive capture, selection,
  annotation, commit, visible shelf, drag, pin, or reopen end to end.
- Native drag now conforms to the OLE data-object contract, but Explorer,
  Chromium/Electron, and IDE drops still need packaged-app acceptance runs.
- Reopen/edit is not yet non-destructive: normal commits do not persist a usable
  immutable source plus vector scene, and revision commit can flatten onto an
  already-flattened PNG.
- Settings remain incomplete against the spec, notably drop-dismiss behaviour,
  storage statistics/clear controls, and complete named-monitor presentation.
- Mixed-DPI capture, pin interaction, and text-scaling accessibility need real
  packaged Windows validation on representative hardware.

The next release-hardening slice should first add a packaged synthetic
capture → selection → Enter → visible shelf acceptance test, then a controlled
Windows drag-target fixture. Reopen should be implemented only after defining
an immutable-source revision asset in the cache contract.
