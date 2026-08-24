<script lang="ts">
  import { onMount } from "svelte";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { session } from "$lib/stores/session.svelte";
  import { requestCapture, requestCancel, getSessionSnapshot } from "$lib/ipc/commands";
  import type { CaptureDiagnostics, IpcResponse, SecondaryLaunchIntent } from "$lib/ipc/types";
  import SettingsPanel from "$lib/preferences/SettingsPanel.svelte";
  import HotkeyPanel from "$lib/hotkey/HotkeyPanel.svelte";
  import RevisionEditor from "$lib/revision/RevisionEditor.svelte";
  import { createPreferencesStore } from "$lib/preferences/store.svelte";
  import { createHotkeyStore } from "$lib/hotkey/store.svelte";
  import type { MonitorLayout, RevisionContext } from "$lib/ipc/types";

  let lastCaptureId = $state<string | null>(null);
  let lastCaptureBounds = $state<string | null>(null);
  let diagnostics = $state<CaptureDiagnostics | null>(null);
  let pendingError = $state<string | null>(null);
  let settingsOpen = $state(false);
  // Issue #63: the scene forwarded by the shelf card's Edit action.
  // Non-null while the revision editor is open.
  let revisionScene = $state<RevisionContext | null>(null);
  const preferences = createPreferencesStore();
  const hotkeys = createHotkeyStore();

  async function onCaptureIntent(intent: "region" | "full_screen" = "region") {
    pendingError = null;
    const response = await requestCapture({ intent });
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

  // Route a SecondaryLaunchIntent through the same handlers the
  // IPC layer exposes. Tray clicks, global shortcuts, and
  // secondary launches all land here so the user sees identical
  // behaviour regardless of entry point.
  function handleSecondaryIntent(intent: SecondaryLaunchIntent) {
    switch (intent.kind) {
      case "capture_region":
        void onCaptureIntent("region");
        break;
      case "capture_full_screen":
        void onCaptureIntent("full_screen");
        break;
      case "shelf_history":
        // The shelf window is managed by the Rust core; the
        // frontend just refreshes its snapshot so the user sees
        // the latest cards.
        void refreshSnapshot();
        break;
      case "open_settings":
        settingsOpen = true;
        break;
      case "default":
        // No-op: the Rust core has already focused the window.
        break;
    }
  }

  function handlePauseToggle() {
    void hotkeys.togglePaused();
  }

  onMount(() => {
    const unlistenCapture = listen("pixelgrab://request-capture", () => {
      void onCaptureIntent("region");
    });
    const unlistenSecondary: Promise<UnlistenFn> = listen(
      "pixelgrab://secondary-launch",
      (event) => {
        handleSecondaryIntent(event.payload as SecondaryLaunchIntent);
      },
    );
    const unlistenPause = listen("pixelgrab://pause-hotkeys-toggled", () => {
      handlePauseToggle();
    });
    // Issue #63: the shelf card's Edit action forwards the reopened
    // editor scene; mounting RevisionEditor is the main window's half
    // of that hand-off.
    const unlistenRevision = listen<RevisionContext>("pixelgrab://revision-opened", (event) => {
      revisionScene = event.payload;
    });
    // Issue #63: the display watcher announces topology / DPI /
    // work-area changes; expose the resolved per-monitor scale factors
    // on the window so the packaged-app WebDriver pass (mixed-DPI
    // hardware) can assert against them.
    const unlistenDisplay = listen<MonitorLayout>("pixelgrab://display-changed", (event) => {
      (
        window as unknown as { __PIXELGRAB_SCALE_FACTORS__?: number[] }
      ).__PIXELGRAB_SCALE_FACTORS__ = event.payload.monitors.map((monitor) => monitor.scaleFactor);
    });
    refreshSnapshot();
    void preferences.refresh();
    void hotkeys.refresh();
    return () => {
      unlistenCapture.then((fn) => fn());
      unlistenSecondary.then((fn) => fn());
      unlistenPause.then((fn) => fn());
      unlistenRevision.then((fn) => fn());
      unlistenDisplay.then((fn) => fn());
    };
  });

  function openSettings() {
    settingsOpen = true;
  }
</script>

<main class="app">
  <h1>PixelGrab</h1>
  <p class="muted">
    Tracer 14 — configurable hotkeys, dynamic tray, and secondary-launch forwarding. The shortcut
    and tray-menu paths share a single intent handler so the three entry points are always
    equivalent.
  </p>

  <section class="controls">
    <button type="button" onclick={() => onCaptureIntent("region")}> Trigger capture </button>
    <button type="button" onclick={() => onCaptureIntent("full_screen")}>
      Capture full screen
    </button>
    <button type="button" onclick={onCancelIntent}> Cancel </button>
    <button type="button" onclick={refreshSnapshot}>Refresh snapshot</button>
    <button type="button" onclick={openSettings} data-testid="open-settings"> Settings </button>
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

  {#if settingsOpen}
    <SettingsPanel store={preferences} />
    <HotkeyPanel store={hotkeys} />
  {/if}

  {#if revisionScene}
    <RevisionEditor scene={revisionScene} onClosed={() => (revisionScene = null)} />
  {/if}
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
    flex-wrap: wrap;
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
