import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { request } from "./bridge";
import { Modal } from "./Panels";
import type { CommandRun, Slot } from "./model";

type Preview = { summary: CommandRun; stdout: string; stderr: string; preview_truncated: boolean };
export function CommandOutput({ slot, device }: { slot: Slot; device: string }) {
  const summary = slot.command_run;
  const [preview, setPreview] = useState<Preview | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const version = useRef(0);
  useEffect(() => {
    version.current++; setPreview(null); setError(""); setBusy(false);
    return () => { version.current++; };
  }, [slot.context, summary?.id]);
  if (!summary) return null;
  const close = () => { version.current++; setPreview(null); setBusy(false); };
  async function perform(operation: "show" | "command_cancel" | "command_copy_output") {
    if (!summary) return;
    const token = ++version.current;
    setBusy(true); setError("");
    const payload = { device, context: slot.context, id: summary.id };
    try {
      if (operation === "show") {
        const result = await request<Preview>("command_output", payload);
        if (token === version.current) setPreview(result);
      } else await request("command", { ...payload, command: operation });
    } catch (e) {
      if (token === version.current) setError(e instanceof Error ? e.message : String(e));
    } finally { if (token === version.current) setBusy(false); }
  }
  return <section className="command-result" aria-label="Last command result">
    <p role="status">{summary.message}</p>
    <p className="hint">Stdout: {summary.stdout_bytes.toLocaleString()} bytes · Stderr: {summary.stderr_bytes.toLocaleString()} bytes{summary.truncated ? " · Output truncated" : ""}</p>
    {error && <p role="alert" className="error">{error}</p>}
    <div className="row">
      {summary.state === "running" ? <button type="button" disabled={busy} onClick={() => void perform("command_cancel")}>Stop command</button> : <button type="button" disabled={busy} onClick={() => void perform("show")}>Show output</button>}
      <button type="button" disabled={busy || !summary.can_copy} onClick={() => void perform("command_copy_output")}>Copy output</button>
    </div>
    {preview && createPortal(<Modal title="Command output" onClose={close}>
      <p>{preview.summary.message}</p>
      {preview.preview_truncated && <p className="hint">Preview shows the first 16 KiB of each stream. Copy output uses the complete captured stdout when available.</p>}
      <h3>Stdout</h3><pre tabIndex={0} className="command-output" aria-label="Standard output">{preview.stdout || "(empty)"}</pre>
      <h3>Stderr</h3><pre tabIndex={0} className="command-output" aria-label="Standard error">{preview.stderr || "(empty)"}</pre>
      <button type="button" onClick={close}>Close output</button>
    </Modal>, document.body)}
  </section>;
}
