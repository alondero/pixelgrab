# ADR-0008 — Text, blur, and native Save As (tracer-05)

## Status

Accepted (tracer-05).

## Context

The tracer-04 annotation pipeline (Arrow / Rectangle / Numbered Badge)
ships the minimum v1 editor, but issue #17 calls for three more
primitives that close the gap between "annotate and copy" and a usable
labelled capture:

- A **Text** tool that lets the user type labels and see them render
  with wrapped lines and a contrast-aware solid plate so the glyphs
  stay legible over any underlying image.
- A **Blur** tool that obfuscates sensitive pixels — a privacy-safe
  redaction layer that survives every export path so the user can
  share a captured credential / API key / personal number without
  leaking it through the clipboard, the cache, or a Save As PNG.
- A **Ctrl+S** native Save As for the active session so the user
  can save the in-flight capture (with annotations + redactions) to
  a user-chosen path _before_ committing. The existing `commit(save_as:
true)` flag writes to the cache root via `platform.write_png` and
  never opens a dialog — a UX gap that tracer-05 closes.

The blur primitive is the load-bearing piece. The leak guard is
structural: a redaction that depends on the user remembering to apply
it (or that a downstream export path remembers to include it) is
defeated the first time someone forgets. The blur has to be enforced
by the flatten pipeline itself.

## Decision

### Annotation variants

`AnnotationGeometry` gains two new variants:

- `Text { origin, size, text }` — the user-authored label lives on
  the wire (so undo / redo preserves edits) and the rasterizer wraps
  at render time. The stroke preset drives plate padding (Thin=2,
  Medium=4, Thick=6 px) and glyph scale (1×, 2×, 3× the 5×7
  bitmap); the `AnnotationColor` is held on the entity but the
  contrast rule wins at render time.
- `Blur { origin, size, radius }` — `radius` is the box-blur
  half-extent. The colour / stroke fields are kept on the wire for
  shape uniformity but ignored by the rasterizer.

`AnnotationGeometry` is no longer `Copy` (the new `Text` variant
holds a `String`); callers that previously relied on value semantics
must clone explicitly. The `flatten_annotations` signature stays the
same — callers pass `&[u8]` and `&[Annotation]` exactly as before.

### Flatten pipeline

`flatten_annotations(rgba, size, annotations) -> Vec<u8>` keeps its
public shape but the _interior_ now tracks two buffers: the immutable
`src` slice (the frozen crop) and the running `out` buffer (a fresh
copy of `src`). `paint_annotation(src, dst, size, annotation)` is the
new internal entry point. Arrow / Rectangle / Numbered Badge ignore
`src`; **Blur reads from `src`** so the redaction samples the
unmodified source pixels; **Text reads from `src` for plate-contrast**
so the plate colour is chosen from the actual capture contents.

This is the leak-guard: every export path (clipboard, cache PNG, the
new Save As) routes through `flatten_annotations`, and the blur's
samples come from the immutable input — never from the in-flight
output buffer. A pipeline that forgets to flatten loses the blur
_and_ the export in the same step; the leak and the missing output
are the same defect.

Determinism is preserved: annotations are still sorted by
`(z_order, id)` and the rasterizer is pure (no platform state, no
clock, no RNG).

### Text rasterizer

A hand-rolled 5×7 ASCII bitmap font in
`crates/pixelgrab-contracts/src/annotation.rs::ASCII_GLYPHS` covers
printable ASCII 0x20..=0x7E. Characters outside that range render as
a space glyph so an out-of-range input never panics. The font mirrors
the existing 5×7 digit table; `paint_glyph` reuses the
`(bits >> (DIGIT_WIDTH - 1 - col)) & 1` convention.

Wrapping is greedy word-break with hard-break fallback. The plate
colour is chosen from the mean Rec. 709 luminance of the source pixels
under the box: bright source → white plate + dark glyph; dark source
→ black plate + bright glyph. Stroke presets scale the plate padding
and the glyph size (1× / 2× / 3×) so the editor exposes three text
sizes without introducing a free-form slider.

### Blur rasterizer

`paint_blur(src, dst, size, origin, blur_size, radius)` is a
hand-rolled box blur. For each output pixel inside the blur region,
it averages the R, G, B channels over the
`[x-radius, x+radius] × [y-radius, y+radius]` neighbourhood of `src`,
clamped to the buffer. Alpha is forced to `0xFF` (the post-flatten
buffer is opaque). No new dependency — `imageproc` is not in the
workspace, and the badge digit / arrow triangle code already
demonstrates that a hand-rolled rasterizer is the house style.

### Native Save As

New IPC: `save_capture_as(payload: SaveCaptureAsRequest) ->
IpcResponse<SaveCaptureAsResponse>`. Mirrors `save_shelf_card_as`
exactly:

- `DialogExt` + `add_filter("PNG image", &["png"])` +
  `set_file_name(&suggested)` + `spawn_blocking(blocking_save_file)`.
- Cancel returns `Ok(SaveCaptureAsResponse { path: None, png_bytes: 0 })`.
- The chosen path is returned in the **Ok variant only** — error
  paths never include the path, the user's chosen path is the
  _success_ result.
- All errors are categorical kind strings (`save_as_read_failed`,
  `save_as_invalid_target`, `save_as_write_failed`,
  `save_as_encode_*_failed`). The `io::Error`'s `Display` impl on
  Windows can include the absolute path that failed; we discard it
  per the privacy rule in AGENTS.md §9 / ADR-0007.

The flatten pipeline is the same one the commit pipeline uses
(`flatten_crop` → `flatten_annotations`), so blur / text / arrows /
rectangles / badges all land in the exported PNG via the same leak
guard.

Ctrl+S binds in `KonvaStage.handleKey` and calls a new `onSaveAs`
prop on `KonvaStage`; the `OverlayApp` host wires this to the IPC.

### Editor + shortcuts

`AnnotationToolbar` adds two tool buttons (T → text, B → blur). The
toolbar's existing colour / stroke selectors continue to drive the
text glyph colour family; the contrast rule decides the final glyph
colour at flatten time.

`KonvaStage` handles T and B in the same switch as A / R / N / V.
The text tool opens an HTML `<textarea>` overlay positioned at the
text-draft box on pointer-up; the overlay's `keydown` handler
dispatches Enter (commit, no Shift), Escape (cancel), and Shift+Enter
(newline). The blur tool is a rectangle-style drag (drag → area →
commit on pointer-up).

Both text and blur participate in the tracer-04 semantic undo /
redo: `commitDraft` / `commitText` push the _pre-mutation_ snapshot,
and `undo` / `redo` round-trip exactly as for arrows / rectangles.

## Consequences

- Two new annotation variants + two new tool buttons + two new
  shortcuts + one new shortcut for Save As.
- One new IPC (`save_capture_as`); the existing `commit(save_as)`
  flag is unchanged (still writes to the cache root).
- One new ADR number (0008) and a new AGENTS.md section (§17) so
  the navigation aid stays current.
- New tests on both sides:
  - Rust unit tests in `crates/pixelgrab-contracts/src/annotation.rs`:
    text wraps, plate contrast picks the right colour, blur clips
    to source bounds, blur leak guard removes the source pixels,
    blur samples from the immutable source (not the in-flight
    output), text + blur flatten is deterministic, both variants
    round-trip via JSON.
  - Rust integration tests in
    `src-tauri/tests/ipc_contracts.rs`: text + blur in
    `RequestCommitIntent`, `SaveCaptureAsRequest` round-trip.
  - Frontend tests in
    `src/lib/annotation/store.svelte.test.ts` and
    `src/lib/ipc/types.test.ts`: text / blur history semantics,
    wire-shape guards.

## Alternatives

- **Real font crate (fontdue / ab_glyph / rusttype).** Rejected
  for dependency footprint — a 5×7 hand-rolled font matches the
  existing badge-digit style and keeps the rasterizer dependency-
  free. A future tracer can swap in a real font if higher-quality
  text rendering becomes a product requirement.
- **Blur sampling from the in-flight output buffer.** Rejected
  because a late-stage pipeline that forgets to flatten (or runs
  after a different flatten) would defeat the redaction. The leak
  guard is structural — every export path inherits the redaction
  by construction.
- **Save As via `commit(save_as=true)`.** Rejected because that
  path uses `platform.write_png` (cache root), not the native
  dialog. The user's UX expectation is the platform's "Save As"
  sheet, not a silent write into the cache. Tracer-05 ships a
  separate IPC so the two paths can evolve independently.
- **Allow `Blur` to inherit z-order like every other annotation.**
  Rejected only as a _user_ requirement — a redaction painted
  _over_ another annotation would still leak the underlying pixels
  in the source, but the flatten pipeline guarantees the blur
  region is replaced with averaged source values before the
  annotation is composited. The structural guarantee is sufficient;
  no z-order restriction is needed.

## Supersedes

None. ADR-0003 (physical-coordinate ownership) and the tracer-04
flatten invariants are unchanged.
