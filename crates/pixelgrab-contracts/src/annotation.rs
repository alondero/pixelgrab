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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// Text label inside a draggable box. The rasterizer wraps the
    /// text to fit within the box's width and paints a solid plate
    /// (auto-derived from the source region's mean luminance) so the
    /// glyph colour always reads cleanly.
    Text {
        /// Top-left of the text box, in physical pixels.
        origin: PhysicalPoint,
        /// Box extent. Wrapping happens at render time.
        size: PhysicalSize,
        /// User-authored text. Embedded `\n` is honoured; word-wrap
        /// happens at render time.
        text: String,
    },
    /// Blur/redaction. Rasterizer samples the **immutable source
    /// framebuffer** (not the in-flight output buffer) so the
    /// redaction cannot be defeated by reordering the z-order or
    /// rerunning the flatten pipeline without blur.
    Blur {
        /// Top-left of the blur region.
        origin: PhysicalPoint,
        /// Blur region extent.
        size: PhysicalSize,
        /// Half-extent of the box-blur kernel. The kernel covers
        /// `[x-radius, x+radius] × [y-radius, y+radius]`. A radius of
        /// 4 yields a 9×9 neighbourhood average.
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

    /// Convenience constructor for a text annotation.
    pub fn text(
        id: AnnotationId,
        origin: PhysicalPoint,
        size: PhysicalSize,
        text: String,
        color: AnnotationColor,
        stroke: AnnotationStroke,
        z_order: i32,
    ) -> Self {
        Self {
            id,
            geometry: AnnotationGeometry::Text { origin, size, text },
            color,
            stroke,
            z_order,
            number: None,
        }
    }

    /// Convenience constructor for a blur/redaction annotation.
    pub fn blur(
        id: AnnotationId,
        origin: PhysicalPoint,
        size: PhysicalSize,
        radius: u32,
        z_order: i32,
    ) -> Self {
        // Color + stroke are ignored by the blur rasterizer but kept
        // on the wire for shape uniformity with the other variants.
        Self {
            id,
            geometry: AnnotationGeometry::Blur {
                origin,
                size,
                radius,
            },
            color: AnnotationColor::White,
            stroke: AnnotationStroke::Medium,
            z_order,
            number: None,
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
///
/// Source vs. destination: the input slice is the *immutable* source
/// framebuffer (the frozen crop pixels). The output buffer is a fresh
/// copy of the source; paint operations read from `src` (e.g. blur
/// samples the original source, text samples source for plate
/// contrast) and write to `dst`. This split is what makes the blur
/// leak guard structural: a pipeline that forgets to flatten loses
/// the blur along with the export.
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
        paint_annotation(rgba, &mut out, size, annotation);
    }
    out
}

/// Paint a single annotation. The `src` slice is the immutable source
/// framebuffer; the `dst` slice is the running output buffer (already
/// a copy of `src`). Blur and Text read from `src`; arrow / rectangle /
/// badge write through to `dst` only.
///
/// The `src` and `dst` arguments may alias (e.g. a test that paints
/// directly without going through `flatten_annotations`); the arrow,
/// rectangle, and badge variants are agnostic to this.
pub fn paint_annotation(src: &[u8], dst: &mut [u8], size: PhysicalSize, annotation: &Annotation) {
    let color = annotation.color.paint();
    match annotation.geometry {
        AnnotationGeometry::Arrow { tail, tip } => {
            paint_arrow(dst, size, tail, tip, annotation.stroke.width_px(), color);
        }
        AnnotationGeometry::Rectangle {
            origin,
            size: rect_size,
        } => {
            paint_rectangle(
                dst,
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
                dst,
                size,
                center,
                radius,
                annotation.stroke.width_px(),
                color,
                number,
            );
        }
        AnnotationGeometry::Text {
            origin,
            size: box_size,
            ref text,
        } => {
            paint_text(
                src,
                dst,
                size,
                origin,
                box_size,
                text,
                color,
                annotation.stroke,
            );
        }
        AnnotationGeometry::Blur {
            origin,
            size: blur_size,
            radius,
        } => {
            paint_blur(src, dst, size, origin, blur_size, radius);
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

/// Blend the user's chosen colour into the contrast-determined glyph
/// base. The user-colour weight (60 %) is large enough to read as the
/// chosen hue but small enough that the contrast base still wins on
/// legibility. Used by `paint_text` so the 5-colour palette is
/// visible in the export without compromising readability.
fn tint_glyph(base: PaintColor, user: PaintColor) -> PaintColor {
    const USER_WEIGHT: f64 = 0.6;
    const BASE_WEIGHT: f64 = 1.0 - USER_WEIGHT;
    PaintColor {
        r: ((base.r as f64) * BASE_WEIGHT + (user.r as f64) * USER_WEIGHT) as u8,
        g: ((base.g as f64) * BASE_WEIGHT + (user.g as f64) * USER_WEIGHT) as u8,
        b: ((base.b as f64) * BASE_WEIGHT + (user.b as f64) * USER_WEIGHT) as u8,
        a: 0xFF,
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

// --- ASCII bitmap font (tracer-05) -----------------------------------
//
// Each glyph is encoded as a 7-row × 5-column bitmap, mirroring the
// digit font above. Index 0 = `0x20` (space); index 94 = `0x7E` (`~`).
// Bit 4 is the leftmost pixel and bit 0 is the rightmost; `1` paints.
// Characters outside 0x20..=0x7E render as a single blank glyph (space)
// so an out-of-range input never panics. The hand-rolled font matches
// the existing dependency-free rasterizer philosophy; adopting a real
// font crate would inflate the workspace footprint for a single label
// feature.

const ASCII_FIRST: u8 = 0x20;
const ASCII_LAST: u8 = 0x7E;
const ASCII_COUNT: usize = (ASCII_LAST - ASCII_FIRST + 1) as usize;

const ASCII_GLYPHS: [[u8; DIGIT_HEIGHT as usize]; ASCII_COUNT] = [
    // 0x20 ' ' — space
    [0, 0, 0, 0, 0, 0, 0],
    // 0x21 '!'
    [
        0b00100, 0b00100, 0b00100, 0b00100, 0b00000, 0b00100, 0b00000,
    ],
    // 0x22 '"'
    [
        0b01010, 0b01010, 0b01010, 0b00000, 0b00000, 0b00000, 0b00000,
    ],
    // 0x23 '#'
    [
        0b01010, 0b01010, 0b11111, 0b01010, 0b11111, 0b01010, 0b01010,
    ],
    // 0x24 '$'
    [
        0b00100, 0b01111, 0b10100, 0b01110, 0b00101, 0b11110, 0b00100,
    ],
    // 0x25 '%'
    [
        0b11000, 0b11001, 0b00010, 0b00100, 0b01000, 0b10011, 0b00011,
    ],
    // 0x26 '&'
    [
        0b01100, 0b10010, 0b10100, 0b01000, 0b10101, 0b10010, 0b01101,
    ],
    // 0x27 '\''
    [
        0b00100, 0b00100, 0b01000, 0b00000, 0b00000, 0b00000, 0b00000,
    ],
    // 0x28 '('
    [
        0b00010, 0b00100, 0b01000, 0b01000, 0b01000, 0b00100, 0b00010,
    ],
    // 0x29 ')'
    [
        0b01000, 0b00100, 0b00010, 0b00010, 0b00010, 0b00100, 0b01000,
    ],
    // 0x2A '*'
    [
        0b00000, 0b10101, 0b01110, 0b11111, 0b01110, 0b10101, 0b00000,
    ],
    // 0x2B '+'
    [
        0b00000, 0b00100, 0b00100, 0b11111, 0b00100, 0b00100, 0b00000,
    ],
    // 0x2C ','
    [
        0b00000, 0b00000, 0b00000, 0b00000, 0b00100, 0b00100, 0b01000,
    ],
    // 0x2D '-'
    [
        0b00000, 0b00000, 0b00000, 0b11111, 0b00000, 0b00000, 0b00000,
    ],
    // 0x2E '.'
    [
        0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00100, 0b00000,
    ],
    // 0x2F '/'
    [
        0b00001, 0b00010, 0b00010, 0b00100, 0b01000, 0b10000, 0b10000,
    ],
    // 0x30 '0'
    [
        0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
    ],
    // 0x31 '1'
    [
        0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
    ],
    // 0x32 '2'
    [
        0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
    ],
    // 0x33 '3'
    [
        0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110,
    ],
    // 0x34 '4'
    [
        0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
    ],
    // 0x35 '5'
    [
        0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110,
    ],
    // 0x36 '6'
    [
        0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
    ],
    // 0x37 '7'
    [
        0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
    ],
    // 0x38 '8'
    [
        0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
    ],
    // 0x39 '9'
    [
        0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100,
    ],
    // 0x3A ':'
    [
        0b00000, 0b00100, 0b00000, 0b00000, 0b00000, 0b00100, 0b00000,
    ],
    // 0x3B ';'
    [
        0b00000, 0b00100, 0b00000, 0b00000, 0b00100, 0b00100, 0b01000,
    ],
    // 0x3C '<'
    [
        0b00010, 0b00100, 0b01000, 0b10000, 0b01000, 0b00100, 0b00010,
    ],
    // 0x3D '='
    [
        0b00000, 0b00000, 0b11111, 0b00000, 0b11111, 0b00000, 0b00000,
    ],
    // 0x3E '>'
    [
        0b01000, 0b00100, 0b00010, 0b00001, 0b00010, 0b00100, 0b01000,
    ],
    // 0x3F '?'
    [
        0b01110, 0b10001, 0b00001, 0b00110, 0b00100, 0b00000, 0b00100,
    ],
    // 0x40 '@'
    [
        0b01110, 0b10001, 0b10111, 0b10101, 0b10111, 0b10000, 0b01111,
    ],
    // 0x41 'A'
    [
        0b00100, 0b01010, 0b01010, 0b10001, 0b11111, 0b10001, 0b10001,
    ],
    // 0x42 'B'
    [
        0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110,
    ],
    // 0x43 'C'
    [
        0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110,
    ],
    // 0x44 'D'
    [
        0b11100, 0b10010, 0b10001, 0b10001, 0b10001, 0b10010, 0b11100,
    ],
    // 0x45 'E'
    [
        0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111,
    ],
    // 0x46 'F'
    [
        0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000,
    ],
    // 0x47 'G'
    [
        0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01110,
    ],
    // 0x48 'H'
    [
        0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
    ],
    // 0x49 'I'
    [
        0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
    ],
    // 0x4A 'J'
    [
        0b00111, 0b00010, 0b00010, 0b00010, 0b00010, 0b10010, 0b01100,
    ],
    // 0x4B 'K'
    [
        0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001,
    ],
    // 0x4C 'L'
    [
        0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111,
    ],
    // 0x4D 'M'
    [
        0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001,
    ],
    // 0x4E 'N'
    [
        0b10001, 0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001,
    ],
    // 0x4F 'O'
    [
        0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
    ],
    // 0x50 'P'
    [
        0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000,
    ],
    // 0x51 'Q'
    [
        0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101,
    ],
    // 0x52 'R'
    [
        0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001,
    ],
    // 0x53 'S'
    [
        0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110,
    ],
    // 0x54 'T'
    [
        0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100,
    ],
    // 0x55 'U'
    [
        0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
    ],
    // 0x56 'V'
    [
        0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100,
    ],
    // 0x57 'W'
    [
        0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b10101, 0b01010,
    ],
    // 0x58 'X'
    [
        0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001,
    ],
    // 0x59 'Y'
    [
        0b10001, 0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100,
    ],
    // 0x5A 'Z'
    [
        0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111,
    ],
    // 0x5B '['
    [
        0b01110, 0b01000, 0b01000, 0b01000, 0b01000, 0b01000, 0b01110,
    ],
    // 0x5C '\'
    [
        0b10000, 0b10000, 0b01000, 0b00100, 0b00010, 0b00001, 0b00001,
    ],
    // 0x5D ']'
    [
        0b01110, 0b00010, 0b00010, 0b00010, 0b00010, 0b00010, 0b01110,
    ],
    // 0x5E '^'
    [
        0b00100, 0b01010, 0b10001, 0b00000, 0b00000, 0b00000, 0b00000,
    ],
    // 0x5F '_'
    [
        0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b11111,
    ],
    // 0x60 '`'
    [
        0b01000, 0b00100, 0b00010, 0b00000, 0b00000, 0b00000, 0b00000,
    ],
    // 0x61 'a'
    [
        0b00000, 0b00000, 0b01110, 0b00001, 0b01111, 0b10001, 0b01111,
    ],
    // 0x62 'b'
    [
        0b10000, 0b10000, 0b11110, 0b10001, 0b10001, 0b10001, 0b11110,
    ],
    // 0x63 'c'
    [
        0b00000, 0b00000, 0b01110, 0b10000, 0b10000, 0b10001, 0b01110,
    ],
    // 0x64 'd'
    [
        0b00001, 0b00001, 0b01111, 0b10001, 0b10001, 0b10001, 0b01111,
    ],
    // 0x65 'e'
    [
        0b00000, 0b00000, 0b01110, 0b10001, 0b11111, 0b10000, 0b01110,
    ],
    // 0x66 'f'
    [
        0b00110, 0b01001, 0b01000, 0b11110, 0b01000, 0b01000, 0b01000,
    ],
    // 0x67 'g'
    [
        0b00000, 0b01111, 0b10001, 0b10001, 0b01111, 0b00001, 0b01110,
    ],
    // 0x68 'h'
    [
        0b10000, 0b10000, 0b11110, 0b10001, 0b10001, 0b10001, 0b10001,
    ],
    // 0x69 'i'
    [
        0b00100, 0b00000, 0b01100, 0b00100, 0b00100, 0b00100, 0b01110,
    ],
    // 0x6A 'j'
    [
        0b00010, 0b00000, 0b00110, 0b00010, 0b00010, 0b10010, 0b01100,
    ],
    // 0x6B 'k'
    [
        0b10000, 0b10000, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010,
    ],
    // 0x6C 'l'
    [
        0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
    ],
    // 0x6D 'm'
    [
        0b00000, 0b00000, 0b11010, 0b10101, 0b10101, 0b10101, 0b10101,
    ],
    // 0x6E 'n'
    [
        0b00000, 0b00000, 0b11110, 0b10001, 0b10001, 0b10001, 0b10001,
    ],
    // 0x6F 'o'
    [
        0b00000, 0b00000, 0b01110, 0b10001, 0b10001, 0b10001, 0b01110,
    ],
    // 0x70 'p'
    [
        0b00000, 0b00000, 0b11110, 0b10001, 0b11110, 0b10000, 0b10000,
    ],
    // 0x71 'q'
    [
        0b00000, 0b00000, 0b01111, 0b10001, 0b01111, 0b00001, 0b00001,
    ],
    // 0x72 'r'
    [
        0b00000, 0b00000, 0b10110, 0b11001, 0b10000, 0b10000, 0b10000,
    ],
    // 0x73 's'
    [
        0b00000, 0b00000, 0b01110, 0b10000, 0b01110, 0b00001, 0b11110,
    ],
    // 0x74 't'
    [
        0b01000, 0b01000, 0b11110, 0b01000, 0b01000, 0b01001, 0b00110,
    ],
    // 0x75 'u'
    [
        0b00000, 0b00000, 0b10001, 0b10001, 0b10001, 0b10011, 0b01101,
    ],
    // 0x76 'v'
    [
        0b00000, 0b00000, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100,
    ],
    // 0x77 'w'
    [
        0b00000, 0b00000, 0b10001, 0b10001, 0b10101, 0b10101, 0b01010,
    ],
    // 0x78 'x'
    [
        0b00000, 0b00000, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001,
    ],
    // 0x79 'y'
    [
        0b00000, 0b00000, 0b10001, 0b10001, 0b01111, 0b00001, 0b01110,
    ],
    // 0x7A 'z'
    [
        0b00000, 0b00000, 0b11111, 0b00010, 0b00100, 0b01000, 0b11111,
    ],
    // 0x7B '{'
    [
        0b00110, 0b00100, 0b00100, 0b01000, 0b00100, 0b00100, 0b00110,
    ],
    // 0x7C '|'
    [
        0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100,
    ],
    // 0x7D '}'
    [
        0b01100, 0b00100, 0b00100, 0b00010, 0b00100, 0b00100, 0b01100,
    ],
    // 0x7E '~'
    [
        0b01001, 0b10101, 0b10010, 0b00000, 0b00000, 0b00000, 0b00000,
    ],
];

/// Look up the bitmap for a printable ASCII byte. Bytes outside
/// 0x20..=0x7E map to the space glyph so unsupported characters
/// render as a visible gap rather than panicking.
fn ascii_glyph(byte: u8) -> [u8; DIGIT_HEIGHT as usize] {
    if !(ASCII_FIRST..=ASCII_LAST).contains(&byte) {
        return ASCII_GLYPHS[0]; // space
    }
    ASCII_GLYPHS[(byte - ASCII_FIRST) as usize]
}

/// Resolve a single glyph bitmap at `(cx, cy)` with the given colour.
/// Mirrors `paint_digit`'s structure so the digit / text code paths
/// share a single pixel-stamping convention.
fn paint_glyph(rgba: &mut [u8], size: PhysicalSize, cx: i32, cy: i32, byte: u8, color: PaintColor) {
    let glyph = ascii_glyph(byte);
    for (row, bits) in glyph.iter().enumerate() {
        for col in 0..DIGIT_WIDTH {
            if (bits >> (DIGIT_WIDTH - 1 - col)) & 1 == 1 {
                plot_pixel(rgba, size, cx + col, cy + row as i32, color);
            }
        }
    }
}

/// Stroke-driven text geometry. The preset drives both the plate
/// padding (in pixels around the glyphs) and the glyph scale (each
/// 5×7 glyph rendered at `scale` × scale). The values are pinned so
/// the rasterizer output is deterministic regardless of the host's
/// font rendering.
fn text_padding(stroke: AnnotationStroke) -> u32 {
    match stroke {
        AnnotationStroke::Thin => 2,
        AnnotationStroke::Medium => 4,
        AnnotationStroke::Thick => 6,
    }
}

fn text_scale(stroke: AnnotationStroke) -> u32 {
    match stroke {
        AnnotationStroke::Thin => 1,
        AnnotationStroke::Medium => 2,
        AnnotationStroke::Thick => 3,
    }
}

/// Compute the mean Rec. 709 luminance of the `src` pixels inside the
/// plate rectangle (clipped to the buffer). Returns `None` when the
/// rectangle contains no in-bounds pixels. Used by `paint_text` to
/// pick a contrasting plate colour.
fn plate_source_luminance(
    src: &[u8],
    size: PhysicalSize,
    origin: PhysicalPoint,
    box_size: PhysicalSize,
) -> Option<f64> {
    let x0 = origin.x.max(0);
    let y0 = origin.y.max(0);
    let x1 = (origin.x + box_size.width as i32).min(size.width as i32);
    let y1 = (origin.y + box_size.height as i32).min(size.height as i32);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let mut sum = 0.0_f64;
    let mut count = 0_u64;
    for y in y0..y1 {
        for x in x0..x1 {
            let idx = ((y as u32 * size.width + x as u32) as usize) * 4;
            let r = src[idx] as f64 / 255.0;
            let g = src[idx + 1] as f64 / 255.0;
            let b = src[idx + 2] as f64 / 255.0;
            sum += 0.2126 * r + 0.7152 * g + 0.0722 * b;
            count += 1;
        }
    }
    if count == 0 {
        None
    } else {
        Some(sum / count as f64)
    }
}

/// Wrap a string into lines that each fit within `max_width` physical
/// pixels given a `cell_width` (the rendered width of one glyph cell
/// at the active stroke scale). Breaks on word boundaries; falls back
/// to hard-break for words that exceed a single line.
fn wrap_text(text: &str, cell_width: u32, max_width: u32) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }
    // Pre-split on explicit newlines so a user-entered Enter keeps
    // its meaning; then wrap each resulting paragraph independently.
    let mut lines: Vec<String> = Vec::new();
    for paragraph in text.split('\n') {
        if paragraph.is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut current = String::new();
        for word in paragraph.split(' ') {
            if current.is_empty() {
                current.push_str(word);
                continue;
            }
            let candidate_len = current.len() + 1 + word.len(); // space + word
                                                                // Each ASCII char is one cell; wider Unicode glyphs would
                                                                // round-trip via the missing-glyph path so we don't need
                                                                // to weight them. Length in chars is the width in cells.
            if (candidate_len as u32) * cell_width <= max_width {
                current.push(' ');
                current.push_str(word);
            } else {
                if !current.is_empty() {
                    lines.push(std::mem::take(&mut current));
                }
                // Word too long for the box: hard-break at cell width.
                if (word.len() as u32) * cell_width > max_width {
                    let mut chunk = String::new();
                    for ch in word.chars() {
                        if (chunk.len() as u32 + 1) * cell_width > max_width && !chunk.is_empty() {
                            lines.push(std::mem::take(&mut chunk));
                        }
                        chunk.push(ch);
                    }
                    current.push_str(&chunk);
                } else {
                    current.push_str(word);
                }
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// Paint the text annotation. Steps:
///   1. Derive plate padding and glyph scale from the stroke preset.
///   2. Word-wrap the text so each line fits inside the plate width.
///   3. Sample the source pixels under the plate; compute mean
///      luminance. Bright source → white plate + dark glyph; dark
///      source → black plate + bright glyph.
///   4. Paint the plate as a solid rectangle on `dst`.
///   5. Paint each glyph cell from the ASCII bitmap font.
fn paint_text(
    src: &[u8],
    dst: &mut [u8],
    size: PhysicalSize,
    origin: PhysicalPoint,
    box_size: PhysicalSize,
    text: &str,
    color: PaintColor,
    stroke: AnnotationStroke,
) {
    let padding = text_padding(stroke);
    let scale = text_scale(stroke);
    let cell_w = (DIGIT_WIDTH + 1) * scale as i32; // 5 glyph + 1 px spacing
    let cell_h = (DIGIT_HEIGHT + 1) * scale as i32; // 7 glyph + 1 px row spacing
    let plate_x0 = origin.x;
    let plate_y0 = origin.y;
    let plate_w = box_size.width as i32;
    let plate_h = box_size.height as i32;
    let inner_w = (plate_w - 2 * padding as i32).max(0);
    let inner_h = (plate_h - 2 * padding as i32).max(0);
    if inner_w <= 0 || inner_h <= 0 {
        return;
    }
    // Step 3: source luminance + plate / glyph colour.
    let luminance = plate_source_luminance(
        src,
        size,
        origin,
        PhysicalSize::new(plate_w.max(0) as u32, plate_h.max(0) as u32),
    )
    .unwrap_or(0.5);
    // The plate is the contrast-determined base (white over a bright
    // source, near-black over a dark source). The glyph colour blends
    // the contrast-determined base with the user's chosen
    // `AnnotationColor` so the palette choice is visible in the
    // export while the contrast rule still keeps the text legible.
    // The user-colour weight (60 %) is large enough to read as the
    // chosen hue but small enough that the contrast base still wins
    // on legibility.
    let (plate, base_glyph) = if luminance >= 0.5 {
        (
            PaintColor {
                r: 0xFF,
                g: 0xFF,
                b: 0xFF,
                a: 0xFF,
            },
            PaintColor {
                r: 0x14,
                g: 0x14,
                b: 0x14,
                a: 0xFF,
            },
        )
    } else {
        (
            PaintColor {
                r: 0x14,
                g: 0x14,
                b: 0x14,
                a: 0xFF,
            },
            PaintColor {
                r: 0xFF,
                g: 0xFF,
                b: 0xFF,
                a: 0xFF,
            },
        )
    };
    let glyph_color = tint_glyph(base_glyph, color);
    // Step 4: solid plate.
    paint_filled_rect(
        dst,
        size,
        PhysicalPoint::new(plate_x0, plate_y0),
        PhysicalSize::new(plate_w.max(0) as u32, plate_h.max(0) as u32),
        plate,
    );
    // Step 5: wrap + render glyphs.
    let lines = wrap_text(text, cell_w as u32, inner_w as u32);
    let mut cursor_y = plate_y0 + padding as i32;
    for line in lines {
        if cursor_y + DIGIT_HEIGHT * scale as i32 > plate_y0 + plate_h {
            break;
        }
        let mut cursor_x = plate_x0 + padding as i32;
        for byte in line.bytes() {
            if cursor_x + DIGIT_WIDTH * scale as i32 > plate_x0 + plate_w {
                break;
            }
            if scale == 1 {
                paint_glyph(dst, size, cursor_x, cursor_y, byte, glyph_color);
            } else {
                // Multi-scale render: stamp the glyph at each pixel
                // of the scale × scale block to keep the rasterizer
                // dependency-free.
                let glyph = ascii_glyph(byte);
                for (row, bits) in glyph.iter().enumerate() {
                    for col in 0..DIGIT_WIDTH {
                        if (bits >> (DIGIT_WIDTH - 1 - col)) & 1 == 1 {
                            for sy in 0..scale as i32 {
                                for sx in 0..scale as i32 {
                                    plot_pixel(
                                        dst,
                                        size,
                                        cursor_x + col * scale as i32 + sx,
                                        cursor_y + row as i32 * scale as i32 + sy,
                                        glyph_color,
                                    );
                                }
                            }
                        }
                    }
                }
            }
            cursor_x += cell_w;
        }
        cursor_y += cell_h;
    }
}

/// Paint a solid filled rectangle on `dst`, clipped to the buffer.
/// Used by `paint_text` for the plate.
fn paint_filled_rect(
    rgba: &mut [u8],
    size: PhysicalSize,
    origin: PhysicalPoint,
    rect_size: PhysicalSize,
    color: PaintColor,
) {
    let x0 = origin.x.max(0) as u32;
    let y0 = origin.y.max(0) as u32;
    let x1 = (origin.x + rect_size.width as i32)
        .min(size.width as i32)
        .max(0) as u32;
    let y1 = (origin.y + rect_size.height as i32)
        .min(size.height as i32)
        .max(0) as u32;
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    for y in y0..y1 {
        paint_horizontal(rgba, size, x0 as i32, x1 as i32, y as i32, color);
    }
}

/// Box-blur rasterizer. Samples the **immutable source** slice so
/// the redaction cannot be defeated by reordering z-order or
/// rerunning the flatten pipeline without blur.
///
/// For each output pixel inside the blur region, average the R, G, B
/// channels over the `[x - radius, x + radius] × [y - radius, y + radius]`
/// neighbourhood of the source. The neighbourhood is clamped to the
/// buffer bounds so a blur region that extends past the edge averages
/// only the in-bounds pixels (no wrap-around). Alpha is forced to
/// `0xFF` so the output is opaque after the flatten.
fn paint_blur(
    src: &[u8],
    dst: &mut [u8],
    size: PhysicalSize,
    origin: PhysicalPoint,
    blur_size: PhysicalSize,
    radius: u32,
) {
    let x0 = origin.x.max(0);
    let y0 = origin.y.max(0);
    let x1 = (origin.x + blur_size.width as i32).min(size.width as i32);
    let y1 = (origin.y + blur_size.height as i32).min(size.height as i32);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let radius = radius as i32;
    for y in y0..y1 {
        for x in x0..x1 {
            let mut sum_r = 0_u32;
            let mut sum_g = 0_u32;
            let mut sum_b = 0_u32;
            let mut count = 0_u32;
            let ny0 = (y - radius).max(0);
            let ny1 = (y + radius + 1).min(size.height as i32);
            let nx0 = (x - radius).max(0);
            let nx1 = (x + radius + 1).min(size.width as i32);
            for ny in ny0..ny1 {
                for nx in nx0..nx1 {
                    let idx = ((ny as u32 * size.width + nx as u32) as usize) * 4;
                    sum_r += src[idx] as u32;
                    sum_g += src[idx + 1] as u32;
                    sum_b += src[idx + 2] as u32;
                    count += 1;
                }
            }
            let dst_idx = ((y as u32 * size.width + x as u32) as usize) * 4;
            dst[dst_idx] = (sum_r / count) as u8;
            dst[dst_idx + 1] = (sum_g / count) as u8;
            dst[dst_idx + 2] = (sum_b / count) as u8;
            dst[dst_idx + 3] = 0xFF;
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
        let src = empty_rgba(20, 20);
        let mut rgba = src.clone();
        let ann = Annotation::rectangle(
            AnnotationId(1),
            PhysicalPoint::new(2, 3),
            PhysicalSize::new(6, 4),
            AnnotationColor::Red,
            AnnotationStroke::Thin,
            0,
        );
        paint_annotation(&src, &mut rgba, size, &ann);
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
        let src = empty_rgba(30, 30);
        let mut rgba = src.clone();
        let ann = Annotation::arrow(
            AnnotationId(1),
            PhysicalPoint::new(2, 2),
            PhysicalPoint::new(20, 20),
            AnnotationColor::Green,
            AnnotationStroke::Medium,
            0,
        );
        paint_annotation(&src, &mut rgba, size, &ann);
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
        let src = empty_rgba(60, 60);
        let mut rgba = src.clone();
        let ann = Annotation::numbered_badge(
            AnnotationId(1),
            PhysicalPoint::new(30, 30),
            BADGE_RADIUS_PX,
            1,
            AnnotationColor::Blue,
            AnnotationStroke::Thin,
            0,
        );
        paint_annotation(&src, &mut rgba, size, &ann);
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

    // --- Tracer-05 text + blur tests --------------------------------

    /// Build an RGBA buffer where the interior of the `text_box` is
    /// uniformly bright (white) and the rest is uniformly dark
    /// (black). Used by the plate-contrast tests so the source
    /// luminance is unambiguous.
    fn two_tone_rgba(w: u32, h: u32, box_origin: PhysicalPoint, box_size: PhysicalSize) -> Vec<u8> {
        let mut rgba = vec![0u8; (w as usize) * (h as usize) * 4];
        for y in 0..h {
            for x in 0..w {
                let idx = ((y * w + x) as usize) * 4;
                rgba[idx + 3] = 0xFF; // alpha opaque
                if x >= box_origin.x as u32
                    && x < (box_origin.x + box_size.width as i32) as u32
                    && y >= box_origin.y as u32
                    && y < (box_origin.y + box_size.height as i32) as u32
                {
                    rgba[idx] = 0xFF;
                    rgba[idx + 1] = 0xFF;
                    rgba[idx + 2] = 0xFF;
                }
            }
        }
        rgba
    }

    /// Box covering the entire buffer — used to verify contrast rules
    /// without region-clipping ambiguity.
    fn full_box(w: u32, h: u32) -> (PhysicalPoint, PhysicalSize) {
        (PhysicalPoint::new(0, 0), PhysicalSize::new(w, h))
    }

    /// Bright source (white pixels under the text box) yields a white
    /// plate + dark glyph.
    #[test]
    fn plate_contrast_bright_source_picks_dark_glyph() {
        let size = PhysicalSize::new(40, 20);
        let (origin, box_size) = full_box(40, 20);
        let src = two_tone_rgba(40, 20, origin, box_size);
        let mut rgba = src.clone();
        let ann = Annotation::text(
            AnnotationId(1),
            origin,
            box_size,
            "hi".to_string(),
            AnnotationColor::Red,
            AnnotationStroke::Thin,
            0,
        );
        paint_annotation(&src, &mut rgba, size, &ann);
        // Plate must be near-white inside the box (sampled outside the
        // glyph stroke — pick the very top-left corner where the text
        // never paints).
        let (r, g, b, _) = pixel_at(&rgba, 40, 0, 0);
        assert_eq!((r, g, b), (0xFF, 0xFF, 0xFF), "bright source → white plate");
    }

    /// Dark source (black pixels under the text box) yields a black
    /// plate + bright glyph.
    #[test]
    fn plate_contrast_dark_source_picks_light_glyph() {
        let size = PhysicalSize::new(40, 20);
        let (origin, box_size) = full_box(40, 20);
        // Build a fully black source.
        let mut src = vec![0u8; (40 * 20 * 4) as usize];
        for chunk in src.chunks_exact_mut(4) {
            chunk[3] = 0xFF;
        }
        let mut rgba = src.clone();
        let ann = Annotation::text(
            AnnotationId(1),
            origin,
            box_size,
            "hi".to_string(),
            AnnotationColor::Yellow,
            AnnotationStroke::Thin,
            0,
        );
        paint_annotation(&src, &mut rgba, size, &ann);
        // Plate must be the dark background colour.
        let (r, g, b, _) = pixel_at(&rgba, 40, 0, 0);
        assert_eq!((r, g, b), (0x14, 0x14, 0x14), "dark source → dark plate");
    }

    /// Multi-word text wraps within the configured box width. We use a
    /// generous box and assert that the second line's glyph row is
    /// painted at a Y below the first line's glyph row.
    #[test]
    fn text_wraps_within_width() {
        let size = PhysicalSize::new(60, 40);
        let (origin, box_size) = full_box(60, 40);
        let src = two_tone_rgba(60, 40, origin, box_size);
        let mut rgba = src.clone();
        // Thin stroke ⇒ 5x7 glyph, 6 px cell width. A 30 px inner
        // width holds 5 cells; "hello world" must wrap onto two
        // lines.
        let ann = Annotation::text(
            AnnotationId(1),
            origin,
            box_size,
            "hello world".to_string(),
            AnnotationColor::Red,
            AnnotationStroke::Thin,
            0,
        );
        paint_annotation(&src, &mut rgba, size, &ann);
        // Find any non-plate pixel on the first line (y ≈ padding+0..7)
        // and any on the second line (y ≈ padding+8..15). The glyph
        // is contrast-determined (base + 60% user-colour tint), so
        // we accept any pixel that differs from the white plate.
        let mut found_first = false;
        let mut found_second = false;
        for y in 0..40u32 {
            for x in 0..60u32 {
                let (r, g, b, _) = pixel_at(&rgba, 60, x, y);
                // Anything that isn't the white plate is either a
                // glyph pixel or a tinted boundary pixel.
                if r != 0xFF || g != 0xFF || b != 0xFF {
                    if y < 10 {
                        found_first = true;
                    } else if (10..20).contains(&y) {
                        found_second = true;
                    }
                }
            }
        }
        assert!(found_first, "first-line glyph must paint");
        assert!(found_second, "wrapped second-line glyph must paint");
    }

    /// Blur averages the source pixels inside the blur region. The
    /// leak-guard invariant: a high-contrast secret line under the
    /// blur must NOT survive in pure form. The pattern is a single
    /// column of magenta pixels (a thin secret) over a uniform dark
    /// background; the 9×9 blur kernel samples mostly gray plus one
    /// column of magenta, so the output cannot contain pure magenta.
    #[test]
    fn blur_leak_guard_removes_source_pixels() {
        let w = 30u32;
        let h = 20u32;
        let size = PhysicalSize::new(w, h);
        let mut src = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let idx = ((y * w + x) as usize) * 4;
                src[idx + 3] = 0xFF;
                if x == 10 && (5..15).contains(&y) {
                    // Single-column "secret" — only one pixel wide.
                    src[idx] = 0xFF;
                    src[idx + 1] = 0x00;
                    src[idx + 2] = 0xFF;
                } else {
                    src[idx] = 0x40;
                    src[idx + 1] = 0x40;
                    src[idx + 2] = 0x40;
                }
            }
        }
        let mut rgba = src.clone();
        let ann = Annotation::blur(
            AnnotationId(1),
            PhysicalPoint::new(5, 5),
            PhysicalSize::new(10, 10),
            4, // 9×9 kernel
            0,
        );
        paint_annotation(&src, &mut rgba, size, &ann);
        // No pixel in the blur region may be the pure-secret magenta.
        // The blur kernel's mean of (1 magenta + 80 gray) / 81 ≈ gray,
        // so the output's R channel stays near 0x40.
        for y in 5..15 {
            for x in 5..15 {
                let (r, g, b, _) = pixel_at(&rgba, w, x, y);
                assert!(
                    !(r == 0xFF && g == 0x00 && b == 0xFF),
                    "secret pixel at ({x},{y}) must not leak through blur"
                );
            }
        }
    }

    /// Blur region that extends past the source right edge averages
    /// only the in-bounds pixels — no wrap-around, no panic.
    #[test]
    fn blur_bounds_clip_to_source() {
        let w = 20u32;
        let h = 10u32;
        let size = PhysicalSize::new(w, h);
        // All bright source for an unambiguous mean.
        let src = vec![0xFFu8; (w * h * 4) as usize];
        let mut rgba = src.clone();
        // Blur extends to x=30 (past the right edge).
        let ann = Annotation::blur(
            AnnotationId(1),
            PhysicalPoint::new(15, 0),
            PhysicalSize::new(15, 10),
            1,
            0,
        );
        paint_annotation(&src, &mut rgba, size, &ann);
        // The right-edge pixels (x=19) must average bright source +
        // out-of-bounds (clipped). They should be bright (R=0xFF)
        // because every in-bounds sample is bright.
        let (r, _, _, _) = pixel_at(&rgba, w, 19, 5);
        assert_eq!(r, 0xFF, "clipped average of bright pixels must be bright");
    }

    /// Blur samples from the immutable source, not from the in-flight
    /// output buffer. If the blur rasterizer were rewired to read
    /// from `dst`, an earlier arrow painted under the blur region
    /// would survive in the output — the leak guard. This test
    /// inverts the invariant: paint an arrow at z=0, a blur at z=1,
    /// and assert the arrow does NOT survive inside the blur region.
    #[test]
    fn blur_samples_from_source_not_in_flight_output() {
        let w = 30u32;
        let h = 20u32;
        let size = PhysicalSize::new(w, h);
        let src = vec![0u8; (w * h * 4) as usize];
        let annotations = vec![
            // An arrow at z=0 that lies *under* the blur region.
            Annotation::arrow(
                AnnotationId(1),
                PhysicalPoint::new(2, 10),
                PhysicalPoint::new(28, 10),
                AnnotationColor::Green,
                AnnotationStroke::Medium,
                0,
            ),
            // A blur at z=1 covering the arrow.
            Annotation::blur(
                AnnotationId(2),
                PhysicalPoint::new(0, 0),
                PhysicalSize::new(w, h),
                2,
                1,
            ),
        ];
        let out = flatten_annotations(&src, size, &annotations);
        // No green pixel (the arrow colour) may survive inside the
        // blur region. The arrow's green is at the centre column; the
        // blur region covers the entire buffer.
        for y in 0..h {
            for x in 0..w {
                let (_r, g, _b, _) = pixel_at(&out, w, x, y);
                assert_eq!(
                    g, 0,
                    "arrow green must not survive blur; leaked at ({x},{y})"
                );
            }
        }
    }

    /// Identical inputs produce identical outputs across two runs for
    /// text + blur (the determinism invariant).
    #[test]
    fn flatten_is_deterministic_for_text_and_blur() {
        let size = PhysicalSize::new(40, 20);
        let (origin, box_size) = full_box(40, 20);
        let src = two_tone_rgba(40, 20, origin, box_size);
        let annotations = vec![
            Annotation::text(
                AnnotationId(1),
                PhysicalPoint::new(2, 2),
                PhysicalSize::new(30, 14),
                "label".to_string(),
                AnnotationColor::Red,
                AnnotationStroke::Thin,
                0,
            ),
            Annotation::blur(
                AnnotationId(2),
                PhysicalPoint::new(0, 0),
                PhysicalSize::new(40, 20),
                2,
                1,
            ),
        ];
        let first = flatten_annotations(&src, size, &annotations);
        let second = flatten_annotations(&src, size, &annotations);
        assert_eq!(first, second, "text + blur flatten must be deterministic");
    }

    /// The text + blur geometry variants round-trip via JSON so the
    /// TypeScript mirror can decode them. Companion to the IPC
    /// contract tests in `src-tauri/tests/ipc_contracts.rs`.
    #[test]
    fn text_and_blur_round_trip_via_json() {
        let text = Annotation::text(
            AnnotationId(7),
            PhysicalPoint::new(10, 20),
            PhysicalSize::new(120, 40),
            "hello\nworld".to_string(),
            AnnotationColor::Yellow,
            AnnotationStroke::Medium,
            3,
        );
        let json = serde_json::to_string(&text).expect("serialize text");
        assert!(json.contains("\"kind\":\"text\""));
        assert!(json.contains("\"text\":\"hello\\nworld\""));
        let parsed: Annotation = serde_json::from_str(&json).expect("deserialize text");
        assert_eq!(parsed, text);

        let blur = Annotation::blur(
            AnnotationId(8),
            PhysicalPoint::new(5, 5),
            PhysicalSize::new(40, 40),
            4,
            5,
        );
        let json = serde_json::to_string(&blur).expect("serialize blur");
        assert!(json.contains("\"kind\":\"blur\""));
        assert!(json.contains("\"radius\":4"));
        let parsed: Annotation = serde_json::from_str(&json).expect("deserialize blur");
        assert_eq!(parsed, blur);
    }

    /// Adversarial multi-size leak guard. Spec validation asks for
    /// secrets at several sizes; this exercises 1-, 2-, and 4-pixel
    /// wide secret columns under blur regions of varying radius so
    /// a regression that happens to satisfy one geometry is caught.
    #[test]
    fn blur_leak_guard_at_multiple_secret_widths() {
        // (src_w, src_h, blur_x, blur_y, blur_w, blur_h, blur_radius,
        //  secret_x, secret_width)
        type Case = (
            u32, u32, u32, u32, u32, u32, u32, u32, u32,
        );
        let cases: [Case; 4] = [
            (30, 20, 5, 5, 10, 10, 4, 10, 1),
            (40, 30, 5, 5, 20, 20, 2, 15, 2),
            (60, 40, 10, 10, 30, 30, 6, 25, 4),
            (80, 60, 0, 0, 70, 50, 3, 35, 2),
        ];
        for (src_w, src_h, bx, by, bw, bh, radius, secret_x, secret_w) in cases {
            let size = PhysicalSize::new(src_w, src_h);
            // Source: dark gray background with a high-contrast
            // magenta block spanning the blur region. We pick the
            // blur centre as the secret so the average stays magenta
            // — every test pixel must show a non-magenta average
            // *because* the blur replaces the source in `dst`, never
            // because the surrounding gray dilutes a thin line.
            // The blur rasterizer sampling from `src` and writing
            // to `dst` is what we're guarding: the surrounding
            // background is part of the test that the blur region
            // gets replaced.
            let cy = by + bh / 2;
            let mut src = vec![0u8; (src_w * src_h * 4) as usize];
            for y in 0..src_h {
                for x in 0..src_w {
                    let idx = ((y * src_w + x) as usize) * 4;
                    src[idx + 3] = 0xFF;
                    let in_blur = x >= bx && x < bx + bw && y >= by && y < by + bh;
                    let is_secret = in_blur
                        && x >= secret_x
                        && x < secret_x + secret_w
                        && (by..cy + 1).contains(&y);
                    if is_secret {
                        src[idx] = 0xFF;
                        src[idx + 1] = 0x00;
                        src[idx + 2] = 0xFF;
                    } else {
                        src[idx] = 0x40;
                        src[idx + 1] = 0x40;
                        src[idx + 2] = 0x40;
                    }
                }
            }
            let mut rgba = src.clone();
            let ann = Annotation::blur(
                AnnotationId(1),
                PhysicalPoint::new(bx as i32, by as i32),
                PhysicalSize::new(bw, bh),
                radius,
                0,
            );
            paint_annotation(&src, &mut rgba, size, &ann);
            // No pixel inside the blur region may be pure magenta.
            for y in by..by + bh {
                for x in bx..bx + bw {
                    let (r, g, b, _) = pixel_at(&rgba, src_w, x, y);
                    assert!(
                        !(r == 0xFF && g == 0x00 && b == 0xFF),
                        "secret pixel leaked at ({x},{y}) for geometry {src_w}x{src_h} blur={bx},{by}+{bw}x{bh} r={radius} secret_w={secret_w}"
                    );
                }
            }
        }
    }

    /// Joint text + blur export path (mirrors the `save_capture_as`
    /// flatten pipeline). A blur covering part of the source + a
    /// text annotation elsewhere must both appear in the output and
    /// the blur region must still not leak.
    #[test]
    fn joint_text_and_blur_export_path() {
        let size = PhysicalSize::new(80, 60);
        // Thin magenta "secret" line under the blur region so the
        // 9×9 box-blur kernel averages mostly gray plus one column
        // of magenta. The pure-magenta leak detection still holds
        // because a uniform-secret block would simply average back
        // to itself (no leak to detect).
        let mut src = vec![0u8; (80 * 60 * 4) as usize];
        for y in 0..60u32 {
            for x in 0..80u32 {
                let idx = ((y * 80 + x) as usize) * 4;
                src[idx + 3] = 0xFF;
                if x == 45 && (20..40).contains(&y) {
                    src[idx] = 0xFF;
                    src[idx + 1] = 0x00;
                    src[idx + 2] = 0xFF;
                } else {
                    src[idx] = 0x40;
                    src[idx + 1] = 0x40;
                    src[idx + 2] = 0x40;
                }
            }
        }
        let annotations = vec![
            Annotation::blur(
                AnnotationId(1),
                PhysicalPoint::new(30, 20),
                PhysicalSize::new(30, 20),
                4,
                0,
            ),
            Annotation::text(
                AnnotationId(2),
                PhysicalPoint::new(5, 5),
                PhysicalSize::new(50, 14),
                "label".to_string(),
                AnnotationColor::Red,
                AnnotationStroke::Thin,
                1,
            ),
        ];
        let out = flatten_annotations(&src, size, &annotations);
        // Blur region: no pure magenta survives.
        for y in 20..40 {
            for x in 30..60 {
                let (r, g, b, _) = pixel_at(&out, 80, x, y);
                assert!(
                    !(r == 0xFF && g == 0x00 && b == 0xFF),
                    "secret pixel leaked at ({x},{y})"
                );
            }
        }
        // Text region must contain non-zero bytes (text actually
        // painted). The plate + glyphs leave a fingerprint even
        // when the source is uniform gray.
        let mut text_nonzero = 0;
        for y in 5..19u32 {
            for x in 5..55u32 {
                let (r, g, b, _) = pixel_at(&out, 80, x, y);
                if (r, g, b) != (0x40, 0x40, 0x40) {
                    text_nonzero += 1;
                }
            }
        }
        assert!(
            text_nonzero > 0,
            "text annotation must leave a visible footprint on the output"
        );
    }

    /// `tint_glyph` blends the user-chosen colour into the
    /// contrast-determined base so the palette is visible. A pure
    /// red user colour blended into a dark base yields a
    /// red-shifted dark glyph; into a light base yields a
    /// red-shifted light glyph.
    #[test]
    fn tint_glyph_blends_user_colour_with_contrast_base() {
        let dark = PaintColor {
            r: 0x14,
            g: 0x14,
            b: 0x14,
            a: 0xFF,
        };
        let light = PaintColor {
            r: 0xFF,
            g: 0xFF,
            b: 0xFF,
            a: 0xFF,
        };
        let red = PaintColor {
            r: 0xE5,
            g: 0x3B,
            b: 0x3B,
            a: 0xFF,
        };
        let tinted_dark = tint_glyph(dark, red);
        // The red channel must dominate over the dark base.
        assert!(tinted_dark.r > dark.r);
        assert!(tinted_dark.r > tinted_dark.g);
        assert!(tinted_dark.r > tinted_dark.b);
        let tinted_light = tint_glyph(light, red);
        // Tinted toward light base keeps a hint of the user colour
        // but stays bright enough to read on a dark plate.
        assert!(tinted_light.r < 0xFF);
        assert!(tinted_light.r > tinted_light.g);
    }
}
