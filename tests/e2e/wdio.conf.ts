// WebdriverIO configuration for packaged-app acceptance tests.
//
// Tracer-01 only ships the synthetic capture flow. Subsequent tracers
// expand the spec list. The Tauri service drives the real packaged binary
// so the acceptance seam is the production app, not a development build.

import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));

export const config = {
  runner: "local",
  specs: ["./specs/**/*.spec.ts"],
  maxInstances: 1,
  capabilities: [
    {
      "tauri:options": {
        application: resolve(__dirname, "..", "..", "dist-build", "pixelgrab.exe"),
      },
    },
  ],
  reporters: ["spec"],
  framework: "mocha",
  mochaOpts: {
    ui: "bdd",
    timeout: 60_000,
  },
};
