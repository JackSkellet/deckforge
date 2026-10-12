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

// The native worker queue is bounded. Loading every library icon at once can
// crowd out startup/draft requests; keep only four icon reads in flight.
const iconQueues = new WeakMap<DesktopBridge, ReturnType<typeof iconQueue>>();
function iconQueue(bridge: DesktopBridge) {
  let active = 0;
  const waiting: Array<() => void> = [];
  const pending = new Map<string, Promise<unknown>>();
  const drain = () => {
    while (active < 4 && waiting.length) waiting.shift()!();
  };
  return (payload: unknown): Promise<unknown> => {
    const key = JSON.stringify(payload ?? {});
    const existing = pending.get(key);
    if (existing) return existing;
    const result = new Promise((resolve, reject) => {
      waiting.push(() => {
        active++;
        void (async () => {
          try {
            resolve(await bridge.request("action_icon", payload));
          } catch (error) {
            reject(error);
          } finally {
            pending.delete(key);
            active--;
            drain();
          }
        })();
      });
    });
    pending.set(key, result);
    drain();
    return result;
  };
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
  if (method === "action_icon") {
    let queue = iconQueues.get(window.deckforge);
    if (!queue) {
      queue = iconQueue(window.deckforge);
      iconQueues.set(window.deckforge, queue);
    }
    return queue(payload) as Promise<T>;
  }
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
  "uninstall_plugin",
  "plugin_update_apply",
  "plugin_update_cancel",
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
