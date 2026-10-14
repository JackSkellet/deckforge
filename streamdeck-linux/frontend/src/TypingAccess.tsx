import { request } from "./bridge";
import type { TypingStatus } from "./model";
export function TypingAccess({ status, onError }: { status?: TypingStatus; onError: (error: unknown) => void }) {
  const enable = !status || status.phase === "disabled" || status.phase === "error";
  return <section className="command-result" aria-label="Desktop typing access">
    <p role="status">{status?.message || "Desktop typing is not enabled"}</p>
    <button type="button" disabled={status?.phase === "stopping"} onClick={() => request("command", { command: enable ? "text_typing_setup" : "text_typing_disable" }).catch(onError)}>
      {enable ? "Enable desktop typing" : status?.phase === "requesting" ? "Cancel permission request" : status?.phase === "typing" ? "Stop typing" : status?.phase === "stopping" ? "Stopping typing…" : "Disable desktop typing"}
    </button>
  </section>;
}
