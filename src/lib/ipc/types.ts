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

export interface SessionSnapshot {
  state: SessionState;
  lastCapture?: CaptureResolutionDto;
  selection?: PhysicalBounds;
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
