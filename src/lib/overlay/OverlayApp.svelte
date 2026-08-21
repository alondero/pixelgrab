<script lang="ts">
  import { onMount } from "svelte";
  import {
    getSessionSnapshot,
    requestCommit,
    requestCancel,
    saveCaptureAs,
  } from "$lib/ipc/commands";
  import KonvaStage from "./KonvaStage.svelte";
  import AnnotationToolbar from "$lib/annotation/AnnotationToolbar.svelte";
  import { annotationStore } from "$lib/annotation/store.svelte";
  import type { CaptureResolutionDto, PhysicalBounds } from "$lib/ipc/types";

  let capture = $state<CaptureResolutionDto | null>(null);
  let selection = $state<PhysicalBounds | null>(null);
  let lastDiagnosticsId = $state<string | null>(null);
  let commitError = $state<string | null>(null);
  let saveAsError = $state<string | null>(null);
  let lastSaveAsPath = $state<string | null>(null);
  // The overlay spans the entire primary monitor. Values are tuned for the
  // tracer-02 default layout; the real multi-monitor sizing arrives in
  // tracer-04.
  const STAGE_WIDTH = 1920;
  const STAGE_HEIGHT = 1080;

  onMount(async () => {
    // Issue #60: the overlay reveal contract is collapsed into one
    // backend seam (`show_over_virtual_desktop` → `overlay_mounted`),
    // so the frontend never has to drive the `Ready -> Selecting`
    // transition. We just read the snapshot the orchestrator already
    // stamped.
    const response = await getSessionSnapshot();
    if (response.status === "ok" && response.data.lastCapture) {
      capture = response.data.lastCapture;
      lastDiagnosticsId = response.data.lastCapture.captureId;
    }
  });

  function onSelectionChange(next: PhysicalBounds | null) {
    selection = next;
    if (!next) {
      commitError = null;
    }
  }

  async function onCommit() {
    if (!selection) return;
    commitError = null;
    // Tracer 04: ship the editor's annotations alongside the crop so
    // the Rust commit pipeline can flatten them onto the frozen
    // framebuffer before publishing to the clipboard or the cache.
    const annotations = annotationStore.annotations.map((a) => ({
      id: a.id,
      geometry: a.geometry,
      color: a.color,
      stroke: a.stroke,
      zOrder: a.zOrder,
      ...(a.number !== undefined ? { number: a.number } : {}),
    }));
    const result = await requestCommit({
      crop: selection,
      annotations,
      toShelf: true,
      toClipboard: true,
      saveAs: false,
    });
    if (result.status === "err") {
      commitError = result.error.message;
      return;
    }
    // Successful commit cleans up the session so a fresh capture
    // starts with no annotations, no badge counter, and no history.
    selection = null;
    annotationStore.reset();
  }

  async function onCancel() {
    const result = await requestCancel();
    if (result.status === "ok") {
      if (result.data.action === "selection_cleared") {
        // Staged escape: selection cleared, annotations retained.
        selection = null;
      } else if (result.data.action === "session_cancelled") {
        // Full cancel: drop the in-flight annotations + draft too.
        selection = null;
        annotationStore.reset();
      }
    }
  }

  /// Tracer-05: native Save As (Ctrl+S). Opens the platform's save
  /// dialog, flattens the active crop + annotations, and writes a
  /// PNG. Cancel returns Ok(None); a write failure is surfaced as a
  /// categorical kind string.
  async function onSaveAs() {
    if (!selection) return;
    saveAsError = null;
    const annotations = annotationStore.annotations.map((a) => ({
      id: a.id,
      geometry: a.geometry,
      color: a.color,
      stroke: a.stroke,
      zOrder: a.zOrder,
      ...(a.number !== undefined ? { number: a.number } : {}),
    }));
    const suggested = `pixelgrab-${capture?.captureId?.slice(0, 8) ?? "capture"}.png`;
    const result = await saveCaptureAs({
      crop: selection,
      annotations,
      suggestedFilename: suggested,
    });
    if (result.status === "err") {
      saveAsError = result.error.message;
      lastSaveAsPath = null;
      return;
    }
    if (result.data.path) {
      lastSaveAsPath = result.data.path;
      saveAsError = null;
    } else {
      // User cancelled — session is unchanged.
      lastSaveAsPath = null;
    }
  }
</script>

<section class="overlay" data-testid="overlay">
  <header class="header">
    <span class="pill">Overlay</span>
    <span class="muted">
      {capture ? "Capture loaded" : "No capture yet"}
    </span>
    {#if lastDiagnosticsId}
      <span class="diag" data-testid="diagnostics-id">{lastDiagnosticsId}</span>
    {/if}
  </header>
  {#if capture}
    <div class="stage-wrap">
      <KonvaStage
        assetUrl={capture.assetUrl}
        bounds={capture.bounds}
        stageWidth={STAGE_WIDTH}
        stageHeight={STAGE_HEIGHT}
        {onSelectionChange}
        {onCommit}
        {onCancel}
        {onSaveAs}
      />
      <div class="toolbar-slot" data-testid="toolbar-slot">
        <AnnotationToolbar visible={selection !== null} />
      </div>
    </div>
  {:else}
    <div class="placeholder">No capture</div>
  {/if}
  {#if selection}
    <footer class="footer" data-testid="selection">
      Selection: {selection.size.width} x {selection.size.height}
      {#if annotationStore.annotations.length > 0}
        · {annotationStore.annotations.length} annotation{annotationStore.annotations.length === 1
          ? ""
          : "s"}
      {/if}
    </footer>
  {/if}
  {#if commitError}
    <footer class="error" data-testid="commit-error">{commitError}</footer>
  {/if}
  {#if saveAsError}
    <footer class="error" data-testid="save-as-error">{saveAsError}</footer>
  {/if}
  {#if lastSaveAsPath}
    <footer class="success" data-testid="save-as-success" aria-live="polite">Saved.</footer>
  {/if}
</section>

<style>
  .overlay {
    display: flex;
    flex-direction: column;
    height: 100vh;
    width: 100vw;
    background: rgba(0, 0, 0, 0.65);
    color: white;
    font-family: system-ui, sans-serif;
  }
  .header {
    padding: 0.5rem 1rem;
    display: flex;
    gap: 0.75rem;
    align-items: center;
  }
  .pill {
    background: #4f46e5;
    padding: 0.1rem 0.5rem;
    border-radius: 4px;
    font-size: 0.75rem;
  }
  .muted {
    opacity: 0.7;
    font-size: 0.85rem;
  }
  .diag {
    font-family: monospace;
    opacity: 0.7;
    font-size: 0.75rem;
  }
  .stage-wrap {
    position: relative;
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    overflow: hidden;
  }
  .toolbar-slot {
    position: absolute;
    bottom: 1rem;
    left: 50%;
    transform: translateX(-50%);
    pointer-events: auto;
  }
  .placeholder {
    flex: 1;
    display: grid;
    place-items: center;
    opacity: 0.6;
  }
  .footer {
    padding: 0.5rem 1rem;
    border-top: 1px solid rgba(255, 255, 255, 0.2);
    font-size: 0.85rem;
  }
  .error {
    color: #ffb3b3;
    padding: 0.5rem 1rem;
    border-top: 1px solid rgba(255, 80, 80, 0.4);
    font-size: 0.85rem;
  }
  .success {
    color: #b6f0c8;
    padding: 0.5rem 1rem;
    border-top: 1px solid rgba(80, 200, 120, 0.4);
    font-size: 0.85rem;
  }
</style>
