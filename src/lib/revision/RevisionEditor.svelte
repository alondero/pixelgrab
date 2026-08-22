<!--
  RevisionEditor - the reopen surface for a shelf capture (issue #63).

  The component is mounted by the main window when the shelf card's
  Edit action fires `open_revision` and forwards the restored
  `RevisionContext` through `pixelgrab://revision-opened`. It renders
  the restored scene summary (annotation count, badge counter, loader
  status), lets the user edit the metadata, and drives the
  commit / cancel round-trip. The heavy annotation editing itself
  stays in the overlay editor; this surface guarantees every reopen
  path can be committed or cancelled safely from the companion
  window.
-->
<script lang="ts">
  import type { CacheEntryMetadata, RevisionContext } from "$lib/ipc/types";
  import { cancelRevision, commitRevision, updateRevision } from "$lib/ipc/commands";

  let {
    scene,
    onCommitted = () => {},
    onClosed = () => {},
  }: {
    scene: RevisionContext;
    /** Fired after a successful commit with the NEW entry's shelf id. */
    onCommitted?: (newShelfId: string) => void;
    /** Fired when the editor closes (commit or cancel). */
    onClosed?: () => void;
  } = $props();

  // The editable fields intentionally capture the scene's initial
  // metadata — the reopened snapshot is a starting point for local
  // edits, not a live view (the registry owns the authoritative copy).
  // svelte-ignore state_referenced_locally
  let title = $state(scene.revision.metadata.title ?? "");
  // svelte-ignore state_referenced_locally
  let note = $state(scene.revision.metadata.note ?? "");
  // svelte-ignore state_referenced_locally
  let tagsText = $state((scene.revision.metadata.tags ?? []).join(", "));
  let busy = $state(false);
  let lastError = $state<string | null>(null);

  let annotationCount = $derived(
    scene.revision.annotations.length + (scene.revision.draft ? 1 : 0),
  );

  function currentMetadata(): CacheEntryMetadata {
    return {
      title,
      note,
      tags: tagsText
        .split(",")
        .map((tag) => tag.trim())
        .filter((tag) => tag.length > 0),
    };
  }

  // Debounced in-progress persistence: every metadata keystroke lands
  // in `revision.json` shortly after typing stops, so a crash mid-edit
  // loses at most one beat of work.
  let updateTimer: ReturnType<typeof setTimeout> | undefined;

  function scheduleUpdate(): void {
    if (updateTimer) clearTimeout(updateTimer);
    updateTimer = setTimeout(() => {
      void updateRevision({
        shelfId: scene.shelfId,
        revision: { ...scene.revision, metadata: currentMetadata() },
      });
    }, 500);
  }

  async function onCommit(): Promise<void> {
    busy = true;
    lastError = null;
    const result = await commitRevision({
      shelfId: scene.shelfId,
      annotations: scene.revision.annotations,
      badgeCounter: scene.revision.badgeCounter,
      activeTool: scene.revision.activeTool,
      activeColor: scene.revision.activeColor,
      activeStroke: scene.revision.activeStroke,
      metadata: currentMetadata(),
      toClipboard: false,
    });
    busy = false;
    if (result.status === "ok") {
      // A commit that shelved the revision always carries the new
      // entry's shelf id; a clipboard-only outcome has none.
      const newShelfId = result.data.outcome.shelfId;
      if (newShelfId) {
        onCommitted(newShelfId);
      }
      onClosed();
    } else {
      lastError = result.error.message;
    }
  }

  async function onCancel(): Promise<void> {
    busy = true;
    lastError = null;
    if (updateTimer) clearTimeout(updateTimer);
    const result = await cancelRevision({ shelfId: scene.shelfId });
    busy = false;
    if (result.status === "err") {
      lastError = result.error.message;
      return;
    }
    onClosed();
  }
</script>

<section class="editor" data-testid="revision-editor" aria-label="Capture revision editor">
  <h2>Reopened capture</h2>
  <p class="meta" data-testid="revision-loader-status">
    Scene restored: <strong>{scene.loaderStatus}</strong>
    · {annotationCount} annotation{annotationCount === 1 ? "" : "s"}
    · badge counter <span data-testid="revision-badges">{scene.revision.badgeCounter}</span>
  </p>

  <label for="revision-title">Title</label>
  <input
    id="revision-title"
    data-testid="revision-title"
    type="text"
    bind:value={title}
    oninput={scheduleUpdate}
  />

  <label for="revision-note">Note</label>
  <textarea
    id="revision-note"
    data-testid="revision-note"
    rows="3"
    bind:value={note}
    oninput={scheduleUpdate}
  ></textarea>

  <label for="revision-tags">Tags (comma separated)</label>
  <input
    id="revision-tags"
    data-testid="revision-tags"
    type="text"
    bind:value={tagsText}
    oninput={scheduleUpdate}
  />

  {#if lastError}
    <p class="error" data-testid="revision-error">{lastError}</p>
  {/if}

  <div class="actions">
    <button type="button" data-testid="revision-commit" disabled={busy} onclick={onCommit}>
      Commit revision
    </button>
    <button type="button" data-testid="revision-cancel" disabled={busy} onclick={onCancel}>
      Cancel
    </button>
  </div>
</section>

<style>
  .editor {
    border-top: 1px solid #ddd;
    margin-top: 1rem;
    padding-top: 1rem;
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    max-width: 480px;
  }
  .meta {
    color: #666;
  }
  label {
    font-weight: 600;
    margin-top: 0.4rem;
  }
  input,
  textarea {
    font: inherit;
    padding: 0.25rem 0.4rem;
  }
  .actions {
    display: flex;
    gap: 0.5rem;
    margin-top: 0.6rem;
  }
  .error {
    color: #b00020;
  }
</style>
