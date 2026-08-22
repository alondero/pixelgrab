/** Commit destinations exposed by the overlay keyboard shortcuts. */
export type CommitTarget = "shelf" | "clipboard";

/** Map a keyboard destination to the wire flags consumed by request_commit. */
export function commitOptions(target: CommitTarget): {
  toShelf: boolean;
  toClipboard: boolean;
} {
  return { toShelf: target === "shelf", toClipboard: true };
}
