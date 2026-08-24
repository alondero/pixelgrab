import { render, screen, waitFor } from "@testing-library/svelte";
import { describe, expect, it, vi, beforeEach } from "vitest";
import RevisionEditor from "./RevisionEditor.svelte";
import type { RevisionContext } from "$lib/ipc/types";

vi.mock("$lib/ipc/commands", () => ({
  updateRevision: vi.fn().mockResolvedValue({
    status: "ok",
    data: { revision: {} },
  }),
  commitRevision: vi.fn().mockResolvedValue({
    status: "ok",
    data: {
      outcome: {
        captureId: "new-capture",
        shelfId: "new-shelf",
        pngBytes: 10,
        sizeBytes: 10,
        createdAtMs: 1,
      },
    },
  }),
  cancelRevision: vi.fn().mockResolvedValue({
    status: "ok",
    data: { cancelled: true, reason: "cancelled" },
  }),
}));

import { updateRevision, commitRevision, cancelRevision } from "$lib/ipc/commands";

function makeContext(): RevisionContext {
  return {
    shelfId: "shelf-1",
    captureId: "capture-1",
    pngPath: "/cache/shelf-1/capture.png",
    locks: ["shelf"],
    loaderStatus: "full",
    revision: {
      schemaVersion: 1,
      sourceShelfId: "shelf-1",
      sourceCaptureId: "capture-1",
      crop: { origin: { x: 0, y: 0 }, size: { width: 100, height: 80 } },
      size: { width: 100, height: 80 },
      annotations: [],
      badgeCounter: 3,
      activeTool: "arrow",
      activeColor: "red",
      activeStroke: "medium",
      metadata: { title: "Old title", note: "", tags: [] },
    },
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  // `vi.clearAllMocks` wipes the resolved values, so re-prime them.
  vi.mocked(updateRevision).mockResolvedValue({
    status: "ok",
    data: { revision: {} as RevisionContext["revision"] },
  });
  vi.mocked(commitRevision).mockResolvedValue({
    status: "ok",
    data: {
      outcome: {
        captureId: "new-capture",
        shelfId: "new-shelf",
        pngBytes: 10,
        sizeBytes: 10,
        createdAtMs: 1,
      },
    },
  });
  vi.mocked(cancelRevision).mockResolvedValue({
    status: "ok",
    data: { cancelled: true, reason: "cancelled" },
  });
});

describe("RevisionEditor", () => {
  it("renders the reopened scene with the restored title and badge count", () => {
    render(RevisionEditor, { scene: makeContext() });
    const title = screen.getByTestId("revision-title") as HTMLInputElement;
    expect(title.value).toBe("Old title");
    expect(screen.getByTestId("revision-badges").textContent).toContain("3");
    expect(screen.getByTestId("revision-loader-status").textContent).toContain("full");
  });

  it("commits the revised scene with the edited metadata", async () => {
    const onCommitted = vi.fn();
    const onClosed = vi.fn();
    render(RevisionEditor, {
      scene: makeContext(),
      onCommitted,
      onClosed,
    });
    const title = screen.getByTestId("revision-title") as HTMLInputElement;
    title.value = "New title";
    title.dispatchEvent(new Event("input", { bubbles: true }));
    (screen.getByTestId("revision-commit") as HTMLButtonElement).click();
    await waitFor(() => expect(commitRevision).toHaveBeenCalledTimes(1));
    const intent = vi.mocked(commitRevision).mock.calls[0][0];
    expect(intent.shelfId).toBe("shelf-1");
    expect(intent.metadata.title).toBe("New title");
    await waitFor(() => expect(onCommitted).toHaveBeenCalledWith("new-shelf"));
    expect(onClosed).toHaveBeenCalled();
  });

  it("cancels without mutating anything and notifies the parent", async () => {
    const onClosed = vi.fn();
    render(RevisionEditor, { scene: makeContext(), onClosed });
    (screen.getByTestId("revision-cancel") as HTMLButtonElement).click();
    await waitFor(() => expect(cancelRevision).toHaveBeenCalledWith({ shelfId: "shelf-1" }));
    expect(commitRevision).not.toHaveBeenCalled();
    await waitFor(() => expect(onClosed).toHaveBeenCalled());
  });
});
