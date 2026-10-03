# ADR-0012: Quick capture, reusable previews, and editable source assets

## Status

Accepted. Amends ADR-0005 (shelf), ADR-0006 (drag), ADR-0009 (revision),
and ADR-0011 (capture delivery and native window lifetime).

## Context

The Shottr workflow review found that a simple screenshot required entering
the annotation UI and confirming a second time. Recent images were hard to
recognise, sharing could delete an offered file before the receiving app read
it, and disabling expiry still expired cards. Revision metadata lacked a
corresponding immutable image, so removing annotations could not restore pixels.

## Decision

1. A new region selection commits to clipboard and shelf on mouse release.
   Holding Ctrl/Meta at release or choosing **Edit before sharing** retains the
   selection for annotation. Resizing an existing crop does not auto-commit.
   Full-screen capture selects the cursor's monitor, falls back to the primary
   monitor, and delivers directly in the native capture operation.
2. The native shelf shows complete image previews. The companion window shows
   recent screenshots, including overflow, with the same actions and event
   subscriptions. Listeners are registered before the startup snapshot; newer
   events cannot be overwritten by a delayed snapshot. Capture hides the
   companion and shelf before freezing the desktop and restores the shelf
   after cancellation or failure.
3. New preferences retain cards until dismissed. Existing preferences are
   respected. Disabling expiry clears existing deadlines as well as changing
   new ones. An infinite deadline survives hover/unhover and duplicate unhover
   notifications do not restart a timer.
4. Drag acquires the cache's Drag guard before resolving assets and pauses the
   native queue timer for the operation. The UI requests `dismissOnAccepted:
false` and never deletes a capture after a successful drop. The Shelf guard
   remains held for repeated sharing and targets that read files after OLE
   returns. User dismissal or configured expiry still controls shelf lifetime.
5. Editable commits publish `source.png` (immutable crop), `capture.png`
   (flattened export), metadata, and the vector scene before the manifest.
   Source bytes count toward cache usage. Revision commit flattens onto the
   source, publishes a new entry, and leaves the old entry unchanged. Missing
   source or unsupported/missing scene falls back to the flattened image with
   an empty editable scene. This preserves existing visible marks.
6. Native revision open owns lock acquisition, window reveal, and scene event
   delivery. Reveal failure rolls back the session and lock. Failed revision
   delivery retains the source lock and restores Reopening for retry/close.
   Shelf dismissal cannot release another consumer's Editor guard.
7. Main, overlay, and shelf windows are preallocated and survive OS close
   requests. Closing the companion cancels its open revision and clears the UI;
   closing is blocked during an in-flight revision commit. Settings
   cannot unmount an active editor. Each new capture remounts its canvas.
8. Card dimensions are WebView logical pixels; placement and native window
   bounds are physical pixels. Placement multiplies by monitor DPI, includes
   the overflow control, and reduces visible cards to fit the work area while
   retaining the others in overflow.

## Consequences

- A simple capture reaches sharing without a second confirmation. Editing is
  available before or after delivery.
- Shared screenshots remain available for reuse. Retained cards hold Shelf
  locks and are protected from cache eviction; dismissal releases them.
- Editable entries store two PNGs, increasing storage and encoding work.
  Existing flattened entries remain usable, but baked marks cannot be removed.
- Native workflow integration tests use synthetic pixels and MockRuntime for
  window/event I/O. Windows test binaries embed the Common Controls v6 manifest
  dependency because Cargo does not inherit the application's resource manifest.
- Synthetic and browser checks do not establish native OLE target compatibility,
  actual mixed-monitor capture accuracy, or Windows text-scaling acceptance.

## Alternatives

- Always open the editor: adds a confirmation to every simple screenshot.
- Delete a file when OLE reports success: breaks deferred reads and repeat use.
- Restore vectors over the flattened export: applies existing marks twice and
  cannot undo them.
- Use a hidden WebView to relay editor reveal: makes native delivery depend on
  frontend scheduling and listener availability.
- Keep physical window bounds equal to CSS card sizes: clips cards above 100% DPI.
