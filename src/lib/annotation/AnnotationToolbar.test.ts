// Annotation toolbar rendering tests. Verifies the toolbar renders the
// expected tool/colour/stroke/history controls, surfaces the right
// aria-pressed state when the store changes, and reflects undo/redo
// availability. The toolbar never calls the IPC layer directly — it
// always drives the `annotationStore` rune object.

import { describe, it, expect, beforeEach } from "vitest";
import { render } from "@testing-library/svelte";
import { tick } from "svelte";
import AnnotationToolbar from "./AnnotationToolbar.svelte";
import { annotationStore } from "./store.svelte";

describe("AnnotationToolbar", () => {
  beforeEach(() => {
    annotationStore.reset();
  });

  it("renders every tool button with the right shortcut label", () => {
    const { getByTestId } = render(AnnotationToolbar, { visible: true });
    expect(getByTestId("tool-select")).toBeInTheDocument();
    expect(getByTestId("tool-arrow")).toBeInTheDocument();
    expect(getByTestId("tool-rectangle")).toBeInTheDocument();
    expect(getByTestId("tool-numbered_badge")).toBeInTheDocument();
  });

  it("reflects the active tool via aria-pressed", () => {
    annotationStore.setTool("arrow");
    const { getByTestId } = render(AnnotationToolbar, { visible: true });
    expect(getByTestId("tool-arrow").getAttribute("aria-pressed")).toBe("true");
    expect(getByTestId("tool-rectangle").getAttribute("aria-pressed")).toBe("false");
  });

  it("clicking a tool button updates the store", async () => {
    const { getByTestId } = render(AnnotationToolbar, { visible: true });
    getByTestId("tool-rectangle").click();
    expect(annotationStore.tool).toBe("rectangle");
  });

  it("clicking a colour button updates the active color", async () => {
    const { getByTestId } = render(AnnotationToolbar, { visible: true });
    getByTestId("color-blue").click();
    expect(annotationStore.color).toBe("blue");
  });

  it("clicking a stroke button updates the active stroke width", async () => {
    const { getByTestId } = render(AnnotationToolbar, { visible: true });
    getByTestId("stroke-thick").click();
    expect(annotationStore.stroke).toBe("thick");
  });

  it("with a single selection, the toolbar reflects the selected annotation's colour", () => {
    annotationStore.setTool("rectangle");
    annotationStore.beginDraft("rectangle", { x: 0, y: 0 });
    annotationStore.updateDraft({ x: 50, y: 50 });
    annotationStore.commitDraft();
    annotationStore.setColor("blue");
    annotationStore.beginDraft("rectangle", { x: 100, y: 100 });
    annotationStore.updateDraft({ x: 150, y: 150 });
    annotationStore.commitDraft();
    const ids = annotationStore.annotations.map((a) => a.id);
    // Select the second rectangle (which is blue).
    annotationStore.selectOnly(ids[1]);
    const { getByTestId } = render(AnnotationToolbar, { visible: true });
    const blue = getByTestId("color-blue");
    expect(blue.getAttribute("data-active")).toBe("active");
    expect(getByTestId("color-state").getAttribute("data-state")).toBe("selected");
  });

  it("mixed selection reports the heterogeneous state on the swatch group", () => {
    annotationStore.setTool("rectangle");
    annotationStore.beginDraft("rectangle", { x: 0, y: 0 });
    annotationStore.updateDraft({ x: 50, y: 50 });
    annotationStore.commitDraft();
    annotationStore.setColor("green");
    annotationStore.beginDraft("rectangle", { x: 100, y: 100 });
    annotationStore.updateDraft({ x: 150, y: 150 });
    annotationStore.commitDraft();
    annotationStore.selectAll();
    const { getByTestId } = render(AnnotationToolbar, { visible: true });
    expect(getByTestId("color-state").getAttribute("data-state")).toBe("mixed");
    // The swatches carry the indeterminate marker so the styling
    // layer can dashed-border the active swatch.
    expect(getByTestId("color-red").getAttribute("data-active")).toBe("mixed");
  });

  it("clicking a colour swatch with a selection updates every selected annotation", () => {
    annotationStore.setTool("rectangle");
    annotationStore.beginDraft("rectangle", { x: 0, y: 0 });
    annotationStore.updateDraft({ x: 50, y: 50 });
    annotationStore.commitDraft();
    annotationStore.beginDraft("rectangle", { x: 100, y: 100 });
    annotationStore.updateDraft({ x: 150, y: 150 });
    annotationStore.commitDraft();
    annotationStore.selectAll();
    const { getByTestId } = render(AnnotationToolbar, { visible: true });
    getByTestId("color-yellow").click();
    for (const a of annotationStore.annotations) {
      expect(a.color).toBe("yellow");
    }
  });

  it("undo/redo buttons start disabled and enable after a committed action", async () => {
    const { getByTestId } = render(AnnotationToolbar, { visible: true });
    expect((getByTestId("undo") as HTMLButtonElement).disabled).toBe(true);
    expect((getByTestId("redo") as HTMLButtonElement).disabled).toBe(true);

    // beginDraft + commitDraft alone push history (pointer-move
    // frames do not). setTool is intentionally NOT used here
    // because it is also a history-pushing action.
    annotationStore.beginDraft("arrow", { x: 0, y: 0 });
    annotationStore.updateDraft({ x: 100, y: 0 });
    annotationStore.commitDraft();
    await tick();
    expect((getByTestId("undo") as HTMLButtonElement).disabled).toBe(false);

    annotationStore.undo();
    await tick();
    expect((getByTestId("redo") as HTMLButtonElement).disabled).toBe(false);
  });

  it("palette hex values mirror the Rust AnnotationColor::rgba constants", () => {
    // Pin the frontend palette so a one-byte drift on either side of
    // the IPC line fails CI. The Rust-side pin lives in
    // `src-tauri/tests/ipc_contracts.rs::annotation_palette_and_stroke_widths_are_pinned`.
    const render2 = render(AnnotationToolbar, { visible: true });
    const swatches = render2.container.querySelectorAll(
      '[data-testid^="color-"]',
    ) as NodeListOf<HTMLButtonElement>;
    const swatchMap = new Map<string, string>();
    swatches.forEach((el) => {
      const id = el.getAttribute("data-testid")!.replace("color-", "");
      // jsdom exposes the inline `style` as a CSSStyleDeclaration; the
      // custom property `--swatch` lives there. We use
      // `getPropertyValue` so the assertion stays robust against
      // whitespace / order changes.
      const value = (el.style as CSSStyleDeclaration).getPropertyValue("--swatch");
      swatchMap.set(id, value.trim().toLowerCase());
    });
    expect(swatchMap.get("red")).toBe("#e53b3b");
    expect(swatchMap.get("green")).toBe("#3be55c");
    expect(swatchMap.get("blue")).toBe("#3b82e5");
    expect(swatchMap.get("yellow")).toBe("#f6e33b");
    expect(swatchMap.get("white")).toBe("#ffffff");
  });

  it("stroke buttons expose 2 / 4 / 8 px bar heights", () => {
    const { container } = render(AnnotationToolbar, { visible: true });
    const thin = container.querySelector('[data-testid="stroke-thin"] .stroke-bar') as HTMLElement;
    const medium = container.querySelector(
      '[data-testid="stroke-medium"] .stroke-bar',
    ) as HTMLElement;
    const thick = container.querySelector(
      '[data-testid="stroke-thick"] .stroke-bar',
    ) as HTMLElement;
    expect(thin.style.getPropertyValue("--bar-height")).toBe("2px");
    expect(medium.style.getPropertyValue("--bar-height")).toBe("4px");
    expect(thick.style.getPropertyValue("--bar-height")).toBe("8px");
  });
});
