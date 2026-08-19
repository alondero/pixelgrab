# ADR-0003: Physical-coordinate ownership

## Status

Accepted (tracer-01). Extended in tracer-02 with the four conversion boundaries
introduced by the real Windows capture pipeline.

## Context

PixelGrab deals with multiple coordinate systems:

- **Client coordinates** — coordinates relative to the WebView's CSS
  origin. Used by Konva and the pointer event system.
- **Physical coordinates** — positions in actual desktop pixels. Always
  non-negative. The canonical wire format.
- **Virtual desktop coordinates** — the union of every monitor's
  framebuffer, including negative origins.
- **Capture buffer coordinates** — physical coordinates relative to the
  captured framebuffer's origin.
- **Export crop coordinates** — the physical coordinates of the final
  flattened PNG.

Without a deliberate ownership rule, these systems are frequently
confused, leading to misaligned captures, offset selections, and
unreadable exports.

## Decision

The Rust core is the canonical owner of every physical coordinate. The
frontend (Svelte + Konva) expresses selections in physical coordinates
and reports them back to the Rust core as `PhysicalBounds`. The Rust
core uses the same physical bounds to:

- Crop the captured framebuffer.
- Compute the export dimensions.
- Drive the cross-monitor logic.

Conversions only happen at the Rust / frontend boundary:

| From                | To          | Where                        |
| ------------------- | ----------- | ---------------------------- |
| Client (CSS pixels) | Physical    | Rust core, on overlay commit |
| Physical            | Capture buf | Rust core, on commit flatten |
| Capture buf         | Export      | Rust core, on commit flatten |
| Capture buf         | Physical    | Rust core, when cropping     |

The frontend never infers a physical coordinate from a CSS coordinate.
Every selection report is in physical coordinates, and the overlay
recomputes the on-screen crop from the physical bounds it received.

### Conversion boundaries (tracer-02)

1. **Client → Physical (`client_to_physical`).** The overlay drags a
   rectangle in CSS pixels; on commit, the Rust core converts to
   physical pixels using `capture_bounds.size / stage_size` as the
   scale factor and `capture_bounds.origin` as the translation. Both
   axes use `round-half-away-from-zero` so a click that lands on a
   half-pixel rounds consistently.
2. **Physical → Capture buffer (`physical_to_capture_buffer`).** The
   commit pipeline translates the physical crop into the frozen
   framebuffer's local coordinate space by subtracting the capture
   bounds origin. The result is clamped to zero so a crop that lies
   before the capture origin cannot produce a negative offset.
3. **Capture buffer → Export (`clamp_to_capture_buffer`).** The final
   clamp ensures the export crop stays within the captured framebuffer
   extents. This is the last guard before the PNG and bitmap clipboard
   representations are encoded.
4. **Capture buffer → Physical (crop extract).** When the Windows
   `FrozenFrame::crop` reads bytes out of the framebuffer, it returns
   the rectangle in physical coordinates by re-adding the capture
   bounds origin to the in-buffer coordinates.

### Rounding policy

All conversions round to the nearest pixel using
`f64::round`-away-from-zero semantics. NaN and non-finite inputs
collapse to zero so a bad transform cannot propagate a wildly
out-of-range coordinate downstream. The exact rules live in
`pixelgrab_contracts::coordinate::transform::round_to_i32` /
`round_to_u32`; both functions are unit-tested for the boundary
conditions (NaN, infinity, exact half, overflow).

## Consequences

### Positive

- The Rust core is the single source of truth for physical coordinates.
- Selections are stable across DPI changes.
- Multi-monitor layouts with negative origins are handled correctly.
- The four boundary functions cover every pixel-shuffling operation in
  the codebase, so auditing coordinate correctness reduces to reading
  one module.

### Negative

- The frontend must always report physical coordinates, even when the
  user is dragging in CSS pixels.
- Negligible: each conversion is a handful of arithmetic operations
  per commit.

### Trade-offs

- We accept a small conversion cost for correctness across DPI scales
  and negative-origin layouts.

## Alternatives

- **Frontend owns physical coordinates.** Rejected. The frontend does
  not know the monitor layout; it would have to ask the Rust core for
  every transform.
- **Single global scale factor.** Rejected. Per-monitor DPI is real
  and varies between monitors on the same machine.
- **WebView client coordinates are physical.** Rejected. The WebView
  uses CSS pixels, which scale with the browser zoom and DPI.
- **Inline per-call rounding.** Rejected. Spreads rounding policy
  across the codebase and makes consistency impossible to audit.
