// Thin Tauri command wrappers for the pin IPC. The frontend never calls
// `invoke` directly outside this module so the tests can swap in a mock.

import { invoke } from "@tauri-apps/api/core";

import type { IpcResponse } from "../ipc/types";

import type {
  OpenPinRequest,
  PinAction,
  PinActionOutcome,
  PinCommand,
  PinViewModel,
} from "./types";

export async function openPin(request: OpenPinRequest): Promise<IpcResponse<PinViewModel>> {
  return invoke<IpcResponse<PinViewModel>>("open_pin", { request });
}

export async function closePin(pinId: string): Promise<IpcResponse<null>> {
  return invoke<IpcResponse<null>>("close_pin", { pinId });
}

export async function applyPinCommand(
  pinId: string,
  command: PinCommand,
): Promise<IpcResponse<PinViewModel>> {
  return invoke<IpcResponse<PinViewModel>>("apply_pin_command", {
    pinId,
    command,
  });
}

export async function getPin(pinId: string): Promise<IpcResponse<PinViewModel>> {
  return invoke<IpcResponse<PinViewModel>>("get_pin", { pinId });
}

export async function listPins(): Promise<IpcResponse<PinViewModel[]>> {
  return invoke<IpcResponse<PinViewModel[]>>("list_pins");
}

export async function pinAction(
  pinId: string,
  action: PinAction,
): Promise<IpcResponse<PinActionOutcome>> {
  return invoke<IpcResponse<PinActionOutcome>>("pin_action", { pinId, action });
}

export async function notifyPinDisplayChange(workArea: {
  origin: { x: number; y: number };
  size: { width: number; height: number };
}): Promise<IpcResponse<null>> {
  return invoke<IpcResponse<null>>("notify_pin_display_change", { workArea });
}
