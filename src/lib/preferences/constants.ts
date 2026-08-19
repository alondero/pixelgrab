// Shared constants for the shelf preferences UI. Mirrors the Rust
// constants in `crates/pixelgrab-contracts/src/shelf_preferences.rs`
// — keep them in sync. When these drift, the UI clamps to the wider
// range so a Rust bump that widens the allowed range still works.

export const MIN_LIFETIME_SECONDS = 5;
export const MAX_LIFETIME_SECONDS = 300;
export const MIN_MARGIN_PX = 0;
export const MAX_MARGIN_PX = 128;
export const MIN_VISIBLE_CARDS = 1;
export const MAX_VISIBLE_CARDS = 8;

// Lifetime preset chips. The values are seconds; the UI shows them
// as human-friendly labels (e.g. "10s", "1m").
export const LIFETIME_PRESETS_SECONDS = [10, 30, 60, 120, 300] as const;
