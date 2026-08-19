<script lang="ts">
  import type { ShelfCorner } from "$lib/ipc/types";
  import type { PreferencesStore } from "./store.svelte";
  import {
    LIFETIME_PRESETS_SECONDS,
    MAX_LIFETIME_SECONDS,
    MAX_MARGIN_PX,
    MAX_VISIBLE_CARDS,
    MIN_LIFETIME_SECONDS,
    MIN_MARGIN_PX,
    MIN_VISIBLE_CARDS,
  } from "./constants";

  // Settings panel UI. Renders controls for every user-facing
  // preference (corner picker, display dropdown, margin slider,
  // auto-dismiss toggle + lifetime presets + progress toggle,
  // visible-card stepper) and wires them through the preferences
  // store. Changes update the Rust core immediately (in-memory +
  // debounced disk write); the "Apply" button force-flushes the
  // pending debounce and reapplies the timer config so a process
  // that exits immediately afterwards cannot lose the change.
  let { store }: { store: PreferencesStore } = $props();

  // The store is the single source of truth. We read its current
  // value via the `value` getter and call `applyPatch` to update
  // it. The Rust core mirrors the in-memory state with its own
  // copy, so the disk write is debounced without blocking the UI.
  let prefs = $derived(store.value);

  const CORNERS: { value: ShelfCorner; label: string }[] = [
    { value: "top_left", label: "Top Left" },
    { value: "top_right", label: "Top Right" },
    { value: "bottom_left", label: "Bottom Left" },
    { value: "bottom_right", label: "Bottom Right" },
  ];

  function clamp(value: number, min: number, max: number): number {
    if (Number.isNaN(value)) return min;
    return Math.max(min, Math.min(max, value));
  }

  function onCorner(value: ShelfCorner) {
    void store.applyPatch({ corner: value });
  }

  function onMarginInput(value: number) {
    void store.applyPatch({ marginPx: clamp(value, MIN_MARGIN_PX, MAX_MARGIN_PX) });
  }

  function onVisibleCount(value: number) {
    void store.applyPatch({
      visibleCardCount: clamp(value, MIN_VISIBLE_CARDS, MAX_VISIBLE_CARDS),
    });
  }

  function onAutoDismissToggle(enabled: boolean) {
    void store.applyPatch({ autoDismissEnabled: enabled });
  }

  function onLifetimePreset(seconds: number) {
    void store.applyPatch({ lifetimeSeconds: seconds });
  }

  function onLifetimeInput(value: number) {
    void store.applyPatch({
      lifetimeSeconds: clamp(value, MIN_LIFETIME_SECONDS, MAX_LIFETIME_SECONDS),
    });
  }

  function onCountdownToggle(show: boolean) {
    void store.applyPatch({ showCountdown: show });
  }

  function onCommit() {
    void store.commitPreferences();
  }
</script>

<section class="panel" data-testid="shelf-settings-panel">
  <h2>Shelf settings</h2>
  {#if store.error}
    <p class="error" data-testid="settings-error">{store.error}</p>
  {/if}

  <fieldset>
    <legend>Anchor corner</legend>
    <div class="corner-grid" role="radiogroup" aria-label="Shelf corner">
      {#each CORNERS as corner (corner.value)}
        <button
          type="button"
          class="corner-button"
          aria-pressed={prefs.corner === corner.value}
          data-corner={corner.value}
          data-testid={`corner-${corner.value}`}
          onclick={() => onCorner(corner.value)}
        >
          {corner.label}
        </button>
      {/each}
    </div>
  </fieldset>

  <fieldset>
    <legend>Display</legend>
    <p class="muted">
      The shelf pins to the primary monitor when "Display" is unset. Reconnects to the named display
      apply automatically.
    </p>
    <select
      aria-label="Target display"
      value={prefs.targetMonitorId ?? ""}
      data-testid="display-picker"
      onchange={(event) => {
        const next = (event.currentTarget as HTMLSelectElement).value;
        const target = next === "" ? null : next;
        void store.applyPatch({ targetMonitorId: target });
      }}
    >
      <option value="">Primary (follow)</option>
      {#if prefs.targetMonitorId && prefs.targetMonitorId !== ""}
        <option value={prefs.targetMonitorId}>
          Pinned: {prefs.targetMonitorId}
        </option>
      {/if}
    </select>
  </fieldset>

  <fieldset>
    <legend>Margin from work-area edges (px)</legend>
    <input
      type="range"
      min={MIN_MARGIN_PX}
      max={MAX_MARGIN_PX}
      step="1"
      value={prefs.marginPx}
      data-testid="margin-slider"
      oninput={(event) => {
        const next = Number((event.currentTarget as HTMLInputElement).value);
        onMarginInput(next);
      }}
    />
    <output data-testid="margin-value">{prefs.marginPx}</output>
  </fieldset>

  <fieldset>
    <legend>Auto-dismiss</legend>
    <label class="checkbox">
      <input
        type="checkbox"
        checked={prefs.autoDismissEnabled}
        data-testid="auto-dismiss-toggle"
        onchange={(event) => onAutoDismissToggle((event.currentTarget as HTMLInputElement).checked)}
      />
      Dismiss cards after their lifetime
    </label>

    {#if prefs.autoDismissEnabled}
      <div class="lifetime-controls">
        <div class="preset-chips" role="group" aria-label="Lifetime presets">
          {#each LIFETIME_PRESETS_SECONDS as preset (preset)}
            <button
              type="button"
              class="preset-chip"
              aria-pressed={prefs.lifetimeSeconds === preset}
              data-testid={`lifetime-preset-${preset}`}
              onclick={() => onLifetimePreset(preset)}
            >
              {preset}s
            </button>
          {/each}
        </div>
        <input
          type="range"
          min={MIN_LIFETIME_SECONDS}
          max={MAX_LIFETIME_SECONDS}
          step="1"
          value={prefs.lifetimeSeconds}
          data-testid="lifetime-slider"
          oninput={(event) => {
            const next = Number((event.currentTarget as HTMLInputElement).value);
            onLifetimeInput(next);
          }}
        />
        <output data-testid="lifetime-value">{prefs.lifetimeSeconds}s</output>
      </div>
    {/if}
  </fieldset>

  <fieldset>
    <legend>Visible cards</legend>
    <input
      type="range"
      min={MIN_VISIBLE_CARDS}
      max={MAX_VISIBLE_CARDS}
      step="1"
      value={prefs.visibleCardCount}
      data-testid="visible-card-stepper"
      oninput={(event) => {
        const next = Number((event.currentTarget as HTMLInputElement).value);
        onVisibleCount(next);
      }}
    />
    <output data-testid="visible-card-value">{prefs.visibleCardCount}</output>
  </fieldset>

  <fieldset>
    <legend>Progress</legend>
    <label class="checkbox">
      <input
        type="checkbox"
        checked={prefs.showCountdown}
        data-testid="countdown-toggle"
        onchange={(event) => onCountdownToggle((event.currentTarget as HTMLInputElement).checked)}
      />
      Show countdown on each card
    </label>
  </fieldset>

  <div class="actions">
    <button type="button" data-testid="apply-button" onclick={onCommit} disabled={store.loading}>
      Apply
    </button>
    {#if store.loading}
      <span class="muted" data-testid="settings-loading">Loading…</span>
    {/if}
  </div>
</section>

<style>
  .panel {
    font-family: system-ui, sans-serif;
    padding: 1.5rem;
    max-width: 640px;
    display: grid;
    gap: 1rem;
  }
  fieldset {
    border: 1px solid #ddd;
    border-radius: 6px;
    padding: 0.75rem 1rem 1rem 1rem;
    display: grid;
    gap: 0.5rem;
  }
  legend {
    font-weight: 600;
    padding: 0 0.25rem;
  }
  .muted {
    color: #666;
    margin: 0;
    font-size: 0.875rem;
  }
  .error {
    color: #b00020;
    margin: 0;
  }
  .corner-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 0.5rem;
  }
  .corner-button {
    padding: 0.5rem 0.75rem;
    border: 1px solid #ccc;
    border-radius: 4px;
    background: #f5f5f5;
    cursor: pointer;
    font-family: inherit;
  }
  .corner-button[aria-pressed="true"] {
    background: #4ea1ff;
    color: #fff;
    border-color: #4ea1ff;
  }
  .corner-button:focus-visible {
    outline: 2px solid #4ea1ff;
    outline-offset: 2px;
  }
  .preset-chips {
    display: flex;
    gap: 0.25rem;
    flex-wrap: wrap;
  }
  .preset-chip {
    padding: 0.25rem 0.5rem;
    border: 1px solid #ccc;
    border-radius: 999px;
    background: #fafafa;
    cursor: pointer;
    font-family: inherit;
  }
  .preset-chip[aria-pressed="true"] {
    background: #4ea1ff;
    color: #fff;
    border-color: #4ea1ff;
  }
  .checkbox {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }
  .actions {
    display: flex;
    gap: 0.5rem;
    align-items: center;
  }
  button {
    font-family: inherit;
  }
</style>
