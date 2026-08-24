# Windows v1 gap review — 2026-08-22

This review compares the packaged-application wiring with the product map in
issue #1 and the v1 specification in issue #12. It supersedes the release-ready
conclusion in `v1-validation-record.md`: green unit and contract suites did not
exercise several production window and UI paths.

## Reproduced and repaired in this pass

- The configured overlay window loaded the default application entrypoint
  instead of `overlay.html`.
- The preallocated overlay mounted before a capture existed and was not told
  when a later capture became ready.
- The overlay used a fixed 1920 × 1080 stage, while annotation geometry is
  stored in physical capture pixels. This displaced pointer input and rendered
  arrows and other annotations at the wrong scale.
- Terminal commit and cancellation paths did not hide the TopMost overlay.
- Ctrl+C used the same shelf-and-clipboard intent as Enter instead of the
  specified clipboard-only intent.
- Shelf thumbnails depended on the optional global Tauri API and could receive
  an unconverted Windows path.
- A `shelf-cleared` event for one card discarded the entire frontend snapshot,
  hiding cards that remained in the queue.
- Shelf window geometry was not synchronized on every queue mutation, causing
  stale bounds and clipped cards.
- Negative physical origins were rejected even though they are valid virtual
  desktop coordinates.

Each repair has a deterministic regression test at the nearest available seam.
The packaged WebDriver test remains too shallow to prove the complete workflow.

## Release-blocking gaps still open

1. Shelf cards do not expose production Pin, Reopen/Edit, or native Drag
   interactions. The Rust contracts exist, but there is no complete user path.
2. `PinWindow.svelte` is not mounted in independent native TopMost windows.
3. Production pin and drag locks are not backed by the cache lock registry, so
   active assets do not yet have the protection promised by the spec.
4. Windows monitor discovery currently treats raw monitor bounds as the work
   area. Shelf placement can overlap a taskbar.
5. Display topology, resolution, scale, work-area, and taskbar changes are not
   wired to invalidate capture layout and reposition active windows.
6. Cursor-monitor targeting and the live placement preview are incomplete.
7. Mixed-DPI input still requires packaged hardware validation and a
   per-monitor mapping seam; the current WebView-to-framebuffer transform uses
   the overlay's global captured extent.
8. The frozen frame is carried as a full base64 data URL through IPC rather
   than the bounded local asset transport required by the architecture.
9. The packaged acceptance test only checks that a window has a string title.
   It does not drive capture, selection, annotation, commit, shelf, drag, pin,
   or reopen, so it cannot support a release sign-off.

## Required release evidence

Before issue #12 can be considered complete, the packaged Windows application
must be driven through each release-blocking path above on representative
single-monitor and mixed-DPI layouts. The acceptance test should assert visible
overlay content, physical-coordinate crop/annotation results, overlay cleanup,
clipboard and shelf outcomes, card actions, and native window lifecycle—not
only backend state or isolated component behavior.
