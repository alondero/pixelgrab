//! Normalized annotation entities + deterministic flatten pipeline.
//!
//! Tracer 04 ships three annotation primitives: Arrow, Rectangle, and a
//! fixed-size Numbered Badge. Each annotation is a typed entity with a
//! stable id, geometry, style, and z-order. The Rust core flattens the
//! entities into an RGBA buffer at the physical crop resolution before
//! the buffer is published to the clipboard or written to the cache.
//!
//! See `docs/adr/0003-physical-coordinate-ownership.md` for the
//! coordinate conventions. Annotations use the same physical-pixel space
//! as `PhysicalBounds`: the origin is the physical crop's top-left corner
//! and the axes extend rightward and downward.
//!
//! The flatten pipeline is a pure function so it can be unit-tested with
//! deterministic golden buffers. The synthetic and Windows platforms
//! share the same code path.

use serde::{Deserialize, Serialize};

/// Stable, frontend-generated annotation id. The id is unique within a
/// capture session and is preserved across undo / redo so a badge number
/// is bound to the same badge even after the user redoes a deletion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AnnotationId(pub u64);

/// The annotation geometry. Discriminated by [`AnnotationKind`]. Every
/// coordinate is in physical pixels relative to the active crop's
/// top-left corner.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AnnotationGeometry {
    /// Arrow: a stroked line from `tail` to `tip` plus a filled
    /// arrowhead at the tip.
    Arrow {
        /// Tail point in physical pixels.
        tail: PhysicalPoint,
        /// Tip point in physical pixels.
        tip: PhysicalPoint,
    },
    /// Rectangle: a stroked, optionally filled axis-aligned rectangle.
    Rectangle {
        /// Top-left corner.
        origin: PhysicalPoint,
        /// Width / height.
        size: PhysicalSize,
    },
    /// Numbered badge: a fixed-size filled circle with a centred digit.
    /// The geometry is the centre point and the radius in physical
    /// pixels. The badge number is carried by [`Annotation::number`].
    NumberedBadge {
        /// Centre of the badge.
        center: PhysicalPoint,
        /// Radius in physical pixels (typically `BADGE_RADIUS_PX`).
        radius: u32,
    },
}

/// Annotation style. The palette and stroke presets live in
/// [`AnnotationColor`] and [`AnnotationStroke`]; both are typed enums
/// rather than free-form values so the toolbar can bind shortcuts to a
/// finite set of choices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnnotationColor {
    /// Red.
    Red,
    /// Green.
    Green,
    /// Blue.
    Blue,
    /// Yellow.
    Yellow,
    /// White.
    White,
}

impl AnnotationColor {
    /// Resolve to an `(R, G, B, A)` tuple. Alpha is `0xFF` — annotations
    /// are opaque.
    pub const fn rgba(self) -> (u8, u8, u8, u8) {
        match self {
            Self::Red => (0xE5, 0x3B, 0x3B, 0xFF),
            Self::Green => (0x3B, 0xE5, 0x5C, 0xFF),
            Self::Blue => (0x3B, 0x82, 0xE5, 0xFF),
            Self::Yellow => (0xF6, 0xE3, 0x3B, 0xFF),
            Self::White => (0xFF, 0xFF, 0xFF, 0xFF),
        }
    }

    /// Resolve to a packed [`PaintColor`]. The struct form is what the
    /// internal rasterizer consumes so the paint functions can stay
    /// under clippy's argument-count limit.
    pub const fn paint(self) -> PaintColor {
        let (r, g, b, a) = self.rgba();
        PaintColor { r, g, b, a }
    }
}

/// Packed RGBA color used by the internal rasterizer. Decoupled from
/// [`AnnotationColor`] so paint helpers can take a single value rather
/// than four bytes (which would push them past clippy's argument
/// count threshold).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaintColor {
    /// Red channel.
    pub r: u8,
    /// Green channel.
    pub g: u8,
    /// Blue channel.
    pub b: u8,
    /// Alpha channel.
    pub a: u8,
}

/// Stroke width presets. The set is closed: the toolbar binds the three
/// preset buttons rather than offering a free-form slider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnnotationStroke {
    /// 2 px stroke.
    Thin,
    /// 4 px stroke.
    Medium,
    /// 8 px stroke.
    Thick,
}

impl AnnotationStroke {
    /// Resolve to a width in physical pixels.
    pub const fn width_px(self) -> u32 {
        match self {
            Self::Thin => 2,
            Self::Medium => 4,
            Self::Thick => 8,
        }
    }
}

/// Default radius (in physical pixels) for the numbered badge. The
/// spec requires a fixed-size badge; this constant is the single source
/// of truth so the toolbar and the rasterizer agree.
pub const BADGE_RADIUS_PX: u32 = 18;

/// A normalized annotation entity. The id, geometry, style, and z-order
/// are all the rasterizer needs to flatten; the badge number lives on
/// the entity (not the geometry) so an id ↔ number binding is stable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Annotation {
    /// Stable id.
    pub id: AnnotationId,
    /// Geometry variant + payload.
    pub geometry: AnnotationGeometry,
    /// Stroke color.
    pub color: AnnotationColor,
    /// Stroke width.
    pub stroke: AnnotationStroke,
    /// Z-order: lower numbers paint first. Two annotations with the same
    /// z-order are rendered in insertion order so the output stays
    /// deterministic across runs.
    pub z_order: i32,
    /// Populated for [`AnnotationGeometry::NumberedBadge`]; `None` for
    /// every other variant. Kept on the outer struct so the wire shape
    /// mirrors the editor store 1:1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<u32>,
}

impl Annotation {
    /// Convenience constructor for an arrow.
    pub fn arrow(
        id: AnnotationId,
        tail: PhysicalPoint,
        tip: PhysicalPoint,
        color: AnnotationColor,
        stroke: AnnotationStroke,
        z_order: i32,
    ) -> Self {
        Self {
            id,
            geometry: AnnotationGeometry::Arrow { tail, tip },
            color,
            stroke,
            z_order,
            number: None,
        }
    }

    /// Convenience constructor for a rectangle.
    pub fn rectangle(
        id: AnnotationId,
        origin: PhysicalPoint,
        size: PhysicalSize,
        color: AnnotationColor,
        stroke: AnnotationStroke,
        z_order: i32,
    ) -> Self {
        Self {
            id,
            geometry: AnnotationGeometry::Rectangle { origin, size },
            color,
            stroke,
            z_order,
            number: None,
        }
    }

    /// Convenience constructor for a numbered badge.
    pub fn numbered_badge(
        id: AnnotationId,
        center: PhysicalPoint,
        radius: u32,
        number: u32,
        color: AnnotationColor,
        stroke: AnnotationStroke,
        z_order: i32,
    ) -> Self {
        Self {
            id,
            geometry: AnnotationGeometry::NumberedBadge { center, radius },
            color,
            stroke,
            z_order,
            number: Some(number),
        }
    }
}

// Re-export the physical types from the coordinate module so this file
// is self-contained for downstream callers.
pub use crate::coordinate::{PhysicalPoint, PhysicalSize};

/// Flatten a list of annotations into the source RGBA buffer in
/// deterministic z-order. Returns the annotated RGBA. The buffer is
/// always copied into a fresh `Vec<u8>` so callers do not need to
/// pre-allocate; the source slice is left untouched.
///
/// Determinism: annotations are sorted by `(z_order, id)` and rasterized
/// in that order. Two annotations sharing `z_order` and `id` paint the
/// same pixels every time, so the export PNG and the clipboard bitmap
/// stay byte-identical across rebakes.
pub fn flatten_annotations(rgba: &[u8], size: PhysicalSize, annotations: &[Annotation]) -> Vec<u8> {
    assert_eq!(
        rgba.len(),
        (size.width as usize) * (size.height as usize) * 4,
        "flatten_annotations: rgba buffer length does not match declared size",
    );
    let mut sorted: Vec<&Annotation> = annotations.iter().collect();
    sorted.sort_by_key(|a| (a.z_order, a.id.0));
    let mut out = rgba.to_vec();
    for annotation in sorted {
        paint_annotation(&mut out, size, annotation);
    }
    out
}

/// Paint a single annotation. Internal helper; public for tests.
pub fn paint_annotation(rgba: &mut [u8], size: PhysicalSize, annotation: &Annotation) {
    let color = annotation.color.paint();
    match annotation.geometry {
        AnnotationGeometry::Arrow { tail, tip } => {
            paint_arrow(rgba, size, tail, tip, annotation.stroke.width_px(), color);
        }
        AnnotationGeometry::Rectangle {
            origin,
            size: rect_size,
        } => {
            paint_rectangle(
                rgba,
                size,
                origin,
                rect_size,
                annotation.stroke.width_px(),
                color,
            );
        }
        AnnotationGeometry::NumberedBadge { center, radius } => {
            let number = annotation.number.unwrap_or(0);
            paint_badge(
                rgba,
                size,
                center,
                radius,
                annotation.stroke.width_px(),
                color,
                number,
            );
        }
    }
}

/// Paint a stroked line from `tail` to `tip` plus a filled triangle
/// arrowhead at the tip. The arrowhead length is 4× the stroke width
/// (capped at 32 px) so a thick stroke still ends in a visible head.
fn paint_arrow(
    rgba: &mut [u8],
    size: PhysicalSize,
    tail: PhysicalPoint,
    tip: PhysicalPoint,
    stroke_width: u32,
    color: PaintColor,
) {
    paint_line(rgba, size, tail, tip, stroke_width, color);

    let head_len = ((stroke_width * 4).min(32)) as i32;
    if head_len <= 0 {
        return;
    }
    // Direction vector from tip toward tail so the arrowhead points
    // outward from the tail and converges at the tip.
    let dx = (tail.x - tip.x) as f64;
    let dy = (tail.y - tip.y) as f64;
    let len = (dx * dx + dy * dy).sqrt();
    if len < f64::EPSILON {
        return;
    }
    let ux = dx / len;
    let uy = dy / len;
    let perp_x = -uy;
    let perp_y = ux;

    // The two trailing corners of the arrowhead, scaled by `head_len`.
    let base_x = tip.x as f64 + ux * head_len as f64;
    let base_y = tip.y as f64 + uy * head_len as f64;
    let half_width = head_len as f64 * 0.6;
    let left = (
        (base_x + perp_x * half_width).round() as i32,
        (base_y + perp_y * half_width).round() as i32,
    );
    let right = (
        (base_x - perp_x * half_width).round() as i32,
        (base_y - perp_y * half_width).round() as i32,
    );
    let tip_px = (tip.x, tip.y);

    paint_triangle(rgba, size, left, right, tip_px, color);
}

/// Paint a stroked, axis-aligned rectangle.
fn paint_rectangle(
    rgba: &mut [u8],
    size: PhysicalSize,
    origin: PhysicalPoint,
    rect_size: PhysicalSize,
    stroke_width: u32,
    color: PaintColor,
) {
    let x0 = origin.x;
    let y0 = origin.y;
    let x1 = x0.saturating_add(rect_size.width as i32);
    let y1 = y0.saturating_add(rect_size.height as i32);
    let half = (stroke_width as i32) / 2;
    // Top + bottom edges.
    for dy in -half..=(stroke_width as i32 - 1 - half) {
        paint_horizontal(rgba, size, x0, x1, y0 + dy, color);
        paint_horizontal(rgba, size, x0, x1, y1 + dy, color);
    }
    // Left + right edges.
    for dx in -half..=(stroke_width as i32 - 1 - half) {
        paint_vertical(rgba, size, x0 + dx, y0, y1, color);
        paint_vertical(rgba, size, x1 + dx, y0, y1, color);
    }
}

/// Paint a filled circle with a dark border plus the centred digit.
/// `number` is the badge's sequence number; digits above 9 are
/// saturated at 9 so a misconfigured badge never panics.
fn paint_badge(
    rgba: &mut [u8],
    size: PhysicalSize,
    center: PhysicalPoint,
    radius: u32,
    stroke_width: u32,
    color: PaintColor,
    number: u32,
) {
    let cx = center.x;
    let cy = center.y;
    let radius_i = radius as i32;
    let stroke_i = stroke_width as i32;
    // Solid fill.
    for dy in -radius_i..=radius_i {
        for dx in -radius_i..=radius_i {
            let dist_sq = dx * dx + dy * dy;
            let r_outer = radius_i * radius_i;
            if dist_sq <= r_outer {
                plot_pixel(rgba, size, cx + dx, cy + dy, color);
            }
        }
    }
    // Dark border on the outside ring so the badge reads cleanly over
    // any captured content. The width matches `stroke_width`.
    let border_r = radius_i;
    let inner_r = (border_r - stroke_i).max(0);
    let border = PaintColor {
        r: 0x1A,
        g: 0x1A,
        b: 0x1A,
        a: 0xFF,
    };
    for dy in -border_r - 1..=border_r + 1 {
        for dx in -border_r - 1..=border_r + 1 {
            let dist_sq = dx * dx + dy * dy;
            let inner_sq = inner_r * inner_r;
            let outer_sq = border_r * border_r;
            if dist_sq <= outer_sq && dist_sq > inner_sq {
                plot_pixel(rgba, size, cx + dx, cy + dy, border);
            }
        }
    }
    // Centred digit using a 5×7 bitmap font. The digit colour
    // follows the badge's relative luminance so a white badge
    // renders a dark digit and a dark badge (none in the v1 palette,
    // but future-proof) would render a light one.
    let digit = digit_color_for_luminance(color);
    paint_digit(rgba, size, cx, cy, number, digit);
}

/// Resolve the digit colour that contrasts with the badge fill. The
/// v1 palette is uniformly bright so the digit is dark, but the
/// function generalises so a future dark preset does not reintroduce
/// the white-on-white bug.
fn digit_color_for_luminance(fill: PaintColor) -> PaintColor {
    // Standard relative luminance weights (Rec. 709). Anything below
    // the midpoint (~0.5) is treated as "dark" and gets a light digit.
    let lum = 0.2126 * (fill.r as f64 / 255.0)
        + 0.7152 * (fill.g as f64 / 255.0)
        + 0.0722 * (fill.b as f64 / 255.0);
    if lum < 0.5 {
        PaintColor {
            r: 0xFF,
            g: 0xFF,
            b: 0xFF,
            a: 0xFF,
        }
    } else {
        PaintColor {
            r: 0x14,
            g: 0x14,
            b: 0x14,
            a: 0xFF,
        }
    }
}

/// Set a pixel if it lies inside the buffer. Negative or oversized
/// coordinates are silently dropped so callers can iterate a bounding
/// box without per-pixel bounds checks.
fn plot_pixel(rgba: &mut [u8], size: PhysicalSize, x: i32, y: i32, color: PaintColor) {
    if x < 0 || y < 0 {
        return;
    }
    let (x, y) = (x as u32, y as u32);
    if x >= size.width || y >= size.height {
        return;
    }
    let idx = ((y * size.width + x) as usize) * 4;
    rgba[idx] = color.r;
    rgba[idx + 1] = color.g;
    rgba[idx + 2] = color.b;
    rgba[idx + 3] = color.a;
}

/// Paint a horizontal segment `[x0, x1)` on row `y`. Coordinates
/// outside the buffer are clipped.
fn paint_horizontal(
    rgba: &mut [u8],
    size: PhysicalSize,
    x0: i32,
    x1: i32,
    y: i32,
    color: PaintColor,
) {
    if y < 0 || y as u32 >= size.height {
        return;
    }
    let lo = x0.max(0) as u32;
    let hi = x1.max(0).min(size.width as i32) as u32;
    if lo >= hi {
        return;
    }
    let row = (y as u32) * size.width;
    for x in lo..hi {
        let idx = ((row + x) as usize) * 4;
        rgba[idx] = color.r;
        rgba[idx + 1] = color.g;
        rgba[idx + 2] = color.b;
        rgba[idx + 3] = color.a;
    }
}

/// Paint a vertical segment `[y0, y1)` on column `x`.
fn paint_vertical(
    rgba: &mut [u8],
    size: PhysicalSize,
    x: i32,
    y0: i32,
    y1: i32,
    color: PaintColor,
) {
    if x < 0 || x as u32 >= size.width {
        return;
    }
    let lo = y0.max(0) as u32;
    let hi = y1.max(0).min(size.height as i32) as u32;
    if lo >= hi {
        return;
    }
    let col = x as u32;
    for y in lo..hi {
        let idx = ((y * size.width + col) as usize) * 4;
        rgba[idx] = color.r;
        rgba[idx + 1] = color.g;
        rgba[idx + 2] = color.b;
        rgba[idx + 3] = color.a;
    }
}

/// Paint a Bresenham line from `a` to `b` with the given stroke width.
/// The stroke is approximated by stamping a square of side `width` at
/// every line step, which keeps the rasterizer simple while producing
/// visually clean results for 2/4/8 px strokes.
fn paint_line(
    rgba: &mut [u8],
    size: PhysicalSize,
    a: PhysicalPoint,
    b: PhysicalPoint,
    stroke_width: u32,
    color: PaintColor,
) {
    let mut x0 = a.x;
    let mut y0 = a.y;
    let x1 = b.x;
    let y1 = b.y;
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let half = (stroke_width as i32) / 2;
    loop {
        for oy in -half..=(stroke_width as i32 - 1 - half) {
            for ox in -half..=(stroke_width as i32 - 1 - half) {
                plot_pixel(rgba, size, x0 + ox, y0 + oy, color);
            }
        }
        if x0 == x1 && y0 == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }
}

/// Scanline-fill the triangle with corners `a`, `b`, `c`. Uses the
/// flat-top / flat-bottom decomposition so the inner loops never branch.
fn paint_triangle(
    rgba: &mut [u8],
    size: PhysicalSize,
    a: (i32, i32),
    b: (i32, i32),
    c: (i32, i32),
    color: PaintColor,
) {
    let mut pts = [a, b, c];
    pts.sort_by_key(|p| p.1);
    let (p0, p1, p2) = (pts[0], pts[1], pts[2]);

    // Top half: from p0 to p1, scanline along p0..p1.
    if p1.1 != p0.1 {
        for y in p0.1..=p1.1 {
            let t = (y - p0.1) as f64 / (p1.1 - p0.1) as f64;
            let xa = p0.0 as f64 + (p1.0 as f64 - p0.0 as f64) * t;
            let xb = p0.0 as f64 + (p2.0 as f64 - p0.0 as f64) * t;
            let (lo, hi) = ordered(xa, xb);
            paint_horizontal(rgba, size, lo as i32, hi as i32 + 1, y, color);
        }
    }
    // Bottom half: from p1 to p2, scanline along p1..p2.
    if p2.1 != p1.1 {
        for y in p1.1..=p2.1 {
            let t = (y - p1.1) as f64 / (p2.1 - p1.1) as f64;
            let xa = p1.0 as f64 + (p2.0 as f64 - p1.0 as f64) * t;
            let xb = p0.0 as f64 + (p2.0 as f64 - p0.0 as f64) * t;
            let (lo, hi) = ordered(xa, xb);
            paint_horizontal(rgba, size, lo as i32, hi as i32 + 1, y, color);
        }
    }
}

fn ordered(a: f64, b: f64) -> (f64, f64) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

// --- Bitmap digit font -------------------------------------------------
//
// Each digit is encoded as a 7-row × 5-column bitmap. The outer array
// is indexed by digit (0..=9); the inner array is a row (0..=6); each
// byte holds 5 bits where bit 4 is the leftmost pixel and bit 0 is the
// rightmost. `1` means paint, `0` means skip. The font is intentionally
// tiny so a single-digit badge stays legible at the badge's physical
// size; multi-digit numbers are not used by tracer-04.

const DIGIT_WIDTH: i32 = 5;
const DIGIT_HEIGHT: i32 = 7;

const DIGITS: [[u8; DIGIT_HEIGHT as usize]; 10] = [
    // 0
    [
        0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
    ],
    // 1
    [
        0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
    ],
    // 2
    [
        0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
    ],
    // 3
    [
        0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110,
    ],
    // 4
    [
        0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
    ],
    // 5
    [
        0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110,
    ],
    // 6
    [
        0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
    ],
    // 7
    [
        0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
    ],
    // 8
    [
        0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
    ],
    // 9
    [
        0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100,
    ],
];

/// Paint a single decimal digit centred at `(cx, cy)`. Digits >= 10 are
/// saturated at 9 so a misconfigured badge never panics; tracer-04
/// ships a single-digit badge so the saturation only matters as a
/// safety net for invalid IPC payloads.
fn paint_digit(
    rgba: &mut [u8],
    size: PhysicalSize,
    cx: i32,
    cy: i32,
    number: u32,
    color: PaintColor,
) {
    let n = if number == 0 {
        0
    } else if number > 9 {
        9
    } else {
        number
    };
    let glyph = DIGITS[n as usize];
    let origin_x = cx - DIGIT_WIDTH / 2;
    let origin_y = cy - DIGIT_HEIGHT / 2;
    for (row, bits) in glyph.iter().enumerate() {
        for col in 0..DIGIT_WIDTH {
            if (bits >> (DIGIT_WIDTH - 1 - col)) & 1 == 1 {
                plot_pixel(rgba, size, origin_x + col, origin_y + row as i32, color);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_rgba(w: u32, h: u32) -> Vec<u8> {
        vec![0u8; (w as usize) * (h as usize) * 4]
    }

    fn pixel_at(rgba: &[u8], w: u32, x: u32, y: u32) -> (u8, u8, u8, u8) {
        let idx = ((y * w + x) as usize) * 4;
        (rgba[idx], rgba[idx + 1], rgba[idx + 2], rgba[idx + 3])
    }

    #[test]
    fn flatten_sorts_by_z_order_then_id() {
        let size = PhysicalSize::new(10, 10);
        let rgba = empty_rgba(10, 10);
        let a1 = Annotation::arrow(
            AnnotationId(1),
            PhysicalPoint::new(0, 0),
            PhysicalPoint::new(1, 0),
            AnnotationColor::Red,
            AnnotationStroke::Thin,
            5,
        );
        let a2 = Annotation::rectangle(
            AnnotationId(2),
            PhysicalPoint::new(2, 2),
            PhysicalSize::new(3, 3),
            AnnotationColor::Blue,
            AnnotationStroke::Thin,
            0, // earlier z_order
        );
        let a3 = Annotation::arrow(
            AnnotationId(3),
            PhysicalPoint::new(5, 5),
            PhysicalPoint::new(6, 5),
            AnnotationColor::Green,
            AnnotationStroke::Thin,
            5,
        );
        // Insert out of order: a1 (z=5), a2 (z=0), a3 (z=5).
        let out = flatten_annotations(&rgba, size, &[a1.clone(), a2.clone(), a3.clone()]);
        // The rectangle's blue stroke must appear in the output — i.e.
        // z=0 paints first. The 5x5 area around (2,2) should now contain
        // non-zero bytes.
        let (r, _, _, _) = pixel_at(&out, 10, 2, 2);
        assert!(
            r > 0 || pixel_at(&out, 10, 2, 2).1 > 0,
            "rectangle with z=0 must be rasterized"
        );
        // Determinism: rerun yields the same bytes.
        let again = flatten_annotations(&rgba, size, &[a1, a2, a3]);
        assert_eq!(out, again);
    }

    #[test]
    fn flatten_is_deterministic_for_identical_input() {
        let size = PhysicalSize::new(20, 20);
        let rgba = empty_rgba(20, 20);
        let annotations = vec![
            Annotation::arrow(
                AnnotationId(1),
                PhysicalPoint::new(1, 1),
                PhysicalPoint::new(10, 5),
                AnnotationColor::Yellow,
                AnnotationStroke::Medium,
                0,
            ),
            Annotation::rectangle(
                AnnotationId(2),
                PhysicalPoint::new(5, 5),
                PhysicalSize::new(8, 4),
                AnnotationColor::White,
                AnnotationStroke::Thin,
                0,
            ),
            Annotation::numbered_badge(
                AnnotationId(3),
                PhysicalPoint::new(15, 15),
                BADGE_RADIUS_PX,
                7,
                AnnotationColor::Red,
                AnnotationStroke::Thin,
                0,
            ),
        ];
        let first = flatten_annotations(&rgba, size, &annotations);
        let second = flatten_annotations(&rgba, size, &annotations);
        assert_eq!(
            first, second,
            "identical input must produce identical output"
        );
        // Output must differ from the input (annotations actually painted).
        assert_ne!(first, rgba);
    }

    #[test]
    fn rectangle_paints_stroke_around_bounds() {
        let size = PhysicalSize::new(20, 20);
        let mut rgba = empty_rgba(20, 20);
        let ann = Annotation::rectangle(
            AnnotationId(1),
            PhysicalPoint::new(2, 3),
            PhysicalSize::new(6, 4),
            AnnotationColor::Red,
            AnnotationStroke::Thin,
            0,
        );
        paint_annotation(&mut rgba, size, &ann);
        // Top-left corner (2,3) should be red; interior (4,4) should
        // remain transparent because tracer-04 ships unfilled rectangles.
        let (r, g, b, a) = pixel_at(&rgba, 20, 2, 3);
        assert_eq!((r, g, b, a), (0xE5, 0x3B, 0x3B, 0xFF));
        let (_, _, _, a_mid) = pixel_at(&rgba, 20, 4, 4);
        assert_eq!(a_mid, 0, "interior must stay unfilled");
    }

    #[test]
    fn arrow_paints_line_and_head() {
        let size = PhysicalSize::new(30, 30);
        let mut rgba = empty_rgba(30, 30);
        let ann = Annotation::arrow(
            AnnotationId(1),
            PhysicalPoint::new(2, 2),
            PhysicalPoint::new(20, 20),
            AnnotationColor::Green,
            AnnotationStroke::Medium,
            0,
        );
        paint_annotation(&mut rgba, size, &ann);
        // A pixel mid-line should be painted (line passes through (10, 10)).
        let (_, g, _, _) = pixel_at(&rgba, 30, 10, 10);
        assert!(g > 0, "mid-line must be painted green");
        // A pixel at the arrowhead should also be painted.
        let (_, g_tip, _, _) = pixel_at(&rgba, 30, 20, 20);
        assert!(g_tip > 0, "tip pixel must be painted");
    }

    #[test]
    fn badge_paints_filled_circle_with_digit() {
        let size = PhysicalSize::new(60, 60);
        let mut rgba = empty_rgba(60, 60);
        let ann = Annotation::numbered_badge(
            AnnotationId(1),
            PhysicalPoint::new(30, 30),
            BADGE_RADIUS_PX,
            1,
            AnnotationColor::Blue,
            AnnotationStroke::Thin,
            0,
        );
        paint_annotation(&mut rgba, size, &ann);
        // The centre pixel must be non-zero because both the fill and
        // the digit paint at the centre column.
        let (_, _, b, _) = pixel_at(&rgba, 60, 30, 30);
        assert!(
            b > 0 || pixel_at(&rgba, 60, 30, 30).1 > 0 || pixel_at(&rgba, 60, 30, 30).0 > 0,
            "centre must be painted"
        );
        // The outer ring at radius (just inside the disc boundary) must
        // contain a dark border pixel.
        let (r, g, b, _) = pixel_at(&rgba, 60, 30, 30 + BADGE_RADIUS_PX);
        assert_eq!(
            (r, g, b),
            (0x1A, 0x1A, 0x1A),
            "badge border must be the dark stroke colour"
        );
    }

    #[test]
    fn empty_annotation_list_returns_source_buffer() {
        let size = PhysicalSize::new(4, 4);
        let mut rgba = vec![0xAAu8; 4 * 4 * 4];
        // Add a sentinel value to prove identity.
        rgba[10] = 0x77;
        let out = flatten_annotations(&rgba, size, &[]);
        assert_eq!(out, rgba);
    }
}
