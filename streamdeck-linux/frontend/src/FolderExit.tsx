import { useState } from "react";
import { Check, NumberField } from "./Fields";
import type { RecordValue } from "./model";

export function FolderExit({
  raw,
  parent,
  back,
  onChange,
  resetCount = 0,
  onReset,
}: {
  raw: RecordValue;
  parent?: RecordValue;
  back: boolean;
  onChange: (update: (raw: RecordValue) => RecordValue) => void;
  resetCount?: number;
  onReset?: () => Promise<void>;
}) {
  const [confirmReset, setConfirmReset] = useState(false);
  const [resetting, setResetting] = useState(false);
  const follows = raw.AutoExit == null;
  const value = raw.AutoExit ??
    parent?.AutoExit ?? { Enabled: false, Delay: 1 };
  const set = (patch: RecordValue) =>
    onChange((current) => ({
      ...current,
      AutoExit: { ...(current.AutoExit ?? value), ...patch },
    }));
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
            onChange((current) => {
              const next = { ...current };
              if (checked) delete next.AutoExit;
              else next.AutoExit = { ...(current.AutoExit ?? value) };
              return next;
            });
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
      {back && onReset && (
        <div className="folder-exit-reset">
          {confirmReset ? (
            <>
              <p>
                Reset {resetCount} action overrides to this folder's default?
                Pinned actions keep their shared settings.
              </p>
              <div className="row">
                <button
                  type="button"
                  disabled={resetting}
                  onClick={async () => {
                    setResetting(true);
                    try {
                      await onReset();
                      setConfirmReset(false);
                    } finally {
                      setResetting(false);
                    }
                  }}
                >
                  Reset overrides
                </button>
                <button
                  type="button"
                  disabled={resetting}
                  onClick={() => setConfirmReset(false)}
                >
                  Cancel
                </button>
              </div>
            </>
          ) : (
            <button
              type="button"
              disabled={resetCount === 0}
              onClick={() => setConfirmReset(true)}
            >
              Reset action overrides ({resetCount})
            </button>
          )}
        </div>
      )}
    </details>
  );
}
