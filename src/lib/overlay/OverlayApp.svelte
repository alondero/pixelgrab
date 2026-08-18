<script lang="ts">
  import { onMount } from "svelte";
  import { mockGetSessionSnapshot } from "$lib/ipc/shell.svelte";
  import KonvaStage from "./KonvaStage.svelte";
  import type { CaptureResolutionDto, PhysicalBounds } from "$lib/ipc/types";

  let capture = $state<CaptureResolutionDto | null>(null);
  let selection = $state<PhysicalBounds | null>(null);

  onMount(async () => {
    const response = await mockGetSessionSnapshot();
    if (response.status === "ok" && response.data.lastCapture) {
      capture = response.data.lastCapture;
    }
  });

  function onSelectionChange(next: PhysicalBounds | null) {
    selection = next;
  }
</script>

<section class="overlay">
  <header class="header">
    <span class="pill">Overlay</span>
    <span class="muted">
      {capture ? "Capture loaded" : "No capture yet"}
    </span>
  </header>
  {#if capture}
    <KonvaStage assetUrl={capture.assetUrl} bounds={capture.bounds} {onSelectionChange} />
  {:else}
    <div class="placeholder">No capture</div>
  {/if}
  {#if selection}
    <footer class="footer" data-testid="selection">
      Selection: {selection.size.width} x {selection.size.height}
    </footer>
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
</style>
