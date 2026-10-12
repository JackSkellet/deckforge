import { useEffect, useId, useState, type ReactNode } from "react";
import { pick, command } from "./bridge";
import { clone, type RecordValue, type Controller } from "./model";
import { getTextDraft, setTextDraft } from "./transient";
import type { MixerConfig } from "./Mixer";
export function Field({
  label,
  children,
  hint,
}: {
  label: string;
  children: ReactNode;
  hint?: string;
}) {
  return (
    <label className="field">
      <span>{label}</span>
      {children}
      {hint && <small>{hint}</small>}
    </label>
  );
}
export function NumberField({
  label,
  value,
  onChange,
  min,
  max,
  step = 1,
  editorKey,
}: {
  label: string;
  value: number;
  onChange: (n: number) => void;
  min?: number;
  max?: number;
  step?: number | "any";
  editorKey?: string;
}) {
  const storageKey = editorKey ? `${editorKey}:number:${label}` : undefined;
  const [text, setText] = useState(
    (storageKey && getTextDraft(storageKey)?.text) ?? String(value),
  );
  useEffect(() => {
    if (!storageKey || !getTextDraft(storageKey)) setText(String(value));
  }, [value, storageKey]);
  return (
    <Field label={label}>
      <input
        type="number"
        required
        value={text}
        min={min}
        max={max}
        step={step}
        onChange={(e) => {
          setText(e.target.value);
          if (
            e.target.value !== "" &&
            Number.isFinite(e.target.valueAsNumber)
          ) {
            if (storageKey) setTextDraft(storageKey, undefined);
            onChange(e.target.valueAsNumber);
          } else if (storageKey)
            setTextDraft(storageKey, {
              text: e.target.value,
              source: String(value),
            });
        }}
      />
    </Field>
  );
}
export function SelectField({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: unknown;
  options: [string | number, string][];
  onChange: (v: string) => void;
}) {
  const chosen = String(value ?? "");
  return (
    <Field label={label}>
      <select value={chosen} onChange={(e) => onChange(e.target.value)}>
        {!options.some(([v]) => String(v) === chosen) && (
          <option value={chosen}>
            {chosen ? `Imported value: ${chosen}` : "Choose…"}
          </option>
        )}
        {options.map(([v, text]) => (
          <option key={v} value={v}>
            {text}
          </option>
        ))}
      </select>
    </Field>
  );
}
export function Check({
  label,
  checked,
  onChange,
  disabled = false,
}: {
  label: string;
  checked: boolean;
  onChange: (v: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <label className="check">
      <input
        type="checkbox"
        disabled={disabled}
        checked={checked}
        onChange={(e) => onChange(e.target.checked)}
      />
      <span>{label}</span>
    </label>
  );
}
export function JsonEditor({
  value,
  onChange,
  label = "Advanced settings",
  editorKey = "anonymous",
}: {
  value: any;
  onChange: (v: any) => void;
  label?: string;
  editorKey?: string;
}) {
  const storageKey = `${editorKey}:${label}`;
  const [text, setText] = useState(
    getTextDraft(storageKey)?.text ?? JSON.stringify(value, null, 2),
  );
  const [dirty, setDirty] = useState(!!getTextDraft(storageKey));
  const [error, setError] = useState("");
  useEffect(() => {
    if (!dirty) setText(JSON.stringify(value, null, 2));
  }, [value, dirty]);
  return (
    <details className="advanced">
      <summary>
        {label}
        {dirty ? " · unapplied text" : ""}
      </summary>
      <textarea
        aria-label={label}
        spellCheck={false}
        value={text}
        rows={8}
        onChange={(e) => {
          setText(e.target.value);
          setDirty(true);
          setTextDraft(storageKey, {
            text: e.target.value,
            source: JSON.stringify(value),
          });
          setError("");
        }}
      />
      <div className="row">
        <button
          type="button"
          disabled={!dirty}
          onClick={() => {
            try {
              const next = JSON.parse(text);
              if (!next || typeof next !== "object" || Array.isArray(next))
                throw new Error("Enter a JSON object.");
              onChange(next);
              setDirty(false);
              setTextDraft(storageKey, undefined);
              setError("");
            } catch (e) {
              setError(String(e));
            }
          }}
        >
          Apply JSON to draft
        </button>
        <button
          type="button"
          disabled={!dirty}
          onClick={() => {
            setDirty(false);
            setTextDraft(storageKey, undefined);
            setError("");
            setText(JSON.stringify(value, null, 2));
          }}
        >
          Discard JSON text
        </button>
      </div>
      {dirty && (
        <p className="hint">Apply this text to include it when saving.</p>
      )}
      {error && (
        <p role="alert" className="error">
          {error}
        </p>
      )}
    </details>
  );
}
const media: [number, string][] = [
  [0, "Previous track"],
  [1, "Play / pause"],
  [2, "Next track"],
  [3, "Stop"],
  [13, "Fast forward"],
  [14, "Rewind"],
  [4, "Mute / unmute"],
  [5, "Volume up"],
  [6, "Volume down"],
];
const keys: [string, number, number][] = [
  ...Array.from(
    { length: 26 },
    (_, i) =>
      [String.fromCharCode(65 + i), 65 + i, 65 + i] as [string, number, number],
  ),
  ...Array.from(
    { length: 10 },
    (_, i) => [String(i), 48 + i, 48 + i] as [string, number, number],
  ),
  ...Array.from(
    { length: 24 },
    (_, i) =>
      [`F${i + 1}`, 0x01000030 + i, 0x70 + i] as [string, number, number],
  ),
  ["Space", 32, 32],
  ["Enter", 0x01000004, 13],
  ["Tab", 0x01000001, 9],
  ["Escape", 0x01000000, 27],
  ["Backspace", 0x01000003, 8],
  ["Insert", 0x01000006, 45],
  ["Delete", 0x01000007, 46],
  ["Home", 0x01000010, 36],
  ["End", 0x01000011, 35],
  ["Left arrow", 0x01000012, 37],
  ["Up arrow", 0x01000013, 38],
  ["Right arrow", 0x01000014, 39],
  ["Down arrow", 0x01000015, 40],
  ["Page up", 0x01000016, 33],
  ["Page down", 0x01000017, 34],
];
const modifiers: [string, string, number][] = [
  ["KeyCtrl", "Ctrl", 2],
  ["KeyShift", "Shift", 1],
  ["KeyOption", "Alt", 4],
  ["KeyCmd", "Super", 8],
];
function Hotkeys({
  settings,
  set,
  controller,
}: {
  settings: RecordValue;
  set: (key: string, value: any) => void;
  controller: Controller;
}) {
  if (settings.Hotkeys != null && !Array.isArray(settings.Hotkeys))
    return (
      <p className="hint">
        Imported shortcut structure is preserved. Edit it in advanced settings.
      </p>
    );
  const entries = settings.Hotkeys ?? [];
  return (
    <>
      {(controller === "Encoder"
        ? ["Turn left", "Turn right", "Press or tap"]
        : ["Shortcut"]
      ).map((label, index) => {
        const chord = entries[index] ?? {};
        if (typeof chord !== "object" || chord === null || Array.isArray(chord))
          return (
            <p key={index}>Imported shortcut requires advanced settings.</p>
          );
        const selected = keys.find((k) =>
          chord.QTKeyCode && ![0, 0x01ffffff].includes(chord.QTKeyCode)
            ? k[1] === chord.QTKeyCode
            : k[2] === chord.VKeyCode,
        );
        function update(patch: RecordValue, standard = false) {
          const next = clone(entries);
          while (next.length <= index) next.push({});
          next[index] = { ...next[index], ...patch };
          if (standard) delete next[index].NativeCode;
          set("Hotkeys", next);
        }
        return (
          <fieldset key={index}>
            <legend>{label}</legend>
            <div className="row">
              {modifiers.map(([field, name, bit]) => (
                <Check
                  key={field}
                  label={name}
                  checked={chord[field] ?? !!((chord.KeyModifiers ?? 0) & bit)}
                  onChange={(value) =>
                    update({
                      [field]: value,
                      KeyModifiers: value
                        ? (chord.KeyModifiers ?? 0) | bit
                        : (chord.KeyModifiers ?? 0) & ~bit,
                    })
                  }
                />
              ))}
            </div>
            <SelectField
              label="Key"
              value={selected?.[1] ?? chord.QTKeyCode ?? ""}
              options={keys.map(([name, qt]) => [qt, name])}
              onChange={(value) => {
                const key = keys.find((k) => k[1] === Number(value))!;
                const patch: RecordValue = {
                  QTKeyCode: key[1],
                  VKeyCode: key[2],
                  KeyModifiers: 0,
                };
                for (const [field, , bit] of modifiers) {
                  patch[field] =
                    chord[field] ?? !!((chord.KeyModifiers ?? 0) & bit);
                  if (patch[field]) patch.KeyModifiers |= bit;
                }
                update(patch, true);
              }}
            />
          </fieldset>
        );
      })}
    </>
  );
}
export function SettingsFields({
  uuid,
  controller,
  settings,
  onChange,
  outputs = [],
  onError,
  editorKey = uuid,
  mixer,
  device,
}: {
  editorKey?: string;
  device?: string;
  mixer?: MixerConfig;
  uuid: string;
  controller: Controller;
  settings: RecordValue;
  onChange: (s: RecordValue) => void;
  outputs?: [string, string][];
  onError: (e: unknown) => void;
}) {
  const id = useId();
  if (!settings || typeof settings !== "object" || Array.isArray(settings))
    return (
      <>
        <p className="warning">
          Imported settings have an unfamiliar structure. They are preserved
          until explicitly edited below.
        </p>
        <JsonEditor
          editorKey={editorKey}
          value={settings}
          onChange={onChange}
        />
      </>
    );
  const handled = new Set<string>();
  const set = (key: string, value: any) =>
    onChange({ ...settings, [key]: value });
  const text = (key: string, label: string, hint?: string) => {
    handled.add(key);
    return (
      <Field key={key} label={label} hint={hint}>
        <input
          value={typeof settings[key] === "string" ? settings[key] : ""}
          onChange={(e) => set(key, e.target.value)}
        />
      </Field>
    );
  };
  const num = (
    key: string,
    label: string,
    fallback: number,
    min?: number,
    max?: number,
    step = 1,
  ) => {
    handled.add(key);
    return (
      <NumberField
        editorKey={editorKey}
        key={key}
        label={label}
        value={settings[key] ?? fallback}
        min={min}
        max={max}
        step={step}
        onChange={(v) => set(key, v)}
      />
    );
  };
  const select = (
    key: string,
    label: string,
    options: [string | number, string][],
    fallback?: any,
  ) => {
    handled.add(key);
    return (
      <SelectField
        key={key}
        label={label}
        value={settings[key] ?? fallback}
        options={options}
        onChange={(v) =>
          set(key, typeof options[0]?.[0] === "number" ? Number(v) : v)
        }
      />
    );
  };
  const check = (key: string, label: string, fallback = false) => {
    handled.add(key);
    return (
      <Check
        key={key}
        label={label}
        checked={settings[key] ?? fallback}
        onChange={(v) => set(key, v)}
      />
    );
  };
  const choose = (key: string, kind: string, label: string) => (
    <button
      type="button"
      onClick={() =>
        pick(kind)
          .then((path) => {
            if (path) set(key, path);
          })
          .catch(onError)
      }
    >
      {label}
    </button>
  );
  let special: ReactNode = null;
  if (
    uuid === "com.deckforge.mixer.channel" ||
    uuid === "com.deckforge.mixer.mix"
  ) {
    for (const key of ["channel", "mix", "step", "mode", "volume", "delta"])
      handled.add(key);
    special = (
      <>
        {uuid.endsWith(".channel") &&
          select(
            "channel",
            "Input group",
            (mixer?.channels ?? []).map((c) => [c.id, c.name]),
          )}
        {select(
          "mix",
          "Mix",
          (mixer?.mixes ?? []).map((m) => [m.id, m.name]),
          "monitor",
        )}
        {controller === "Encoder" ? (
          <>
            {num("step", "Volume change per dial tick (%)", 5, 0.1, 100, 0.1)}
            <p className="hint">Press the dial to toggle mute.</p>
          </>
        ) : (
          <>
            {select(
              "mode",
              "Key action",
              [
                ["toggle_mute", "Toggle mute"],
                ["set_volume", "Set volume"],
                ["change_volume", "Change volume"],
              ],
              "toggle_mute",
            )}
            {settings.mode === "set_volume" &&
              num("volume", "Volume (%)", 50, 0, 100)}
            {settings.mode === "change_volume" &&
              num("delta", "Volume change (%)", 5, -100, 100)}
          </>
        )}
        <p className="hint">
          Configure and enable routes in Audio mixer first. This action only
          controls existing routes.
        </p>
      </>
    );
  }

  if (uuid.endsWith(".system.website"))
    special = (
      <>
        {text("path", "Website address")}
        {check("openInBrowser", "Open in a browser", true)}
        {settings.openInBrowser !== false &&
          text(
            "browser",
            "Browser application",
            "Leave empty for the desktop default.",
          )}
      </>
    );
  if (uuid.endsWith(".system.open"))
    special = (
      <>
        {text("path", "Application, file, or folder")}
        <div className="row">
          {choose("path", "file", "Choose file…")}
          {choose("path", "directory", "Choose folder…")}
        </div>
      </>
    );
  if (uuid.endsWith(".system.hotkey")) {
    handled.add("Hotkeys");
    special = <Hotkeys settings={settings} set={set} controller={controller} />;
  }
  if (uuid.endsWith(".system.multimedia"))
    special = (
      <>
        {select(
          "actionIdx",
          "Media control",
          controller === "Encoder" ? [[18, "System volume"]] : media,
        )}
        {controller === "Encoder" &&
          num(
            "linuxVolumeStep",
            "Volume change per dial tick (%)",
            5,
            0.1,
            100,
            0.1,
          )}
      </>
    );
  if (uuid === "com.elgato.streamdeck.system.keybrightness")
    special =
      controller === "Keypad" ? (
        select(
          "actionIdx",
          "Brightness action",
          [
            [0, "Increase 10%"],
            [1, "Decrease 10%"],
            [2, "100%"],
            [3, "75%"],
            [4, "50%"],
            [5, "25%"],
            [6, "0%"],
          ],
          0,
        )
      ) : (
        <p className="hint">Turn for 5% steps. Hold and turn for 1% steps.</p>
      );
  if (uuid.endsWith(".system.sleep"))
    special = text(
      "DeviceUUID",
      "Device ID",
      "Leave empty for all connected devices.",
    );
  if (uuid.endsWith(".soundboard.playaudio"))
    special = (
      <>
        {text("path", "Audio file")}
        {choose("path", "audio", "Choose audio…")}
        {select(
          "actionType",
          "Playback",
          [
            [1, "Play / stop"],
            [0, "Play / overlap"],
            [2, "Play / restart"],
            [3, "Loop / stop"],
          ],
          1,
        )}
        {num("volume", "Playback volume (%)", 50, 0, 100)}
        {select(
          "fadeType",
          "Fade",
          [
            [0, "None"],
            [1, "Fade in"],
            [2, "Fade out"],
            [3, "Fade in and out"],
          ],
          0,
        )}
        {settings.fadeType > 0 &&
          num("fadeLen", "Fade duration (seconds)", 1, 1, 5, 0.1)}
        {select(
          "linuxOutput",
          "Audio output",
          [["", "System default"], ...outputs],
          settings.outputType === "" ? "" : undefined,
        )}
        <button
          type="button"
          onClick={() =>
            command({ command: "soundboard_refresh" }, device).catch(onError)
          }
        >
          Refresh outputs
        </button>
      </>
    );
  if (uuid.endsWith(".page.goto"))
    special = num("PageIndex", "Page number", 1, 1);
  if (uuid.endsWith(".multiactions.delay"))
    special = num("delay", "Delay (milliseconds)", 1000, 0);
  return (
    <div className="settings-fields" id={id}>
      {special}
      {Object.entries(settings ?? {})
        .filter(
          ([key, value]) =>
            !handled.has(key) &&
            ["string", "number", "boolean"].includes(typeof value),
        )
        .map(([key, value]) =>
          typeof value === "boolean" ? (
            <Check
              key={key}
              label={key}
              checked={value}
              onChange={(v) => set(key, v)}
            />
          ) : typeof value === "number" ? (
            <NumberField
              editorKey={editorKey}
              key={key}
              label={key}
              value={value}
              step="any"
              onChange={(v) => set(key, v)}
            />
          ) : (
            text(key, key)
          ),
        )}
      <JsonEditor editorKey={editorKey} value={settings} onChange={onChange} />
    </div>
  );
}
