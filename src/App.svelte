<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { session } from "$lib/stores/session.svelte";
  import { requestCapture, getSessionSnapshot } from "$lib/ipc/commands";
  import type { CaptureResolutionDto, IpcResponse } from "$lib/ipc/types";

  let lastCapture = $state<CaptureResolutionDto | null>(null);
  let pendingError = $state<string | null>(null);

  async function onCaptureIntent() {
    pendingError = null;
    const response = await requestCapture({ intent: "region" });
    if (response.status === "ok") {
      lastCapture = response.data;
      session.setSnapshot({
        state: "ready",
        lastCapture: response.data,
      });
    } else {
      pendingError = response.error.message;
    }
  }

  async function refreshSnapshot() {
    const response: IpcResponse<typeof session.snapshot> = await getSessionSnapshot();
    if (response.status === "ok") {
      session.setSnapshot(response.data);
    }
  }

  onMount(() => {
    const unlisten = listen("pixelgrab://request-capture", () => {
      onCaptureIntent();
    });
    refreshSnapshot();
    return () => {
      unlisten.then((fn) => fn());
    };
  });
</script>

<main class="app">
  <h1>PixelGrab</h1>
  <p class="muted">
    Tracer 01 foundation. The tray, shortcut, and overlay wiring are in place; subsequent tracers
    deliver the capture pipeline and annotation experience.
  </p>

  <section class="controls">
    <button type="button" onclick={onCaptureIntent}> Trigger synthetic capture </button>
    <button type="button" onclick={refreshSnapshot}>Refresh snapshot</button>
  </section>

  <section class="status">
    <h2>Session</h2>
    <dl>
      <dt>State</dt>
      <dd data-testid="session-state">{session.snapshot.state}</dd>
      <dt>Last capture id</dt>
      <dd data-testid="session-capture-id">
        {lastCapture?.captureId ?? "(none)"}
      </dd>
      <dt>Last capture bounds</dt>
      <dd data-testid="session-capture-bounds">
        {lastCapture
          ? `${lastCapture.bounds.size.width}x${lastCapture.bounds.size.height}`
          : "(none)"}
      </dd>
    </dl>
    {#if pendingError}
      <p class="error" data-testid="pending-error">{pendingError}</p>
    {/if}
  </section>
</main>

<style>
  .app {
    font-family: system-ui, sans-serif;
    padding: 1.5rem;
    max-width: 640px;
    margin: 0 auto;
  }
  .muted {
    color: #666;
  }
  .controls {
    margin: 1rem 0;
    display: flex;
    gap: 0.5rem;
  }
  .status {
    border-top: 1px solid #ddd;
    padding-top: 1rem;
  }
  dl {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 0.25rem 1rem;
  }
  dt {
    font-weight: bold;
  }
  .error {
    color: #b00020;
  }
</style>
