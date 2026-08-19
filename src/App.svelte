<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { session } from "$lib/stores/session.svelte";
  import { requestCapture, requestCancel, getSessionSnapshot } from "$lib/ipc/commands";
  import type { CaptureDiagnostics, IpcResponse } from "$lib/ipc/types";
  import SettingsPanel from "$lib/preferences/SettingsPanel.svelte";
  import { createPreferencesStore } from "$lib/preferences/store.svelte";

  let lastCaptureId = $state<string | null>(null);
  let lastCaptureBounds = $state<string | null>(null);
  let diagnostics = $state<CaptureDiagnostics | null>(null);
  let pendingError = $state<string | null>(null);
  const preferences = createPreferencesStore();

  async function onCaptureIntent() {
    pendingError = null;
    const response = await requestCapture({ intent: "region" });
    if (response.status === "ok") {
      lastCaptureId = response.data.capture.captureId;
      lastCaptureBounds = `${response.data.capture.bounds.size.width}x${response.data.capture.bounds.size.height}`;
      diagnostics = response.data.diagnostics ?? null;
      session.setSnapshot({
        state: "ready",
        lastCapture: response.data.capture,
      });
    } else {
      pendingError = response.error.message;
    }
  }

  async function onCancelIntent() {
    pendingError = null;
    const response = await requestCancel();
    if (response.status === "ok") {
      session.setSnapshot(response.data.snapshot);
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
    void preferences.refresh();
    return () => {
      unlisten.then((fn) => fn());
    };
  });
</script>

<main class="app">
  <h1>PixelGrab</h1>
  <p class="muted">
    Tracer 02 capture pipeline. The tray, shortcut, and overlay wiring drive a real Windows region
    capture through the xcap-backed adapter; the overlay renders a dim mask, crosshair, and resize
    handles and honours Ctrl+C commit and staged Escape.
  </p>

  <section class="controls">
    <button type="button" onclick={onCaptureIntent}> Trigger capture </button>
    <button type="button" onclick={onCancelIntent}> Cancel </button>
    <button type="button" onclick={refreshSnapshot}>Refresh snapshot</button>
  </section>

  <section class="status">
    <h2>Session</h2>
    <dl>
      <dt>State</dt>
      <dd data-testid="session-state">{session.snapshot.state}</dd>
      <dt>Last capture id</dt>
      <dd data-testid="session-capture-id">
        {lastCaptureId ?? "(none)"}
      </dd>
      <dt>Last capture bounds</dt>
      <dd data-testid="session-capture-bounds">
        {lastCaptureBounds ?? "(none)"}
      </dd>
      <dt>Capture-to-overlay latency</dt>
      <dd data-testid="capture-to-overlay">
        {diagnostics?.captureToOverlayMs !== undefined
          ? `${diagnostics.captureToOverlayMs} ms`
          : "(n/a)"}
      </dd>
    </dl>
    {#if pendingError}
      <p class="error" data-testid="pending-error">{pendingError}</p>
    {/if}
  </section>

  <SettingsPanel store={preferences} />
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
