// Pin window entrypoint (issue #63). Each pin lives in its own
// borderless TopMost webview window created by the Rust core when the
// shelf card's Pin action fires `open_pin`. The window label is
// `pin-{pinId}` and the pin id arrives via the `?id=` query parameter,
// so the entrypoint fetches its own view model with `get_pin` — no
// event race on startup. Subsequent registry updates (zoom, opacity,
// display re-anchoring) arrive as targeted
// `pixelgrab://pin-viewmodel` events.
//
// The native window owns the on-screen position: dragging inside the
// content routes a `drag` command to the registry, and the IPC layer
// applies the updated transform to the window itself.

import { mount } from "svelte";
import { listen } from "@tauri-apps/api/event";
import type { PinViewModel } from "./lib/pin/types";
import { getPin } from "./lib/pin/commands";
import PinWindow from "./lib/pin/PinWindow.svelte";

const target = document.getElementById("pin");
if (!target) {
  throw new Error("pin root element not found");
}

const pinId = new URLSearchParams(window.location.search).get("id");
if (!pinId) {
  throw new Error("pin window opened without an ?id= parameter");
}

const response = await getPin(pinId);
if (response.status !== "ok") {
  // A pin that cannot be restored would otherwise leave an invisible
  // borderless window on screen with no close affordance. Render the
  // categorical failure and stop — the backend's registry lock is
  // released by the caller rolling back.
  const message =
    response.status === "err"
      ? `Pin could not be restored (${response.error.kind})`
      : "Pin unavailable";
  document.body.textContent = message;
  throw new Error(message);
}

let latestView = $state<PinViewModel>(response.data);

mount(PinWindow, {
  target,
  props: {
    get view() {
      return latestView;
    },
  },
});

listen<PinViewModel>("pixelgrab://pin-viewmodel", (event) => {
  if (event.payload.id === pinId) {
    latestView = event.payload;
  }
});
