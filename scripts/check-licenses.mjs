#!/usr/bin/env node
// License-policy check. Verifies that every dependency declares a license
// and that the license is in the allow-list. The allow-list is intentionally
// conservative; expand it intentionally when adopting a new license.
//
// Usage: node scripts/check-licenses.mjs
import { readFileSync } from "node:fs";
import { join } from "node:path";

const ALLOWED = new Set([
  "MIT",
  "Apache-2.0",
  "Apache-2.0 WITH LLVM-exception",
  "BSD-2-Clause",
  "BSD-3-Clause",
  "ISC",
  "CC0-1.0",
  "MPL-2.0",
  "0BSD",
  "Python-2.0",
  "Unicode-DFS-2016",
  "Unicode-3.0",
  "Zlib",
  "WTFPL",
  "BlueOak-1.0.0",
  "Unlicense",
]);

const BLOCKED = new Set(["AGPL-3.0", "AGPL-3.0-or-later", "SSPL-1.0", "Commons-Clause"]);

const root = process.cwd();
const pkg = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));

const allDeps = {
  ...(pkg.dependencies ?? {}),
  ...(pkg.devDependencies ?? {}),
  ...(pkg.optionalDependencies ?? {}),
};

const offenders = [];
const checked = [];

for (const [name, version] of Object.entries(allDeps)) {
  try {
    const manifest = JSON.parse(
      readFileSync(join(root, "node_modules", name, "package.json"), "utf8"),
    );
    const license = manifest.license ?? manifest.licenses ?? "";
    const text = Array.isArray(license) ? license.join(" OR ") : license;
    if (!text || text === "UNKNOWN") {
      offenders.push({ name, version: manifest.version, reason: "missing license" });
      continue;
    }
    if (BLOCKED.has(text)) {
      offenders.push({ name, version: manifest.version, reason: `blocked license: ${text}` });
      continue;
    }
    // Allow if any token is in the allow-list.
    const tokens = text.split(/\s+(?:OR|AND)\s+|\//).map((t) => t.trim());
    const ok = tokens.some((t) => ALLOWED.has(t));
    if (!ok) {
      offenders.push({ name, version: manifest.version, reason: `disallowed license: ${text}` });
      continue;
    }
    checked.push({ name, version: manifest.version, license: text });
  } catch (err) {
    offenders.push({ name, version, reason: `unable to read manifest: ${err.message}` });
  }
}

if (offenders.length > 0) {
  console.error("License policy violations:");
  for (const o of offenders) {
    console.error(`  - ${o.name}@${o.version}: ${o.reason}`);
  }
  process.exit(1);
}

console.log(`License check passed: ${checked.length} dependencies verified.`);
