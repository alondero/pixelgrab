<script lang="ts">
  import { onMount } from "svelte";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import {
    getSessionSnapshot,
    requestCommit,
    requestCancel,
    saveCaptureAs,
  } from "$lib/ipc/commands";
  import KonvaStage from "./KonvaStage.svelte";
  import AnnotationToolbar from "$lib/annotation/AnnotationToolbar.svelte";
  import { annotationStore } from "$lib/annotation/store.svelte";
  import { commitOptions, type CommitTarget } from "./commitIntent";
  import type { CaptureResponse, CaptureResolutionDto, PhysicalBounds } from "$lib/ipc/types";

  let capture = $state<CaptureResolutionDto | null>(null);
  let selection = $state<PhysicalBounds | null>(null);
  let lastDiagnosticsId = $state<string | null>(null);
  let commitError = $state<string | null>(null);
  let saveAsError = $state<string | null>(null);
  let lastSaveAsPath = $state<string | null>(null);
  let committing = $state(false);
  // Konva uses CSS-pixel stage dimensions. The native overlay window is
  // sized in physical pixels, so using a fixed 1920×1080 stage silently
  // breaks mixed-DPI and non-1080p desktops. Track the actual webview
  // viewport and let Konva's layer transform map it to physical capture
  // pixels.
  let stageWidth = $state(1920);
  let stageHeight = $state(1080);

  function loadCapture(next: CaptureResolutionDto | null) {
    if (next?.captureId === capture?.captureId) return;
    capture = next;
    lastDiagnosticsId = next?.captureId ?? null;
    selection = null;
    commitError = null;
    saveAsError = null;
    lastSaveAsPath = null;
    annotationStore.reset();
  }

  onMount(() => {
    function syncViewport() {
      stageWidth = Math.max(1, window.innerWidth);
      stageHeight = Math.max(1, window.innerHeight);
    }
    syncViewport();
    window.addEventListener("resize", syncViewport);
    // Issue #60: the overlay reveal contract is collapsed into one
    // backend seam (`show_over_virtual_desktop` → `overlay_mounted`),
    // so the frontend never has to drive the `Ready -> Selecting`
    // transition. We just read the snapshot the orchestrator already
    // stamped.
    async function refreshCapture() {
      const response = await getSessionSnapshot();
      if (response.status !== "ok") return;
      // The event is authoritative once one has arrived. An initial snapshot
      // request can resolve after `capture-ready`; it must not overwrite that
      // newer capture with the pre-capture snapshot it observed.
      if (!capture) loadCapture(response.data.lastCapture ?? null);
    }

    // The overlay is pre-allocated while the app is idle. Its Svelte tree
    // therefore mounts before a capture exists; subscribe to the native
    // capture-ready event so tray/global-shortcut captures hydrate the
    // already-mounted overlay without a permanent polling loop.
    const unlistenCapture: Promise<UnlistenFn> = listen<CaptureResponse>(
      "pixelgrab://capture-ready",
      (event) => {
        loadCapture(event.payload.capture);
      },
    );
    // Install the listener before reading the initial snapshot. Otherwise a
    // capture can land between the read and listener registration, leaving
    // the pre-warmed overlay permanently empty.
    void unlistenCapture.then(() => refreshCapture());
    return () => {
      unlistenCapture.then((fn) => fn());
      window.removeEventListener("resize", syncViewport);
    };
  });

  function onSelectionChange(next: PhysicalBounds | null) {
    selection = next;
    if (!next) {
      commitError = null;
    }
  }

  async function onCommit(target: CommitTarget = "shelf") {
    if (!selection || committing) return;
    committing = true;
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
    try {
      const result = await requestCommit({
        crop: selection,
        annotations,
        ...commitOptions(target),
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
    } finally {
      committing = false;
    }
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
        {stageWidth}
        {stageHeight}
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
    position: fixed;
    inset: 0;
    display: flex;
    flex-direction: column;
    height: 100vh;
    width: 100vw;
    overflow: hidden;
    background: #11131a;
    color: white;
    font-family: system-ui, sans-serif;
  }
  .header {
    position: absolute;
    z-index: 20;
    top: 0.75rem;
    left: 0.75rem;
    padding: 0.35rem 0.6rem;
    display: flex;
    gap: 0.75rem;
    align-items: center;
    border-radius: 999px;
    background: rgba(20, 20, 28, 0.78);
    box-shadow: 0 2px 10px rgba(0, 0, 0, 0.3);
    pointer-events: none;
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
    position: absolute;
    inset: 0;
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
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    opacity: 0.6;
  }
  .footer {
    position: absolute;
    z-index: 20;
    left: 0.75rem;
    bottom: 0.75rem;
    padding: 0.5rem 1rem;
    border-radius: 6px;
    background: rgba(20, 20, 28, 0.78);
    border-top: 1px solid rgba(255, 255, 255, 0.2);
    font-size: 0.85rem;
  }
  .error {
    position: absolute;
    z-index: 21;
    left: 0.75rem;
    right: 0.75rem;
    bottom: 0.75rem;
    background: rgba(80, 20, 20, 0.92);
    border-radius: 6px;
    color: #ffb3b3;
    padding: 0.5rem 1rem;
    border-top: 1px solid rgba(255, 80, 80, 0.4);
    font-size: 0.85rem;
  }
  .success {
    position: absolute;
    z-index: 21;
    left: 0.75rem;
    bottom: 0.75rem;
    background: rgba(20, 70, 35, 0.92);
    border-radius: 6px;
    color: #b6f0c8;
    padding: 0.5rem 1rem;
    border-top: 1px solid rgba(80, 200, 120, 0.4);
    font-size: 0.85rem;
  }
</style>
