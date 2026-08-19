# Accessibility

PixelGrab must be usable with the keyboard alone, must respect Windows text
scaling, must not rely on color for state, and must expose accessible names
for every interactive control. These expectations are enforced during
interaction tests and (where possible) by automated checks.

## Baseline requirements

Every interactive control MUST:

- Be reachable from a keyboard-only completion of the primary workflows
  (region capture, full-screen capture, edit, commit, shelf, drag, pin).
- Have a visible focus indicator when focused.
- Have an accessible name (`aria-label` or visible label text).
- Convey selected/active state through more than color alone — typically a
  border weight, fill, or shape change in addition to colour.
- Render at the default Windows text scaling of 100%, 125%, 150%, and 200%
  without overlap or clipped content.

The primary workflows are:

1. **Region capture** — tray or global shortcut → overlay → drag → Enter
   (commit) or Escape (cancel).
2. **Full-screen capture** — tray → Enter.
3. **Annotation** — toolbar shortcut selection (`A`, `R`, `T`, `B`, `N`,
   `V`) → drag → Enter.
4. **Shelf** — keyboard focus traversal to a card → `C` (copy) /
   `Ctrl+S` (save) / `P` (pin) / `Delete` (dismiss).
5. **Drag-out** — `Ctrl+X` from the focused card to copy the card payload
   to the system clipboard as a fallback to OLE drag.

## Accessibility properties of the v1 stack

- **Tauri shell** — the WebView inherits WebView2's accessibility tree. The
  application does not configure a custom focus-trap or intercept keyboard
  events at the OS level, so screen-reader and high-contrast announcements
  pass through unchanged.
- **Svelte components** — every button uses `<button>` rather than `<div>`,
  every icon-only control has either `aria-label` or a wrapping `<label>`,
  and the contextual toolbar uses `role="toolbar"` with
  `aria-orientation="horizontal"`.
- **Konva stage** — the canvas exposes its current selection as an
  `aria-live="polite"` region on a sibling element. The Konva scene itself
  is decorative — the canonical selection state lives in the Svelte store
  (`session.svelte.ts`) so a screen reader never has to interpret pixel
  layout from the canvas.
- **Escape behaviour** — Escape clears the active selection before
  dismissing the overlay. This protects a keyboard user from accidentally
  closing a session that contains a half-finished crop.
- **Tray menu** — every tray entry has a visible label that doubles as the
  accessible name. The hotkey hints show in the label so the user can
  reason about the shortcuts without hidden tooltips.

## Test seams

Accessibility expectations are enforced by the integration tests in two
places:

- `src/App.test.ts` asserts that every `<button>` in the main window has
  either text content or an `aria-label`. This is the first regression
  guard for the v1 surfaces.
- `src/lib/overlay/OverlayApp.svelte` is tested with `@testing-library/svelte`
  to confirm the selection bounding box reaches the live region.

Future tracers add per-tool assertions (text tool exposes both visible
label and `aria-label`, badge tool renders its number as accessible text,
etc.).

## Out of scope (today)

- Voice control — Windows Voice Access can drive the canvas toolbar via
  the keyboard layer described above, but we do not yet expose a dedicated
  Voice Access grammar.
- Custom screen-reader announcements for tray icon state changes — these
  arrive in tracer-15 once the accessibility test seam is hardened.
- High-contrast mode forced colours — placeholder asset — replaced when
  the designer hands off the v1 icon set.
