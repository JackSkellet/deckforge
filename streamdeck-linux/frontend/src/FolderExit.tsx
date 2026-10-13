import { Check, NumberField } from "./Fields";
import type { RecordValue } from "./model";

export function FolderExit({
  raw,
  parent,
  back,
  onChange,
}: {
  raw: RecordValue;
  parent?: RecordValue;
  back: boolean;
  onChange: (raw: RecordValue) => void;
}) {
  const follows = raw.AutoExit == null;
  const value = raw.AutoExit ??
    parent?.AutoExit ?? { Enabled: false, Delay: 1 };
  const set = (patch: RecordValue) =>
    onChange({
      ...raw,
      AutoExit: { ...value, ...patch },
    });
  return (
    <details className="folder-exit-options">
      <summary>Folder auto-exit</summary>
      <p className="hint">
        {back
          ? "Default for actions in this folder. The delay starts after a key is released."
          : "Return to the parent folder after using this action. Another press cancels the countdown."}
      </p>
      {!back && (
        <Check
          label="Follow folder settings"
          checked={follows}
          onChange={(checked) => {
            const next = { ...raw };
            if (checked) delete next.AutoExit;
            else next.AutoExit = { ...value };
            onChange(next);
          }}
        />
      )}
      <fieldset disabled={!back && follows}>
        <Check
          label="Automatically exit folder"
          checked={value.Enabled === true}
          onChange={(Enabled) => set({ Enabled })}
        />
        <fieldset disabled={value.Enabled !== true}>
          <NumberField
            label="Exit delay (seconds)"
            min={0}
            max={60}
            value={value.Delay ?? 0}
            onChange={(Delay) => set({ Delay })}
          />
        </fieldset>
      </fieldset>
      <p className="hint">0 seconds returns immediately after release.</p>
    </details>
  );
}
