# ADR-0003: Physical-coordinate ownership

## Status

Accepted (tracer-01).

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

| From                | To          | Where                              |
| ------------------- | ----------- | ---------------------------------- |
| Client (CSS pixels) | Physical    | Rust core, on commit               |
| Physical            | Client      | Rust core, on overlay show         |
| Capture buffer      | Physical    | Rust core, when stitching monitors |
| Physical            | Export crop | Rust core, on commit               |

The frontend never infers a physical coordinate from a CSS coordinate.
Every selection report is in physical coordinates, and the overlay
recomputes the on-screen crop from the physical bounds it received.

## Consequences

### Positive

- The Rust core is the single source of truth for physical coordinates.
- Selections are stable across DPI changes.
- Multi-monitor layouts with negative origins are handled correctly.

### Negative

- The frontend must always report physical coordinates, even when the
  user is dragging in CSS pixels.
- Negligible: the conversion happens once per overlay show and once per
  commit.

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
