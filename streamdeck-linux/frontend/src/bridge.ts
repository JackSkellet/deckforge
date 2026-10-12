import type { Snapshot, RecordValue } from "./model";
export interface DesktopBridge {
  request<T = any>(method: string, payload?: unknown): Promise<T>;
  onSnapshot(callback: (snapshot: Snapshot) => void): () => void;
}
declare global {
  interface Window {
    deckforge?: DesktopBridge;
  }
}
export function request<T = any>(
  method: string,
  payload?: unknown,
): Promise<T> {
  if (!window.deckforge)
    return Promise.reject(
      new Error(
        "Open Deckforge from the installed desktop application. The native connection is unavailable.",
      ),
    );
  return window.deckforge.request<T>(method, payload);
}
export const command = (payload: RecordValue, capturedDevice?: string) =>
  request("command", scopedCommand(payload, capturedDevice));
export const pick = (kind: string) =>
  request<string | null>("pick_file", { kind });

const globalCommands = new Set([
  "device_select",
  "device_add",
  "install_plugin",
  "restart_plugin",
  "plugin_applications",
  "plugin_language",
  "plugin_enabled",
  "reference_app",
  "mixer_refresh",
  "mixer_config",
]);
/** The caller passes the device captured by its UI event. Never read a mutable current-device global here. */
export function scopedCommand(
  payload: RecordValue,
  capturedDevice?: string,
): RecordValue {
  if (globalCommands.has(payload.command) || !capturedDevice) return payload;
  if (payload.device !== undefined && payload.device !== capturedDevice)
    throw new Error(
      "This command belongs to a different device. Return to its editor and retry.",
    );
  return { ...payload, device: capturedDevice };
}
