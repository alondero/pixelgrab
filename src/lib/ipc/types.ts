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

export interface RequestOverlayResult {
  snapshot: SessionSnapshot;
  diagnostics?: CaptureDiagnostics;
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

export interface RequestCaptureIntent {
  intent: "region" | "full_screen";
}

export interface RequestOverlayIntent {
  selection: PhysicalBounds;
}

export interface RequestCommitIntent {
  crop: PhysicalBounds;
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
  request: DragRequest;
  dismissOnAccepted?: boolean;
}

export interface StartShelfDragResult {
  outcome: DragOutcomeKind;
  diagnostics: DragDiagnostics;
  shouldDismiss: boolean;
}

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
