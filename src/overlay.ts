import { mount } from "svelte";
import OverlayApp from "./lib/overlay/OverlayApp.svelte";

const overlay = mount(OverlayApp, {
  target: document.getElementById("overlay")!,
});

export default overlay;
