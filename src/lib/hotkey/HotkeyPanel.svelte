<script lang="ts">
  import {
    canonicaliseChord,
    HOTKEY_ACTIONS,
    HOTKEY_LABELS,
    actionBinding,
    type HotkeyAction,
    type HotkeyStore,
  } from "./store.svelte";

  let { store }: { store: HotkeyStore } = $props();

  let bindings = $derived(store.value);
  let status = $derived(store.status);

  // Per-action capture state. When `capturing === action` the
  // panel is listening for the next chord the user types.
  let capturing = $state<HotkeyAction | null>(null);

  // Display name of the chord currently typed. Surfaced in the
  // preview so the user can see the canonical form before they
  // confirm.
  let preview = $state<string | null>(null);

  // Lifecycle: when the user clicks "Rebind", we register a
  // global keydown + keyup listener and let them type one chord.
  // On Escape we abort; on Enter or space we accept the current
  // preview. The global listeners are torn down on every exit
  // path so the WebView does not leak listeners.
  let onKeyDown: ((event: KeyboardEvent) => void) | null = null;
  let onKeyUp: ((event: KeyboardEvent) => void) | null = null;

  function beginCapture(action: HotkeyAction) {
    capturing = action;
    preview = null;
    onKeyDown = (event) => {
      event.preventDefault();
      const chord = chordFromEvent(event);
      if (chord) {
        preview = chord;
      }
    };
    onKeyUp = (event) => {
      if (event.key === "Escape") {
        cancelCapture();
        return;
      }
      if (preview && (event.key === "Enter" || event.key === " ")) {
        void acceptPreview();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    window.addEventListener("keyup", onKeyUp);
  }

  function cancelCapture() {
    capturing = null;
    preview = null;
    detachListeners();
  }

  function detachListeners() {
    if (onKeyDown) window.removeEventListener("keydown", onKeyDown);
    if (onKeyUp) window.removeEventListener("keyup", onKeyUp);
    onKeyDown = null;
    onKeyUp = null;
  }

  function chordFromEvent(event: KeyboardEvent): string | null {
    const parts: string[] = [];
    if (event.ctrlKey) parts.push("Control");
    if (event.metaKey) parts.push("Meta");
    if (event.altKey) parts.push("Alt");
    if (event.shiftKey) parts.push("Shift");
    // `event.key` is always set on a real KeyboardEvent; the
    // single-letter branch is taken when the pressed key is a
    // printable character. Push it normalised to upper-case so
    // the canonicaliser recognises it as the main key.
    parts.push(event.key.toUpperCase());
    return canonicaliseChord(parts);
  }

  async function acceptPreview() {
    if (capturing === null || preview === null) return;
    const action = capturing;
    const next = preview;
    cancelCapture();
    await store.setBinding(action, next);
  }

  function clearBinding(action: HotkeyAction) {
    void store.setBinding(action, null);
  }

  function labelFor(action: HotkeyAction): string {
    return HOTKEY_LABELS[action];
  }

  function bindingLabel(action: HotkeyAction): string {
    const value = actionBinding(bindings, action);
    return value ?? "unbound";
  }
</script>

<section class="panel" data-testid="hotkey-settings-panel">
  <h2>Hotkeys</h2>
  {#if status.conflictingAction}
    <p class="error" data-testid="hotkey-conflict">
      Registration failed for {HOTKEY_LABELS[status.conflictingAction as HotkeyAction] ??
        status.conflictingAction}.
      {store.error ?? "Try a different shortcut."}
    </p>
  {:else if status.lastError}
    <p class="error" data-testid="hotkey-error">{status.lastError}</p>
  {:else if store.error}
    <p class="error" data-testid="hotkey-error">{store.error}</p>
  {/if}

  <p class="status" data-testid="hotkey-status">
    {status.active
      ? "Global hotkeys are live."
      : status.paused
        ? "Global hotkeys are paused."
        : "Hotkeys are not registered. Check the conflict above."}
  </p>

  <label class="checkbox">
    <input
      type="checkbox"
      checked={bindings.paused ?? false}
      data-testid="hotkey-pause-toggle"
      onchange={() => store.togglePaused()}
    />
    Pause global hotkeys
  </label>

  <ul class="bindings">
    {#each HOTKEY_ACTIONS as action (action)}
      <li>
        <span class="action-label">{labelFor(action)}</span>
        <code class="binding" data-testid={`hotkey-binding-${action}`}>
          {bindingLabel(action)}
        </code>
        <div class="actions">
          <button
            type="button"
            data-testid={`hotkey-rebind-${action}`}
            onclick={() => beginCapture(action)}
          >
            Rebind
          </button>
          <button
            type="button"
            data-testid={`hotkey-clear-${action}`}
            onclick={() => clearBinding(action)}
          >
            Clear
          </button>
        </div>
      </li>
    {/each}
  </ul>

  {#if capturing}
    <div class="capture-overlay" role="dialog" aria-modal="true" aria-label="Rebind hotkey">
      <div class="capture-card">
        <p>
          Press the new chord for <strong>{labelFor(capturing)}</strong>. Press Enter or Space to
          confirm, Escape to cancel.
        </p>
        <code class="preview">{preview ?? "Listening…"}</code>
        <div class="actions">
          <button type="button" data-testid="hotkey-confirm" onclick={acceptPreview}> Save </button>
          <button type="button" data-testid="hotkey-cancel" onclick={cancelCapture}>
            Cancel
          </button>
        </div>
      </div>
    </div>
  {/if}
</section>

<style>
  .panel {
    font-family: system-ui, sans-serif;
    padding: 1.5rem;
    max-width: 640px;
    display: grid;
    gap: 1rem;
    border-top: 1px solid #ddd;
  }
  .bindings {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 0.5rem;
  }
  .bindings li {
    display: grid;
    grid-template-columns: 1fr auto auto;
    gap: 0.75rem;
    align-items: center;
    padding: 0.5rem;
    border-radius: 6px;
    background: #fafafa;
  }
  .action-label {
    font-weight: 600;
  }
  .binding {
    font-family: ui-monospace, "SF Mono", monospace;
    padding: 0.25rem 0.5rem;
    border-radius: 4px;
    background: #fff;
    border: 1px solid #ddd;
  }
  .actions {
    display: flex;
    gap: 0.5rem;
  }
  .capture-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.4);
    display: grid;
    place-items: center;
    z-index: 100;
  }
  .capture-card {
    background: #fff;
    padding: 1.5rem;
    border-radius: 8px;
    width: min(420px, 90%);
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.3);
    display: grid;
    gap: 0.75rem;
  }
  .preview {
    font-family: ui-monospace, "SF Mono", monospace;
    background: #f5f5f5;
    border-radius: 4px;
    padding: 0.5rem;
    text-align: center;
  }
  .status {
    margin: 0;
    color: #444;
    font-size: 0.95rem;
  }
  .error {
    color: #b00020;
    margin: 0;
  }
  .checkbox {
    display: flex;
    gap: 0.5rem;
    align-items: center;
  }
  button {
    font-family: inherit;
  }
</style>
