import { Field, Check } from "./Fields";
import type { RecordValue } from "./model";

export function CommandEnvironment({ value, onChange }: { value: unknown; onChange: (rows: RecordValue[]) => void }) {
  const valid = value === undefined || (Array.isArray(value) && value.every((row) => row && typeof row === "object" && !Array.isArray(row) &&
    (row.name === undefined || typeof row.name === "string") && (row.value === undefined || typeof row.value === "string") && (row.unset === undefined || typeof row.unset === "boolean")));
  if (!valid) return <div>
    <p className="error">This imported environment configuration has an unsupported format. It has been preserved.</p>
    <button type="button" onClick={() => onChange([])}>Replace with empty environment list</button>
  </div>;
  const rows = (value ?? []) as RecordValue[];
  const update = (index: number, patch: RecordValue) => onChange(rows.map((row, i) => i === index ? { ...row, ...patch } : row));
  return <details className="command-environment">
    <summary>Environment variables{rows.length ? ` (${rows.length})` : ""}</summary>
    <p className="hint">Inherits the desktop environment. These literal overrides apply only to the command. They are saved with the action and included in profile exports.</p>
    {rows.map((row, index) => <fieldset key={index} aria-label={`Environment variable ${index + 1}`} className="environment-row">
      <div className="environment-fields">
        <Field label="Name"><input aria-label={`Variable ${index + 1} name`} value={row.name ?? ""} spellCheck={false} onChange={(e) => update(index, { name: e.target.value })} /></Field>
        <Field label="Value"><textarea aria-label={`Variable ${index + 1} value`} rows={2} disabled={row.unset === true} value={row.value ?? ""} spellCheck={false} onChange={(e) => update(index, { value: e.target.value })} /></Field>
      </div>
      <div className="row">
        <Check label={`Unset variable ${index + 1}`} checked={row.unset === true} onChange={(unset) => update(index, { unset })} />
        <button type="button" aria-label={`Remove variable ${index + 1}`} onClick={() => onChange(rows.filter((_, i) => i !== index))}>Remove</button>
      </div>
    </fieldset>)}
    <button type="button" disabled={rows.length >= 64} onClick={() => onChange([...rows, { name: "", value: "", unset: false }])}>Add environment variable</button>
    <p className="hint">A blank value sets an empty string. Unset removes an inherited variable. PATH overrides also control program lookup; use absolute directories.</p>
  </details>;
}
