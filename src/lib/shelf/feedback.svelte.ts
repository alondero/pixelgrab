// Short-lived feedback messages for the shelf quick actions.
// Exposed as a Svelte runes store so the ShelfQueue component can
// render an `aria-live="polite"` region and clear the message
// after a few seconds. Privacy: messages must not interpolate
// filesystem paths (the cache is the only place paths live); the
// callers pre-scrub any path before calling `flash`.

export type FeedbackKind = "success" | "error" | "info";

export interface FeedbackEntry {
  /** Monotonic ms when the entry was created. */
  at: number;
  /** Stable string label for the message; no PII or paths. */
  text: string;
  /** Visual / aural emphasis. */
  kind: FeedbackKind;
}

export function createFeedbackStore(): {
  readonly message: FeedbackEntry | null;
  flash(text: string, kind: FeedbackKind): void;
  clear(): void;
} {
  let message = $state<FeedbackEntry | null>(null);
  let timer: ReturnType<typeof setTimeout> | null = null;

  function clear() {
    if (timer !== null) {
      clearTimeout(timer);
      timer = null;
    }
    message = null;
  }

  function flash(text: string, kind: FeedbackKind) {
    if (timer !== null) {
      clearTimeout(timer);
    }
    const at = Date.now();
    message = { at, text, kind };
    timer = setTimeout(() => {
      message = null;
      timer = null;
    }, 3_500);
  }

  return {
    get message() {
      return message;
    },
    flash,
    clear,
  };
}
