<script lang="ts">
  import { onMount } from "svelte";
  import { mockGetSessionSnapshot } from "$lib/ipc/shell.svelte";
  import { requestCommit, requestCancel } from "$lib/ipc/commands";
  import KonvaStage from "./KonvaStage.svelte";
  import type { CaptureResolutionDto, PhysicalBounds } from "$lib/ipc/types";

  let capture = $state<CaptureResolutionDto | null>(null);
  let selection = $state<PhysicalBounds | null>(null);
  let lastDiagnosticsId = $state<string | null>(null);
  let commitError = $state<string | null>(null);
  // The overlay spans the entire primary monitor. Values are tuned for the
  // tracer-02 default layout; the real multi-monitor sizing arrives in
  // tracer-04.
  const STAGE_WIDTH = 1920;
  const STAGE_HEIGHT = 1080;

  onMount(async () => {
    // The overlay does not own a tray or a session — it borrows the
    // snapshot from the main process via the real IPC. The mock is
    // only used by Vitest, which replaces the entire `$lib/ipc/commands`
    // module; in production this resolves to the Tauri-backed function.
    const response = await mockGetSessionSnapshot();
    if (response.status === "ok" && response.data.lastCapture) {
      capture = response.data.lastCapture;
      lastDiagnosticsId = response.data.lastCapture.captureId;
    }
  });

  function onSelectionChange(next: PhysicalBounds | null) {
    selection = next;
    if (!next) {
      // Selection cleared - hide commit error from the prior attempt.
      commitError = null;
    }
  }

  async function onCommit() {
    if (!selection) return;
    commitError = null;
    // Tracer 07: Enter (and Ctrl+C) commit atomically to cache +
    // clipboard + shelf card. The backend runs the two-phase commit
    // pipeline so either everything lands or nothing does. The IPC
    // layer is the same module the App uses, so the test mock that
    // swaps `$lib/ipc/commands` continues to work.
    const result = await requestCommit({
      crop: selection,
      toShelf: true,
      toClipboard: true,
      saveAs: false,
    });
    if (result.status === "err") {
      commitError = result.error.message;
      return;
    }
    selection = null;
  }

  async function onCancel() {
    const result = await requestCancel();
    if (result.status === "ok") {
      if (result.data.action === "selection_cleared") {
        selection = null;
      } else if (result.data.action === "session_cancelled") {
        selection = null;
      }
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
    <KonvaStage
      assetUrl={capture.assetUrl}
      bounds={capture.bounds}
      stageWidth={STAGE_WIDTH}
      stageHeight={STAGE_HEIGHT}
      {onSelectionChange}
      {onCommit}
      {onCancel}
    />
  {:else}
    <div class="placeholder">No capture</div>
  {/if}
  {#if selection}
    <footer class="footer" data-testid="selection">
      Selection: {selection.size.width} x {selection.size.height}
    </footer>
  {/if}
  {#if commitError}
    <footer class="error" data-testid="commit-error">{commitError}</footer>
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
</style>
