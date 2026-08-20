// Wire-shape types for the pin IPC. Must stay in sync with
// `crates/pixelgrab-contracts/src/pin.rs`. The contract tests mirror the
// same field names and bounds.

import type { PhysicalBounds, PhysicalPoint, PhysicalSize } from "../ipc/types";

export interface PinTransform {
  position: PhysicalPoint;
  windowSize: PhysicalSize;
  sourceSize: PhysicalSize;
  /** 0.20 .. 4.00 */
  zoom: number;
  /** 0.20 .. 1.00 */
  opacity: number;
}

export type PinId = string;

export interface PinSource {
  captureId: string;
  pngPath?: string;
  bounds: PhysicalBounds;
}

export interface PinViewModel {
  id: PinId;
  transform: PinTransform;
  source: PinSource;
}

export interface OpenPinRequest {
  captureId: string;
  pngPath: string;
  bounds: PhysicalBounds;
  initialPosition?: PhysicalPoint;
}

export type PinCommand =
  | { kind: "drag"; dx: number; dy: number }
  | { kind: "zoom"; factor: number; cursorX: number; cursorY: number }
  | { kind: "setOpacity"; opacity: number }
  | { kind: "reset" };

export type PinAction = "copy" | "save_as" | "reset" | "close";

export interface PinActionOutcome {
  pinId: string;
  action: PinAction;
  bytes?: number;
  pngPath?: string;
}

export const PIN_LIMITS = {
  minZoom: 0.2,
  maxZoom: 4.0,
  minOpacity: 0.2,
  maxOpacity: 1.0,
  defaultZoom: 1.0,
  defaultOpacity: 1.0,
} as const;
