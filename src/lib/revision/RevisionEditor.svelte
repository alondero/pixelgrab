<script lang="ts">
  import { onMount } from "svelte";
  import type { CacheEntryMetadata, RevisionContext } from "$lib/ipc/types";
  import { cancelRevision, commitRevision } from "$lib/ipc/commands";
  import KonvaStage from "$lib/overlay/KonvaStage.svelte";
  import AnnotationToolbar from "$lib/annotation/AnnotationToolbar.svelte";
  import { annotationStore } from "$lib/annotation/store.svelte";

  let {
    scene,
    onCommitted = () => {},
    onClosed = () => {},
  }: {
    scene: RevisionContext;
    onCommitted?: (newShelfId: string) => void;
    onClosed?: () => void;
  } = $props();

  // svelte-ignore state_referenced_locally
  let title = $state(scene.revision.metadata.title ?? "");
  // svelte-ignore state_referenced_locally
  let note = $state(scene.revision.metadata.note ?? "");
  // svelte-ignore state_referenced_locally
  let tagsText = $state((scene.revision.metadata.tags ?? []).join(", "));
  let busy = $state(false);
  let lastError = $state<string | null>(null);
  let viewport: HTMLDivElement;
  let availableWidth = $state(640);
  let availableHeight = $state(280);
  const bounds = $derived({ origin: { x: 0, y: 0 }, size: scene.revision.size });
  const scale = $derived(
    Math.min(
      1,
      Math.max(1, availableWidth) / bounds.size.width,
      availableHeight / bounds.size.height,
    ),
  );
  const stageWidth = $derived(Math.max(1, Math.floor(bounds.size.width * scale)));
  const stageHeight = $derived(Math.max(1, Math.floor(bounds.size.height * scale)));

  onMount(() => {
    annotationStore.loadScene(scene.revision);
    const observer = new ResizeObserver((entries) => {
      availableWidth = entries[0]?.contentRect.width ?? 640;
    });
    observer.observe(viewport);
    const resize = () => {
      availableHeight = Math.max(100, window.innerHeight - 400);
    };
    resize();
    window.addEventListener("resize", resize);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", resize);
      annotationStore.reset();
    };
  });

  function metadata(): CacheEntryMetadata {
    return {
      title,
      note,
      tags: tagsText
        .split(",")
        .map((tag) => tag.trim())
        .filter(Boolean),
    };
  }

  async function onCommit(toClipboard = false): Promise<void> {
    if (busy || annotationStore.draft) return;
    busy = true;
    lastError = null;
    try {
      const result = await commitRevision({
        shelfId: scene.shelfId,
        annotations: $state.snapshot(annotationStore.annotations),
        badgeCounter: annotationStore.badgeCounter,
        activeTool: annotationStore.tool,
        activeColor: annotationStore.color,
        activeStroke: annotationStore.stroke,
        metadata: metadata(),
        toClipboard,
      });
      if (result.status === "ok") {
        if (result.data.outcome.shelfId) onCommitted(result.data.outcome.shelfId);
        onClosed();
      } else lastError = result.error.message;
    } catch {
      lastError = "Could not save your edit. Please try again.";
    } finally {
      busy = false;
    }
  }

  async function onCancel(): Promise<void> {
    if (busy) return;
    busy = true;
    try {
      const result = await cancelRevision({ shelfId: scene.shelfId });
      if (result.status === "err") {
        lastError = result.error.message;
        return;
      }
      onClosed();
    } catch {
      lastError = "Could not close the editor. Please try again.";
    } finally {
      busy = false;
    }
  }
</script>

<section class="editor" data-testid="revision-editor" aria-label="Screenshot editor">
  <div class="heading">
    <h2>Edit screenshot</h2>
    <span>{bounds.size.width} × {bounds.size.height}</span>
  </div>
  <div class="viewport" bind:this={viewport}>
    <div class="canvas" style:width="{stageWidth}px" style:height="{stageHeight}px">
      <KonvaStage
        assetUrl={scene.pngPath}
        {bounds}
        {stageWidth}
        {stageHeight}
        fixedSelection
        onSelectionChange={() => {}}
        onCommit={(target) => onCommit(target === "clipboard")}
        {onCancel}
      />
    </div>
  </div>
  <AnnotationToolbar visible />
  <p class="hint">Draw an annotation or use Select to move it. Ctrl+Z to undo. Ctrl+C to copy.</p>
  <details>
    <summary>Title and notes</summary>
    <div class="metadata">
      <label for="revision-title">Title</label><input
        id="revision-title"
        data-testid="revision-title"
        type="text"
        bind:value={title}
      />
      <label for="revision-note">Note</label><textarea
        id="revision-note"
        data-testid="revision-note"
        rows="2"
        bind:value={note}
      ></textarea>
      <label for="revision-tags">Tags (comma separated)</label><input
        id="revision-tags"
        data-testid="revision-tags"
        type="text"
        bind:value={tagsText}
      />
    </div>
  </details>
  {#if lastError}<p class="error" data-testid="revision-error" role="alert">{lastError}</p>{/if}
  <div class="actions">
    <button
      type="button"
      disabled={busy || annotationStore.draft !== null}
      onclick={() => onCommit(true)}>Copy and close</button
    >
    <button
      type="button"
      data-testid="revision-commit"
      disabled={busy || annotationStore.draft !== null}
      onclick={() => onCommit()}>Done</button
    >
    <button type="button" data-testid="revision-cancel" disabled={busy} onclick={onCancel}
      >Close</button
    >
  </div>
</section>

<style>
  .editor {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .heading {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }
  h2 {
    margin: 0;
    font-size: 1.15rem;
  }
  .heading span,
  .hint {
    color: #626a7b;
    font-size: 0.85rem;
  }
  .hint {
    margin: 0;
  }
  .viewport {
    width: 100%;
    display: flex;
    justify-content: center;
    background: #202431;
    border: 1px solid #b9c3d5;
    border-radius: 10px;
    padding: 16px 0;
  }
  .canvas {
    position: relative;
    overflow: hidden;
  }
  .metadata {
    display: grid;
    gap: 6px;
    padding-top: 12px;
  }
  summary {
    cursor: pointer;
  }
  input,
  textarea {
    font: inherit;
    padding: 8px;
    border: 1px solid #b9c3d5;
    border-radius: 6px;
  }
  .actions {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
  }
  button {
    font: inherit;
    padding: 9px 14px;
    cursor: pointer;
    border: 1px solid #b9c3d5;
    border-radius: 6px;
    background: white;
  }
  button:focus-visible,
  input:focus-visible,
  textarea:focus-visible,
  summary:focus-visible {
    outline: 3px solid #477fe8;
    outline-offset: 2px;
  }
  button:disabled {
    opacity: 0.5;
    cursor: wait;
  }
  .error {
    color: #a51b36;
  }
</style>
