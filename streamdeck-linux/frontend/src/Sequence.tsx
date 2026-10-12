import { useState } from "react";
import type { MixerConfig } from "./Mixer";
import {
  children,
  replaceChildren,
  newChild,
  childAllowed,
  isMulti,
  isContainer,
  STACK,
  TRIGGER,
  clone,
  type RecordValue,
  type Action,
  type Slot,
} from "./model";
import {
  SettingsFields,
  NumberField,
  Check,
  SelectField,
  JsonEditor,
  Field,
} from "./Fields";
export function Sequence({
  raw,
  uuid,
  onChange,
  actions,
  outputs,
  onError,
  depth = 0,
  scope = uuid,
  runtime,
  run,
  mixer,
  device,
  encoderDisplay = true,
  folderPages,
}: {
  scope?: string;
  folderPages?: [string, string][];
  device?: string;
  encoderDisplay?: boolean;
  mixer?: MixerConfig;
  runtime?: Slot;
  run?: (p: RecordValue) => Promise<boolean>;
  raw: RecordValue;
  uuid: string;
  onChange: (v: RecordValue) => void;
  actions: Action[];
  outputs: [string, string][];
  onError: (e: unknown) => void;
  depth?: number;
}) {
  const [branch, setBranch] = useState(0);
  const [selected, setSelected] = useState<number | null>(null);
  const [add, setAdd] = useState("");
  const entries = children(raw, uuid, branch);
  const trigger = uuid === TRIGGER;
  const stack = uuid === STACK;
  const selectedEntry = selected === null ? null : entries[selected];
  const runtimeEntries = runtime
    ? runtime.uuid === TRIGGER
      ? runtime.trigger_children
      : runtime.uuid === STACK
        ? runtime.stack_children
        : (runtime.multi_children[branch] ?? [])
    : [];
  const liveChild = selectedEntry
    ? runtimeEntries.find(
        (child) =>
          child &&
          child.uuid === selectedEntry.UUID &&
          child.raw?.ActionID === selectedEntry.ActionID,
      )
    : undefined;
  function childLabel(entry: RecordValue, fallback = "Unassigned") {
    const action = actions.find((candidate) => candidate.uuid === entry.UUID);
    if (entry.Name && entry.Name !== action?.source_name) return entry.Name;
    const child = runtimeEntries.find(
      (candidate) =>
        candidate &&
        candidate.uuid === entry.UUID &&
        candidate.raw?.ActionID === entry.ActionID,
    );
    return child?.name || action?.name || entry.Name || fallback;
  }
  const compatible = actions.filter(
    (a) => childAllowed(a, uuid) && a.visible !== false,
  );
  function update(values: RecordValue[]) {
    try {
      onChange(replaceChildren(raw, uuid, values, branch));
    } catch (e) {
      onError(e);
    }
  }
  function replace(index: number, entry: RecordValue) {
    const values = clone(entries);
    while (values.length <= index) values.push({ Actions: [] });
    values[index] = entry;
    update(values);
  }
  function move(index: number, delta: number) {
    const values = clone(entries);
    const next = index + delta;
    if (next < 0 || next >= values.length) return;
    [values[index], values[next]] = [values[next], values[index]];
    update(values);
    setSelected(next);
  }
  return (
    <section
      className="sequence"
      aria-label={
        stack ? "Dial stack" : trigger ? "Action trigger" : "Action sequence"
      }
    >
      {uuid.endsWith(".routine2") && (
        <div className="segmented" aria-label="Switch sequence">
          {[0, 1].map((n) => (
            <button
              type="button"
              key={n}
              aria-pressed={branch === n}
              onClick={() => {
                setBranch(n);
                setSelected(null);
              }}
            >
              Sequence {n + 1}
            </button>
          ))}
        </div>
      )}
      {stack && (
        <div className="stack-options">
          {[
            ["ShowStackIcon", "Show stack icon", true],
            ["FastNavigation", "Hold and turn to choose", false],
          ]
            .filter(([field]) => encoderDisplay || field !== "ShowStackIcon")
            .map(([field, label, fallback]) => (
              <Check
                key={String(field)}
                label={String(label)}
                checked={raw.Settings?.[String(field)] ?? fallback}
                onChange={(value) =>
                  onChange({
                    ...raw,
                    Settings: { ...raw.Settings, [String(field)]: value },
                  })
                }
              />
            ))}
        </div>
      )}
      <div className="sequence-list">
        {(trigger
          ? Array.from({ length: 3 }, (_, i) => entries[i] ?? { Actions: [] })
          : entries
        ).map((entry, index) => (
          <div
            className={"sequence-row " + (selected === index ? "active" : "")}
            key={entry.ActionID ?? index}
          >
            <button
              type="button"
              className="sequence-select"
              onClick={() => setSelected(index)}
            >
              <span className="step-number">
                {trigger ? ["↶", "↓", "↷"][index] : index + 1}
              </span>
              <span>
                <strong>{childLabel(entry)}</strong>
                {trigger && (
                  <small>
                    {["Turn left", "Press / tap", "Turn right"][index]}
                  </small>
                )}
              </span>
            </button>
            <div className="row compact">
              {!trigger && (
                <>
                  <button
                    type="button"
                    title="Move up"
                    aria-label={`Move action ${index + 1} up`}
                    disabled={index === 0}
                    onClick={() => move(index, -1)}
                  >
                    ↑
                  </button>
                  <button
                    type="button"
                    title="Move down"
                    aria-label={`Move action ${index + 1} down`}
                    disabled={index === entries.length - 1}
                    onClick={() => move(index, 1)}
                  >
                    ↓
                  </button>
                </>
              )}
              <button
                type="button"
                aria-label={`Remove action ${index + 1}`}
                title="Remove"
                onClick={() => {
                  if (trigger) replace(index, { Actions: [] });
                  else update(entries.filter((_, i) => i !== index));
                  setSelected(null);
                }}
              >
                ×
              </button>
            </div>
          </div>
        ))}
      </div>
      {!entries.length && !trigger && (
        <p className="hint">
          {uuid.endsWith(".random")
            ? "Add actions to choose from. Each press runs one."
            : "Add actions in the order you want them to run."}
        </p>
      )}
      <div className="row add-action">
        <select
          aria-label="Action to add"
          value={add}
          onChange={(e) => setAdd(e.target.value)}
        >
          <option value="">Choose an action…</option>
          {compatible.map((a) => (
            <option key={a.uuid} value={a.uuid}>
              {a.name} · {a.plugin_name}
            </option>
          ))}
        </select>
        <button
          type="button"
          disabled={!add || (trigger && selected === null)}
          onClick={() => {
            const action = compatible.find((a) => a.uuid === add);
            if (!action) return;
            const child = newChild(action);
            if (trigger) replace(selected!, child);
            else {
              update([...entries, child]);
              setSelected(entries.length);
            }
            setAdd("");
          }}
        >
          {trigger ? "Assign" : "Add"}
        </button>
      </div>
      {trigger && selected === null && (
        <p className="hint">Select a gesture, then choose its action.</p>
      )}
      {selectedEntry?.UUID && (
        <div className="child-editor">
          <div className="section-label">
            {childLabel(selectedEntry, "Selected action")}
          </div>
          {liveChild?.has_inspector && run && (
            <button
              type="button"
              onClick={() =>
                run({ command: "open_inspector", context: liveChild.context })
              }
            >
              Plugin settings ↗
            </button>
          )}
          <Field label="Action label">
            <input
              value={selectedEntry.Name ?? ""}
              onChange={(e) =>
                replace(selected!, { ...selectedEntry, Name: e.target.value })
              }
            />
          </Field>
          {!isContainer(selectedEntry.UUID) && (
            <SettingsFields
              folderPages={folderPages}
              device={device}
              mixer={mixer}
              key={`settings:${selectedEntry.ActionID ?? selected}`}
              editorKey={`${scope}/${selectedEntry.ActionID ?? selected}`}
              uuid={selectedEntry.UUID}
              controller={stack ? "Encoder" : "Keypad"}
              settings={selectedEntry.Settings ?? {}}
              outputs={outputs}
              onError={onError}
              onChange={(settings) =>
                replace(selected!, { ...selectedEntry, Settings: settings })
              }
            />
          )}
          {isContainer(selectedEntry.UUID) && depth < 8 && (
            <Sequence
              folderPages={folderPages}
              encoderDisplay={encoderDisplay}
              device={device}
              mixer={mixer}
              key={`sequence:${selectedEntry.ActionID ?? selected}`}
              scope={`${scope}/${selectedEntry.ActionID ?? selected}`}
              runtime={liveChild ?? undefined}
              run={run}
              raw={selectedEntry}
              uuid={selectedEntry.UUID}
              actions={actions}
              outputs={outputs}
              depth={depth + 1}
              onError={onError}
              onChange={(next) => replace(selected!, next)}
            />
          )}
          {isContainer(selectedEntry.UUID) && depth >= 8 && (
            <p className="hint">
              Use the advanced action record for deeper nesting.
            </p>
          )}
          {isMulti(uuid) &&
            selectedEntry.UUID !==
              "com.elgato.streamdeck.multiactions.delay" && (
              <>
                <SelectField
                  label="State"
                  value={selectedEntry.OverrideState ?? ""}
                  options={[
                    ["", "Default (state 1)"],
                    ...(selectedEntry.States ?? []).map(
                      (state: RecordValue, i: number) =>
                        [
                          i,
                          `State ${i + 1}${state.Title ? ` · ${state.Title}` : ""}`,
                        ] as [number, string],
                    ),
                  ]}
                  onChange={(value) => {
                    const next = { ...selectedEntry };
                    if (value === "") delete next.OverrideState;
                    else next.OverrideState = Number(value);
                    replace(selected!, next);
                  }}
                />
                <Check
                  label="Custom action timing"
                  checked={selectedEntry.MultiActionDelays !== undefined}
                  onChange={(enabled) => {
                    const next = { ...selectedEntry };
                    if (enabled)
                      next.MultiActionDelays = { Delay1: 100, Delay2: 100 };
                    else delete next.MultiActionDelays;
                    replace(selected!, next);
                  }}
                />
                {selectedEntry.MultiActionDelays !== undefined ? (
                  <>
                    {(
                      [
                        ["Delay1", "Hold duration (milliseconds)"],
                        ["Delay2", "Wait after action (milliseconds)"],
                      ] as const
                    ).map(([field, label]) => (
                      <NumberField
                        key={field}
                        editorKey={`${scope}/${selectedEntry.ActionID ?? selected}`}
                        label={label}
                        value={selectedEntry.MultiActionDelays?.[field] ?? 100}
                        min={1}
                        max={100}
                        onChange={(value) =>
                          replace(selected!, {
                            ...selectedEntry,
                            MultiActionDelays: {
                              ...selectedEntry.MultiActionDelays,
                              Delay1:
                                selectedEntry.MultiActionDelays?.Delay1 ?? 100,
                              Delay2:
                                selectedEntry.MultiActionDelays?.Delay2 ?? 100,
                              [field]: value,
                            },
                          })
                        }
                      />
                    ))}
                  </>
                ) : (
                  <p className="hint">
                    Uses the plugin's timing when supplied, otherwise 100 ms
                    held and 100 ms before the next action.
                  </p>
                )}
              </>
            )}
          <JsonEditor
            key={`raw:${selectedEntry.ActionID ?? selected}`}
            editorKey={`${scope}/${selectedEntry.ActionID ?? selected}`}
            label="Advanced action record"
            value={selectedEntry}
            onChange={(next) => replace(selected!, next)}
          />
        </div>
      )}
      <JsonEditor
        editorKey={scope}
        label="Advanced container record"
        value={raw}
        onChange={onChange}
      />
    </section>
  );
}
