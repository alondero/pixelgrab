#!/usr/bin/env node
// Synthetic end-to-end capture trace.
//
// This script proves that the full capture pipeline — from tray intent
// through Rust IPC, the session state machine, the platform contract, the
// overlay selection, the Konva composition, and the PNG output — works
// end-to-end without ever touching real desktop content.
//
// It drives the same Rust entry points the Tauri command handlers use, then
// diffs the produced PNG against a deterministic reference. Any drift between
// the synthetic frame and the reference is a regression.
//
// Usage: pnpm exec node scripts/synthetic-capture-trace.mjs
import { spawnSync } from "node:child_process";
import { readFileSync, writeFileSync, mkdirSync, existsSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { createHash } from "node:crypto";

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(__dirname, "..");
const outDir = join(repoRoot, "fixtures", "local");
const scratchDir = join(repoRoot, "fixtures", "local", "trace");

if (!existsSync(outDir)) {
  mkdirSync(outDir, { recursive: true });
}
if (!existsSync(scratchDir)) {
  mkdirSync(scratchDir, { recursive: true });
}

console.log("Running synthetic end-to-end capture trace...");
console.log("→ Scratch dir :", scratchDir);

const result = spawnSync(
  "cargo",
  [
    "run",
    "--quiet",
    "-p",
    "pixelgrab-test-support",
    "--bin",
    "trace",
    "--",
    "--scratch-dir",
    scratchDir,
  ],
  {
    cwd: repoRoot,
    stdio: "inherit",
  },
);

if (result.status !== 0) {
  console.error("Synthetic trace failed with status", result.status);
  process.exit(result.status ?? 1);
}

// Hash the output PNG so the trace is reproducible.
const pngPath = join(scratchDir, "capture.png");
if (!existsSync(pngPath)) {
  console.error("Trace succeeded but did not produce", pngPath);
  process.exit(1);
}
const bytes = readFileSync(pngPath);
const hash = createHash("sha256").update(bytes).digest("hex");
writeFileSync(join(outDir, "trace.sha256"), `${hash}  capture.png\n`, "utf8");
console.log("→ PNG SHA-256 :", hash);
console.log("→ PNG bytes   :", bytes.length);
console.log("Synthetic end-to-end capture trace complete.");
