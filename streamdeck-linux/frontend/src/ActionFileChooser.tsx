import { Field } from "./Fields";
import { Modal } from "./Panels";
import type { Action, ActionFileChoice, ActionFileRoot, PendingActionFile } from "./model";
export function ActionFileChooser({ pending, choice, deviceName, actions, busy, error, onChange, onChoose, onClose }: {
  pending: PendingActionFile; choice: ActionFileChoice; deviceName: string; actions: Action[]; busy: boolean; error: string;
  onChange: (choice: ActionFileChoice) => void; onChoose: () => void; onClose: () => void;
}) {
  const compatible = (root: ActionFileRoot) => {
    if (!pending.destination) return true;
    const action = actions.find((a) => a.uuid === root.uuid);
    return action ? !action.multi_only && action.controllers.includes(pending.destination.controller) : root.controller === pending.destination.controller;
  };
  const matches = (query: string) => pending.info.roots.filter((root) => [root.name, root.uuid, root.controller, root.page].some((v) => v.toLowerCase().includes(query.toLowerCase())));
  const roots = matches(choice.query), selected = pending.info.roots.find((r) => r.index === choice.root);
  return <Modal title="Choose action from file" onClose={onClose} closeDisabled={busy}>
    <div className="action-file-choice" onKeyDown={(event) => {
      if (event.key === "Enter" && event.target instanceof HTMLInputElement) { event.preventDefault(); event.stopPropagation(); }
    }}>
      <p>This file contains {pending.info.roots.length} actions. Choose one to {pending.destination ? `import into ${pending.destination.controller === "Keypad" ? "Key" : pending.destination.controller === "Encoder" ? "Dial" : "Infobar"}${pending.destination.controller === "Neo" ? "" : " " + (pending.destination.index + 1)} on ${deviceName}` : "copy to the action clipboard"}.</p>
      <p className="hint action-file-path">{pending.info.path}</p>
      {error && <p role="alert" className="error">{error}</p>}
      <Field label="Find action"><input disabled={busy} value={choice.query} onChange={(e) => {
        const query = e.target.value, filtered = matches(query);
        onChange({ ...choice, query, root: filtered.some((r) => r.index === choice.root && compatible(r)) ? choice.root : filtered.find(compatible)?.index ?? -1 });
      }} /></Field>
      <Field label="Action in file"><select size={Math.min(8, Math.max(3, roots.length))} disabled={busy} value={choice.root} onChange={(e) => onChange({ ...choice, root: Number(e.target.value) })}>
        {roots.map((root) => <option key={root.index} value={root.index} disabled={!compatible(root)}>{root.index + 1}. {root.name} · {root.controller} {root.column + 1},{root.row + 1}{compatible(root) ? "" : " (incompatible control)"}</option>)}
      </select></Field>
      {roots.length === 0 && <p role="status">No matching actions.</p>}
      {selected && <p className="hint action-file-path">{actions.find((a) => a.uuid === selected.uuid)?.plugin_name || selected.uuid} · {selected.page_name || "Shared page"}</p>}
      <div className="modal-actions"><button type="button" disabled={busy} onClick={onClose}>Cancel</button>
        <button type="button" className="primary" disabled={busy || !selected || !roots.includes(selected) || !compatible(selected)} onClick={onChoose}>{pending.destination ? "Import action" : "Copy to clipboard"}</button></div>
    </div>
  </Modal>;
}
