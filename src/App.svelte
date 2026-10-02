<script lang="ts">
  import { onMount } from "svelte";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { requestCapture } from "$lib/ipc/commands";
  import type { RevisionContext, SecondaryLaunchIntent } from "$lib/ipc/types";
  import SettingsPanel from "$lib/preferences/SettingsPanel.svelte";
  import HotkeyPanel from "$lib/hotkey/HotkeyPanel.svelte";
  import RevisionEditor from "$lib/revision/RevisionEditor.svelte";
  import RecentCaptures from "$lib/shelf/RecentCaptures.svelte";
  import { createPreferencesStore } from "$lib/preferences/store.svelte";
  import { createHotkeyStore } from "$lib/hotkey/store.svelte";

  let pendingError = $state<string | null>(null);
  let settingsOpen = $state(false);
  let capturing = $state(false);
  let revisionScene = $state<RevisionContext | null>(null);
  const preferences = createPreferencesStore();
  const hotkeys = createHotkeyStore();

  async function capture(intent: "region" | "full_screen") {
    if (capturing || revisionScene) return;
    capturing = true;
    pendingError = null;
    try {
      const response = await requestCapture({ intent });
      if (response.status === "err") pendingError = response.error.message;
    } catch {
      pendingError = "Capture failed. Please try again.";
    } finally {
      capturing = false;
    }
  }

  onMount(() => {
    const subscriptions: Promise<UnlistenFn>[] = [
      listen("pixelgrab://request-capture", () => {
        void capture("region");
      }),
      listen<SecondaryLaunchIntent>("pixelgrab://secondary-launch", (event) => {
        if (event.payload.kind === "open_settings" && !revisionScene) settingsOpen = true;
      }),
      listen("pixelgrab://pause-hotkeys-toggled", () => {
        void hotkeys.togglePaused();
      }),
      listen<{ message: string }>("pixelgrab://commit-failed", (event) => {
        pendingError = event.payload.message;
      }),
      listen<RevisionContext>("pixelgrab://revision-opened", (event) => {
        settingsOpen = false;
        revisionScene = event.payload;
      }),
      listen("pixelgrab://revision-closed", () => {
        revisionScene = null;
      }),
      listen("pixelgrab://show-recent", () => {
        settingsOpen = false;
      }),
    ];
    void preferences.refresh();
    void hotkeys.refresh();
    return () => {
      for (const subscription of subscriptions) void subscription.then((fn) => fn());
    };
  });
</script>

<main class="app">
  <header class="topbar">
    <div class="brand">
      <span class="logo" aria-hidden="true">P</span>
      <h1>PixelGrab</h1>
    </div>
    <button
      type="button"
      class="quiet"
      onclick={() => (settingsOpen = !settingsOpen)}
      data-testid="open-settings"
      aria-expanded={settingsOpen}
      disabled={revisionScene !== null}
    >
      {settingsOpen ? "Back to screenshots" : "Settings"}
    </button>
  </header>
  {#if pendingError}<p class="error" role="alert">{pendingError}</p>{/if}
  {#if settingsOpen}
    <SettingsPanel store={preferences} />
    <HotkeyPanel store={hotkeys} />
  {:else if revisionScene}
    {#key revisionScene.shelfId}
      <RevisionEditor scene={revisionScene} onClosed={() => (revisionScene = null)} />
    {/key}
  {:else}
    <section class="capture-bar" aria-label="Take a screenshot">
      <div>
        <h2>Capture. Drag. Keep working.</h2>
        <p>Select an area and release to see your screenshot.</p>
      </div>
      <div class="capture-actions">
        <button type="button" class="primary" onclick={() => capture("region")} disabled={capturing}
          >Capture area</button
        >
        <button type="button" onclick={() => capture("full_screen")} disabled={capturing}
          >Full screen</button
        >
      </div>
    </section>
    <RecentCaptures onEdit={(context) => (revisionScene = context)} />
    <p class="tip">
      Need to annotate first? Hold Ctrl when releasing your selection, or choose Edit before
      sharing.
    </p>
  {/if}
</main>

<style>
  :global(body) {
    margin: 0;
    background: #f4f6fb;
    color: #20283b;
    font-family: "Segoe UI", system-ui, sans-serif;
  }
  :global(button) {
    font: inherit;
  }
  .app {
    padding: 24px;
    max-width: 1120px;
    margin: 0 auto;
  }
  .topbar,
  .brand {
    display: flex;
    align-items: center;
  }
  .topbar {
    justify-content: space-between;
    gap: 16px;
    margin-bottom: 28px;
  }
  .brand {
    gap: 10px;
  }
  h1 {
    margin: 0;
    font-size: 1.2rem;
    letter-spacing: -0.03em;
  }
  .logo {
    display: grid;
    place-items: center;
    width: 30px;
    height: 30px;
    color: white;
    font-weight: 700;
    background: #5952dc;
    border-radius: 9px;
  }
  h2 {
    margin: 0;
    font-size: 1.4rem;
    letter-spacing: -0.03em;
  }
  p {
    line-height: 1.5;
  }
  .capture-bar {
    display: flex;
    flex-wrap: wrap;
    justify-content: space-between;
    align-items: center;
    gap: 16px;
    padding: 22px;
    margin-bottom: 28px;
    background: white;
    border: 1px solid #dee4ef;
    border-radius: 14px;
  }
  .capture-bar p {
    color: #626a7b;
    margin-bottom: 0;
  }
  .capture-actions {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
  }
  button {
    padding: 10px 14px;
    border: 1px solid #b9c3d5;
    border-radius: 8px;
    background: white;
    color: #252d41;
    cursor: pointer;
  }
  button.primary {
    background: #5952dc;
    color: white;
    border-color: #5952dc;
  }
  button.quiet {
    background: transparent;
  }
  button:focus-visible {
    outline: 3px solid #477fe8;
    outline-offset: 3px;
  }
  button:disabled {
    opacity: 0.55;
    cursor: wait;
  }
  .tip {
    color: #626a7b;
    font-size: 0.85rem;
  }
  .error {
    padding: 12px;
    border: 1px solid #e6a7b2;
    border-radius: 8px;
    color: #a51b36;
    background: #fff1f3;
  }
  @media (max-width: 420px) {
    .app {
      padding: 16px;
    }
    .capture-bar {
      padding: 16px;
    }
  }
</style>
