// Wire-shape types for the IPC. Must stay in sync with
// `crates/pixelgrab-contracts/src/ipc.rs`. The contract tests in
// `src/lib/ipc/types.test.ts` mirror the same assertions as the Rust-side
// integration tests.

export type CaptureFormat = "virtual_desktop" | "single_monitor" | "physical_region";

export interface PhysicalPoint {
  x: number;
  y: number;
}

export interface PhysicalSize {
  width: number;
  height: number;
}

export interface PhysicalBounds {
  origin: PhysicalPoint;
  size: PhysicalSize;
}

export interface VirtualBounds {
  min: PhysicalPoint;
  max: PhysicalPoint;
}

export interface MonitorDescriptor {
  id: string;
  label: string;
  isPrimary: boolean;
  bounds: PhysicalBounds;
  scaleFactor: number;
  workArea: PhysicalBounds;
}

export interface MonitorLayout {
  monitors: MonitorDescriptor[];
}

export type SessionState = "idle" | "capturing" | "ready" | "selecting" | "committing" | "cleanup";

export interface CaptureResolutionDto {
  format: CaptureFormat;
  bounds: PhysicalBounds;
  assetUrl: string;
  captureId: string;
  capturedAtMs: number;
}

export interface CaptureDiagnostics {
  captureId: string;
  captureStartedAtMs: number;
  captureCompletedAtMs: number;
  captureDurationMs: number;
  overlayVisibleAtMs?: number;
  captureToOverlayMs?: number;
  monitorId: string;
  bounds: PhysicalBounds;
  failureKind?: string;
}

export interface CaptureResponse {
  capture: CaptureResolutionDto;
  diagnostics?: CaptureDiagnostics;
}

export interface SessionSnapshot {
  state: SessionState;
  lastCapture?: CaptureResolutionDto;
  selection?: PhysicalBounds;
}

export interface CancelOutcome {
  action: "selection_cleared" | "session_cancelled" | "noop";
  snapshot: SessionSnapshot;
}

export interface CommitRequest {
  crop: PhysicalBounds;
  toShelf: boolean;
  toClipboard: boolean;
  saveAs: boolean;
}

export interface CommitOutcome {
  captureId: string;
  shelfId?: string;
  pngPath?: string;
  pngBytes: number;
  sizeBytes?: number;
  createdAtMs?: number;
}

export interface UpdateCacheMetadataRequest {
  shelfId: string;
  metadata: {
    title: string;
    note: string;
    tags: string[];
  };
}

export interface DismissCacheEntryRequest {
  shelfId: string;
}

export interface DismissCacheEntryResponse {
  removed: boolean;
  reason: "removed" | "still_locked" | "unknown_shelf_id";
}

export type LockOwner = "shelf" | "editor" | "drag" | "pin";

export interface ShelfSnapshot {
  entry?: CacheEntryDto;
  position?: ShelfPosition;
  locks?: LockOwner[];
}

export interface CacheEntryMetadata {
  title: string;
  note: string;
  tags: string[];
}

export interface CacheEntryDto {
  captureId: string;
  shelfId: string;
  pngPath: string;
  bitmapPath?: string;
  bounds: PhysicalBounds;
  size: PhysicalSize;
  sizeBytes: number;
  metadata: CacheEntryMetadata;
  createdAtMs: number;
  lastAccessAtMs: number;
  monitorId: string;
}

export interface ShelfPosition {
  monitorId: string;
  workArea: PhysicalBounds;
  x: number;
  y: number;
  width: number;
  height: number;
  marginPx: number;
}

export interface CommitResponse {
  outcome: CommitOutcome;
}

export interface ShelfTimerConfig {
  lifetimeMs: number;
  graceMs: number;
}

export interface ShelfTimerState {
  addedAtElapsedMs: number;
  deadlineAtElapsedMs: number;
  pausedAtElapsedMs?: number;
  pausedRemainingMs?: number;
}

export interface ShelfQueueCard {
  shelfId: string;
  captureId: string;
  pngPath: string;
  sizeBytes: number;
  createdAtMs: number;
  bounds: PhysicalBounds;
  metadata: CacheEntryMetadata;
  timer: ShelfTimerState;
}

export interface ShelfQueueSnapshot {
  cards: ShelfQueueCard[];
  overflow: ShelfQueueCard[];
  snapshotAtMs: number;
  position?: ShelfPosition;
}

export interface CopyShelfCardRequest {
  shelfId: string;
}

export interface CopyShelfCardResponse {
  pngBytes: number;
}

export interface SaveShelfCardAsRequest {
  shelfId: string;
}

export interface SaveShelfCardAsResponse {
  path?: string;
  pngBytes: number;
}

export interface HoverShelfCardRequest {
  shelfId: string;
}

export interface UnhoverShelfCardRequest {
  shelfId: string;
}

export type ShelfCorner = "top_left" | "top_right" | "bottom_left" | "bottom_right";

export interface ShelfPreferencesDto {
  schemaVersion: number;
  corner: ShelfCorner;
  targetMonitorId?: string | null;
  marginPx: number;
  autoDismissEnabled: boolean;
  lifetimeSeconds: number;
  visibleCardCount: number;
  showCountdown: boolean;
}

export interface UpdateShelfPreferencesRequest {
  preferences: ShelfPreferencesDto;
  commit?: boolean;
}

// ---------------------------------------------------------------------------
// Hotkey bindings (tracer 14). Wire shape must mirror
// `crates/pixelgrab-contracts/src/hotkey.rs`. Round-trip coverage lives in
// `src/lib/hotkey/store.test.ts` and the Rust-side contract tests.
// ---------------------------------------------------------------------------

export interface HotkeyBindingsDto {
  schemaVersion: number;
  regionCapture?: string | null;
  fullScreenCapture?: string | null;
  shelfToggle?: string | null;
  paused?: boolean;
}

export interface UpdateHotkeyBindingsRequest {
  bindings: HotkeyBindingsDto;
}

export interface HotkeyRegistryStatusDto {
  active: boolean;
  paused: boolean;
  lastError?: string;
  conflictingAction?: string;
}

// ---------------------------------------------------------------------------
// Secondary-launch intent (tracer 14). Mirrors
// `pixelgrab_contracts::SecondaryLaunchIntent`. The single-instance
// plugin emits one of these on the resident process when a secondary
// process is launched, so the frontend can route through the same
// workflow as the tray menu and shortcuts.
// ---------------------------------------------------------------------------

export type SecondaryLaunchIntent =
  | { kind: "default" }
  | { kind: "capture_region" }
  | { kind: "capture_full_screen" }
  | { kind: "shelf_history" }
  | { kind: "open_settings" };

export interface RequestCaptureIntent {
  intent: "region" | "full_screen";
}

export interface RequestCommitIntent {
  crop: PhysicalBounds;
  annotations?: Annotation[];
  toShelf: boolean;
  toClipboard: boolean;
  saveAs: boolean;
}

export type DragFormatKind = "hdrop" | "registered_png" | "dib_v5" | "unicode_text";

export interface DragFormatRequest {
  format: DragFormatKind;
  atMs: number;
}

export type DragOutcomeKind = "accepted" | "rejected" | "cancelled" | "failed";

export type DragTargetEffectKind = "copy" | "move" | "none" | "unknown";

export type DragTargetKindKind =
  | "chromium"
  | "electron"
  | "explorer"
  | "ide"
  | "rejecting"
  | "other"
  | "none";

export interface DragDiagnostics {
  startedAtMs: number;
  completedAtMs: number;
  durationMs: number;
  requestedFormats?: DragFormatRequest[];
  targetEffect: DragTargetEffectKind;
  targetKind: DragTargetKindKind;
  failureKind?: string;
  captureId: string;
  shelfId?: string;
}

export interface DragRequest {
  captureId: string;
  shelfId?: string;
  pngPath: string;
  bgraPixels: number[];
  width: number;
  height: number;
}

export interface StartShelfDragIntent {
  /** Shelf id of the card being dragged. The Rust core resolves the
   * committed entry and builds the OLE payload itself, so the heavy
   * PNG / BGRA bytes never cross the IPC boundary (issue #63). */
  shelfId: string;
  dismissOnAccepted?: boolean;
}

export interface StartShelfDragResult {
  outcome: DragOutcomeKind;
  diagnostics: DragDiagnostics;
  shouldDismiss: boolean;
}

// ---------------------------------------------------------------------------
// Annotation primitives (tracer-04). Wire shape must mirror
// `crates/pixelgrab-contracts/src/annotation.rs`. The contract tests in
// `src/lib/ipc/types.test.ts` cover the JSON-shape half; the
// semantics are covered by the store + history tests in
// `src/lib/annotation/store.svelte.test.ts`.
// ---------------------------------------------------------------------------

export type AnnotationColor = "red" | "green" | "blue" | "yellow" | "white";

export type AnnotationStroke = "thin" | "medium" | "thick";

export type AnnotationKind = "arrow" | "rectangle" | "numbered_badge" | "text" | "blur";

export type AnnotationGeometry =
  | { kind: "arrow"; tail: PhysicalPoint; tip: PhysicalPoint }
  | { kind: "rectangle"; origin: PhysicalPoint; size: PhysicalSize }
  | { kind: "numbered_badge"; center: PhysicalPoint; radius: number }
  | { kind: "text"; origin: PhysicalPoint; size: PhysicalSize; text: string }
  | { kind: "blur"; origin: PhysicalPoint; size: PhysicalSize; radius: number };

export interface Annotation {
  id: number;
  geometry: AnnotationGeometry;
  color: AnnotationColor;
  stroke: AnnotationStroke;
  zOrder: number;
  number?: number;
}

export type AnnotationTool = "select" | "arrow" | "rectangle" | "numbered_badge" | "text" | "blur";

export interface SaveCaptureAsRequest {
  crop: PhysicalBounds;
  annotations: Annotation[];
  suggestedFilename: string;
}

export interface SaveCaptureAsResponse {
  path?: string;
  pngBytes: number;
}

// ---------------------------------------------------------------------------
// Tracer-10: reopen / non-destructive revision IPC payloads.
// ---------------------------------------------------------------------------

/**
 * Wire shape for the `open_revision` IPC. The frontend sends the
 * shelf id of the entry to reopen; the Rust core acquires the
 * `Editor` lock, reads the `revision.json` sidecar (or falls back
 * to the flat PNG when the sidecar is missing / unparseable / has
 * an unsupported version), and returns a {@link RevisionContext}.
 */
export interface OpenRevisionIntent {
  /** Shelf card id of the entry to reopen. */
  shelfId: string;
}

/** Wire shape for the `open_revision` IPC response. */
export interface OpenRevisionResult {
  /** The restored editor scene (PNG path + revision metadata + locks). */
  context: RevisionContext;
}

/**
 * Wire shape for the `commit_revision` IPC. The frontend assembles
 * the final annotation list + style state + badge counter and the
 * Rust core re-runs the two-phase commit to produce a new cache
 * entry whose `captureId` is distinct from the source. The source
 * entry's assets remain untouched — the issue's "Cancellation does
 * not mutate original assets" and "Commit creates a distinct
 * revised capture and card" acceptance criteria.
 */
export interface CommitRevisionIntent {
  /** Shelf id of the source entry (the one being revised). */
  shelfId: string;
  /** Final annotation list to flatten onto the source PNG. */
  annotations: Annotation[];
  /** Badge counter at the moment of commit. */
  badgeCounter: number;
  /** Active draw tool at the moment of commit. */
  activeTool: AnnotationTool;
  /** Active color at the moment of commit. */
  activeColor: AnnotationColor;
  /** Active stroke width at the moment of commit. */
  activeStroke: AnnotationStroke;
  /** Updated user-authored metadata (title / note / tags). */
  metadata: CacheEntryMetadata;
  /** Whether to copy the revised PNG to the clipboard. */
  toClipboard: boolean;
}

/** Wire shape for the `commit_revision` IPC response. */
export interface CommitRevisionResult {
  /** The new entry's commit outcome (the `shelfId` is the NEW entry's). */
  outcome: CommitOutcome;
}

/**
 * Wire shape for the `cancel_revision` IPC. The frontend sends the
 * shelf id of the entry whose `Editor` lock should be released.
 */
export interface CancelRevisionIntent {
  /** Shelf id of the entry whose reopen session should be cancelled. */
  shelfId: string;
}

/** Wire shape for the `cancel_revision` IPC response. */
export interface CancelRevisionResult {
  /** `true` when the `Editor` lock was released. */
  cancelled: boolean;
  /** Stable diagnostic label. One of `"cancelled"`, `"no_active_revision"`. */
  reason: string;
}

/**
 * Wire shape for the `update_revision` IPC. Optional path for
 * persisting the in-progress editor scene to the source entry's
 * `revision.json` without committing. The frontend drives this from
 * a debounced handler on every annotation change.
 */
export interface UpdateRevisionIntent {
  /** Shelf id of the entry whose in-progress revision is being written. */
  shelfId: string;
  /** New revision metadata to persist. */
  revision: RevisionMetadata;
}

/** Wire shape for the `update_revision` IPC response. */
export interface UpdateRevisionResult {
  /** The persisted revision metadata (sanitized so the schema_version is the current one). */
  revision: RevisionMetadata;
}

/**
 * Editor scene persisted alongside every cache entry. Round-trips
 * across the `open_revision` IPC so the user can resume a previous
 * edit. Silently upgrades to a fresh editor when the sidecar is
 * missing or unparseable.
 */
export interface RevisionMetadata {
  /** Schema version. Pinned to 1 by the Rust core. */
  schemaVersion: number;
  /** Shelf id of the entry that authored this revision. */
  sourceShelfId: string;
  /** Capture id of the source entry. */
  sourceCaptureId: string;
  /** Final physical crop used to render the entry's PNG. */
  crop: PhysicalBounds;
  /** Pixel size of the frozen crop. */
  size: PhysicalSize;
  /** Restored annotation list. */
  annotations: Annotation[];
  /** Next badge number to assign (preserved across the reopen). */
  badgeCounter: number;
  /** In-flight draft (pointerdown + drag). */
  draft?: Annotation;
  /** Active draw tool. */
  activeTool: AnnotationTool;
  /** Active color. */
  activeColor: AnnotationColor;
  /** Active stroke width. */
  activeStroke: AnnotationStroke;
  /** User-authored metadata (title / note / tags). */
  metadata: CacheEntryMetadata;
}

/**
 * The wire shape returned by `open_revision`. Paired with the
 * Rust mirror in `crates/pixelgrab-contracts/src/revision.rs` and
 * the contract tests in `src-tauri/tests/ipc_contracts.rs`.
 */
export interface RevisionContext {
  /** Shelf id of the source entry. */
  shelfId: string;
  /** Capture id of the source entry. */
  captureId: string;
  /** Absolute path to the source entry's flattened PNG. */
  pngPath: string;
  /** The restored editor scene. */
  revision: RevisionMetadata;
  /** Active lock owners on the source entry. */
  locks: LockOwner[];
  /** Stable diagnostic label describing the loader's path. */
  loaderStatus: RevisionLoaderStatus;
}

/** Diagnostic label describing how the revision was resolved. */
export type RevisionLoaderStatus = "full" | "flat_fallback";

export interface PlatformErrorKind {
  kind:
    | "capture_unavailable"
    | "monitor_query_failed"
    | "invalid_session_state"
    | "coordinate_transform"
    | "io"
    | "invalid_payload"
    | "singleton_conflict"
    | "unsupported"
    | "internal";
  message: string;
  context?: Record<string, string>;
}

export type IpcResponseOk<T> = { status: "ok"; data: T };
export type IpcResponseErr = { status: "err"; error: PlatformErrorKind };
export type IpcResponse<T> = IpcResponseOk<T> | IpcResponseErr;

// Re-export the shelf card view so callers that already import from
// `$lib/ipc/types` can use the event payload without a second import.
export type { ShelfCardView } from "$lib/shelf/types";
