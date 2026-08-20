// Pin store. Holds the open-pin view models and exposes command helpers
// that translate user gestures into the typed IPC. The store is the
// single source of truth for the UI; the Rust registry is the single
// source of truth for the actual transform.

import type { PinCommand, PinId, PinViewModel } from "./types";

import {
  applyPinCommand,
  closePin,
  listPins,
  notifyPinDisplayChange,
  openPin,
  pinAction,
} from "./commands";
import type { OpenPinRequest, PinAction } from "./types";

export interface PinStoreState {
  pins: PinViewModel[];
  loading: boolean;
  lastError: string | null;
}

class PinStore {
  private state: PinStoreState = $state({ pins: [], loading: false, lastError: null });

  get current(): PinStoreState {
    return this.state;
  }

  private setState(next: Partial<PinStoreState>): void {
    this.state = { ...this.state, ...next };
  }

  upsert(view: PinViewModel): void {
    const existing = this.state.pins.findIndex((p) => p.id === view.id);
    if (existing >= 0) {
      const pins = [...this.state.pins];
      pins[existing] = view;
      this.setState({ pins });
    } else {
      this.setState({ pins: [...this.state.pins, view] });
    }
  }

  remove(id: PinId): void {
    this.setState({ pins: this.state.pins.filter((p) => p.id !== id) });
  }

  async refresh(): Promise<void> {
    this.setState({ loading: true });
    const result = await listPins();
    if (result.status === "ok") {
      this.setState({ pins: result.data, loading: false, lastError: null });
    } else {
      this.setState({ loading: false, lastError: result.error.message });
    }
  }

  async openPin(request: OpenPinRequest): Promise<PinViewModel | null> {
    const result = await openPin(request);
    if (result.status === "ok") {
      this.upsert(result.data);
      return result.data;
    }
    this.setState({ lastError: result.error.message });
    return null;
  }

  async closePin(id: PinId): Promise<void> {
    const result = await closePin(id);
    if (result.status === "ok") {
      this.remove(id);
    } else {
      this.setState({ lastError: result.error.message });
    }
  }

  async applyCommand(id: PinId, command: PinCommand): Promise<void> {
    const result = await applyPinCommand(id, command);
    if (result.status === "ok") {
      this.upsert(result.data);
    } else {
      this.setState({ lastError: result.error.message });
    }
  }

  async runAction(id: PinId, action: PinAction): Promise<void> {
    const result = await pinAction(id, action);
    if (result.status === "ok") {
      if (action === "close") {
        this.remove(id);
      } else {
        await this.refresh();
      }
    } else {
      this.setState({ lastError: result.error.message });
    }
  }

  async notifyDisplayChange(workArea: {
    origin: { x: number; y: number };
    size: { width: number; height: number };
  }): Promise<void> {
    const result = await notifyPinDisplayChange(workArea);
    if (result.status === "ok") {
      await this.refresh();
    }
  }

  // Reset the store to a clean state. Used by tests to isolate one test's
  // local state from the next.
  resetForTesting(): void {
    this.state = { pins: [], loading: false, lastError: null };
  }
}

export const pinStore = new PinStore();
