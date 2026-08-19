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

export interface CacheEntryDto {
  captureId: string;
  shelfId: string;
  pngPath: string;
  bitmapPath?: string;
  bounds: PhysicalBounds;
  size: PhysicalSize;
  sizeBytes: number;
  metadata: {
    title: string;
    note: string;
    tags: string[];
  };
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
