<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
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
  let viewport = $state({ width: window.innerWidth, height: window.innerHeight });

  // The stage fits the freeze frame's physical aspect ratio into the
  // window so the frozen desktop is never distorted, regardless of
  // the monitor layout the capture spans.
  const stageSize = $derived.by(() => {
    if (!capture) return null;
    const boundsWidth = Math.max(1, capture.bounds.size.width);
    const boundsHeight = Math.max(1, capture.bounds.size.height);
    const scale = Math.min(viewport.width / boundsWidth, viewport.height / boundsHeight);
    return {
      width: Math.max(1, Math.floor(boundsWidth * scale)),
      height: Math.max(1, Math.floor(boundsHeight * scale)),
    };
  });

  // Pull the current capture out of the session. The backend pings
  // this window (`pixelgrab://overlay-revealed`) on every reveal; the
  // heavy freeze-frame bytes are fetched here rather than shipped
  // through the event payload, so ordering can never lose a frame.
  async function pullCapture() {
    try {
      const snap = await getSessionSnapshot();
      if (snap.status === "ok" && snap.data.lastCapture) {
        capture = snap.data.lastCapture;
        lastDiagnosticsId = snap.data.lastCapture.captureId;
        // Each reveal starts fresh — drop any stale selection /
        // annotation carry-over from a previous reveal.
        selection = null;
        annotationStore.reset();
      }
    } catch {
      // IPC unavailable (browser dev / unit tests) — leave state as-is.
    }
  }

  onMount(() => {
    // Register the reveal listener BEFORE any await so a fast backend
    // reveal can never outrun the registration.
    const listening = listen("pixelgrab://overlay-revealed", () => {
      void pullCapture();
    });
    listening.catch(() => {});
    void pullCapture();
    const onResize = () => {
      viewport = { width: window.innerWidth, height: window.innerHeight };
    };
    window.addEventListener("resize", onResize);
    return () => {
      window.removeEventListener("resize", onResize);
      listening.then((fn) => fn()).catch(() => {});
    };
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
  {#if capture && stageSize}
    <div class="stage-wrap" style:width="{stageSize.width}px" style:height="{stageSize.height}px">
      <KonvaStage
        assetUrl={capture.assetUrl}
        bounds={capture.bounds}
        stageWidth={stageSize.width}
        stageHeight={stageSize.height}
        {onSelectionChange}
        {onCommit}
        {onCancel}
        {onSaveAs}
      />
      <div class="toolbar-slot" data-testid="toolbar-slot">
        <AnnotationToolbar visible={selection !== null} />
      </div>
    </div>
    <div class="hint" class:faded={selection !== null} data-testid="hint" aria-live="polite">
      Drag to select · Enter to confirm · Esc to cancel
    </div>
  {:else}
    <div class="placeholder">No capture</div>
  {/if}
  {#if lastDiagnosticsId}
    <span class="diag" data-testid="diagnostics-id">{lastDiagnosticsId}</span>
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
    /* The overlay covers the live desktop; dragging must never
       text-select the placeholder or hint UI. */
    user-select: none;
    -webkit-user-select: none;
  }
  .stage-wrap {
    position: absolute;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    display: flex;
    align-items: center;
    justify-content: center;
    overflow: hidden;
    cursor: crosshair;
  }
  .hint {
    position: fixed;
    top: 1.25rem;
    left: 50%;
    transform: translateX(-50%);
    background: rgba(15, 15, 20, 0.78);
    border: 1px solid rgba(255, 255, 255, 0.18);
    border-radius: 999px;
    padding: 0.35rem 0.9rem;
    font-size: 0.85rem;
    letter-spacing: 0.01em;
    pointer-events: none;
    transition: opacity 200ms ease;
  }
  .hint.faded {
    opacity: 0;
  }
  .diag {
    position: fixed;
    bottom: 0.25rem;
    right: 0.5rem;
    font-family: monospace;
    opacity: 0.35;
    font-size: 0.7rem;
    pointer-events: none;
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
