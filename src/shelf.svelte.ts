import { mount } from "svelte";
import { listen } from "@tauri-apps/api/event";
import ShelfQueue from "./lib/shelf/ShelfQueue.svelte";
import { createShelfController } from "./lib/shelf/controller.svelte";

const target = document.getElementById("shelf");
if (!target) throw new Error("shelf root element not found");
const shelf = createShelfController();
mount(ShelfQueue, {
  target,
  props: {
    get snapshot() {
      return shelf.snapshot;
    },
    get feedback() {
      return shelf.feedback.message;
    },
    get showCountdown() {
      return shelf.showCountdown;
    },
    ...shelf.actions,
  },
});
void shelf.start();
const focusListener = listen("pixelgrab://shelf-focus-requested", () => {
  requestAnimationFrame(() =>
    target.querySelector<HTMLElement>('[data-testid="shelf-card"]')?.focus(),
  );
});
window.addEventListener(
  "beforeunload",
  () => {
    shelf.dispose();
    void focusListener.then((fn) => fn());
  },
  { once: true },
);
