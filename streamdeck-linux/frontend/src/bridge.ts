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
export const command = (payload: RecordValue) => request("command", payload);
export const pick = (kind: string) =>
  request<string | null>("pick_file", { kind });
