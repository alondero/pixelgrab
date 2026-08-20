/// <reference types="svelte" />
/// <reference types="vite/client" />

// Tracer 14 follow-up: the shared modifier alias JSON lives
// under `crates/pixelgrab-contracts/data/` so the Rust side can
// `include_str!` it. The Vite `$contracts` alias (see
// `vite.config.ts`) exposes the same directory to TS code; the
// declaration here gives the TS compiler a typed handle for the
// JSON shape.
declare module "$contracts/data/hotkey_modifiers.json" {
  const value: {
    schemaVersion: number;
    modifiers: Array<{ canonical: string; aliases: string[] }>;
    rank: string[];
  };
  export default value;
}
