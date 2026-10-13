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
const keys: [string, number, number, number?, number?][] = [
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
  ...([
    ["Apostrophe", 39, 222],
    ["Comma", 44, 188],
    ["Minus", 45, 189],
    ["Period", 46, 190],
    ["Slash", 47, 191],
    ["Semicolon", 59, 186],
    ["Equals", 61, 187],
    ["Left bracket", 91, 219],
    ["Backslash", 92, 220],
    ["Right bracket", 93, 221],
    ["Backtick", 96, 192],
    ["Numpad Enter", 0x01000005, 13],
    ["Pause", 0x01000008, 19],
    ["Print screen", 0x01000009, 44],
    ["Caps Lock", 0x01000024, 20],
    ["Num Lock", 0x01000025, 144],
    ["Scroll Lock", 0x01000026, 145],
    ["Menu", 0x01000055, 93],
    ["AltGr", 0x01001103, 165],
    ["Shift + Tab", 0x01000002, 9],
    ["Right Super", 0x01000054, 92],
    ["Compose", 0x01001120, -1],
    ["Help", 0x01000058, -1],
    ["Volume down", 0x01000070, 174],
    ["Mute", 0x01000071, 173],
    ["Volume up", 0x01000072, 175],
    ["Media play", 0x01000080, -1],
    ["Media stop", 0x01000081, 178],
    ["Previous track", 0x01000082, 177],
    ["Next track", 0x01000083, 176],
    ["Media record", 0x01000084, -1],
    ["Media play / pause", 0x01000086, 179],
    ["Rewind", 0x010000c5, -1],
    ["Fast forward", 0x01000102, -1],
  ] as [string, number, number][]),
  ...Array.from(
    { length: 10 },
    (_, i) =>
      [`Numpad ${i}`, 48 + i, 96 + i, 0x4000, 96 + i] as [
        string,
        number,
        number,
        number,
        number,
      ],
  ),
  ["Numpad plus", 43, 107, 0x4000, 107],
  ["Numpad minus", 45, 109, 0x4000, 109],
  ["Numpad multiply", 42, 106, 0x4000, 106],
  ["Numpad divide", 47, 111, 0x24000, 111],
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
        const identityFlags = (chord.KeyModifiers ?? 0) & ~0xf;
        const selected = keys.findIndex(
          (k) =>
            k[1] === chord.QTKeyCode &&
            (k[3] ?? 0) === identityFlags &&
            (k[4] === undefined || k[4] === chord.NativeCode),
        );
        const empty =
          [undefined, -1, 0x01ffffff].includes(chord.QTKeyCode) &&
          [undefined, -1, 0].includes(chord.VKeyCode) &&
          [undefined, -1, 0, 146].includes(chord.NativeCode) &&
          identityFlags === 0;
        function update(patch: RecordValue, standard = false) {
          const next = clone(entries);
          while (next.length <= index) next.push({});
          next[index] = { ...next[index], ...patch };
          if (standard && patch.NativeCode === undefined)
            delete next[index].NativeCode;
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
              value={
                selected >= 0
                  ? String(selected)
                  : empty
                    ? "none"
                    : "unrecognized key identity"
              }
              options={[
                ["none", "No key (modifiers only)"],
                ...keys.map(
                  ([name], i) => [String(i), name] as [string, string],
                ),
              ]}
              onChange={(value) => {
                const key = value === "none" ? undefined : keys[Number(value)];
                const patch: RecordValue = {
                  QTKeyCode: key?.[1] ?? 0x01ffffff,
                  VKeyCode: key?.[2] ?? -1,
                  KeyModifiers: key?.[3] ?? 0,
                };
                if (key?.[4] !== undefined) patch.NativeCode = key[4];
                for (const [field, , bit] of modifiers) {
                  patch[field] =
                    chord[field] ?? !!((chord.KeyModifiers ?? 0) & bit);
                  if (patch[field]) patch.KeyModifiers |= bit;
                }
                update(patch, true);
              }}
            />
            <button
              type="button"
              onClick={() =>
                update(
                  {
                    QTKeyCode: 0x01ffffff,
                    VKeyCode: -1,
                    KeyModifiers: 0,
                    KeyCtrl: false,
                    KeyShift: false,
                    KeyOption: false,
                    KeyCmd: false,
                  },
                  true,
                )
              }
            >
              Clear {label.toLowerCase()}
            </button>
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
  folderPages,
}: {
  editorKey?: string;
  folderPages?: [string, string][];
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
  if (uuid === "com.elgato.counter.action") {
    handled.add("count");
    handled.add("incrementBy");
    const increment = settings.incrementBy ?? 1;
    const custom =
      typeof increment !== "number" ||
      !Number.isInteger(increment) ||
      increment < 1 ||
      increment > 5;
    special = (
      <>
        {custom ? (
          <>
            <p className="hint">
              Custom increment: {String(increment)}. This is outside the
              sample's standard 1–5 range and is preserved.
            </p>
            <button type="button" onClick={() => set("incrementBy", 1)}>
              Use standard increment
            </button>
          </>
        ) : (
          num("incrementBy", "Increment by", 1, 1, 5)
        )}
        <p className="hint">
          Tap to increase the counter; hold to reset it. The count is runtime
          data. Custom increments remain editable in Advanced settings.
        </p>
      </>
    );
  }
  if (uuid === "com.elgato.hello-world.newstring") {
    handled.add("language");
    handled.add("titleKey");
    special = (
      <SelectField
        label="Sample language"
        value={settings.language ?? ""}
        options={[
          ["", "Host language"],
          ["zh_CN", "简体中文"],
          ["en", "English"],
          ["fr", "Français"],
          ["de", "Deutsch"],
          ["ja", "日本語"],
          ["ko", "한국어"],
          ["es", "Español"],
        ]}
        onChange={(language) => {
          const next = { ...settings };
          if (language === "") delete next.language;
          else next.language = language;
          onChange(next);
        }}
      />
    );
  }
  if (
    [
      "com.elgato.layouts.built-in-layout",
      "com.elgato.layouts.custom-layout",
      "com.elgato.layouts.feedback",
    ].includes(uuid)
  ) {
    handled.add("value");
    special = (
      <p className="hint">
        This sample has no authored settings. Turn the dial to{" "}
        {uuid.endsWith(".feedback")
          ? "adjust its displayed value"
          : "change the demonstration layout"}
        . Runtime values remain available in Advanced settings.
      </p>
    );
  }
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
              num("volume", "Volume (%)", 50, 0, 100, 0.1)}
            {settings.mode === "change_volume" &&
              num("delta", "Volume change (%)", 5, -100, 100, 0.1)}
          </>
        )}
        <p className="hint">
          Configure and enable routes in Audio mixer first. This action only
          controls existing routes.
        </p>
      </>
    );
  }

  if (uuid.endsWith(".system.website")) {
    handled.add("browser");
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
  }
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
    handled.add("Coalesce");
    special = (
      <>
        <Hotkeys settings={settings} set={set} controller={controller} />
        <p className="hint">
          Shortcuts use Linux key positions. An empty shortcut sends no keys.
          Imported Coalesce is retained but has no separate runtime behavior.
        </p>
      </>
    );
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
  if (uuid === "com.elgato.streamdeck.system.keybrightness") {
    handled.add("actionIdx");
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
  }
  if (uuid.endsWith(".system.sleep"))
    special = text(
      "DeviceUUID",
      "Device ID",
      "Leave empty for the device containing this action. Other device IDs are not supported by this action.",
    );
  if (uuid.endsWith(".soundboard.playaudio")) {
    handled.add("outputType");
    handled.add("fadeLen");
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
          settings.outputType ? "unmapped imported output" : "",
        )}
        {settings.linuxOutput === undefined && settings.outputType && (
          <p className="warning">
            The imported output has no Linux mapping. Choose an audio output,
            including System default, before playing.
          </p>
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
  }
  if (uuid === "com.elgato.streamdeck.profile.openchild")
    special = folderPages ? (
      <>
        {select("ProfileUUID", "Folder page", folderPages)}
        {!folderPages.length && (
          <p className="hint">
            Create a folder or another page in this profile first.
          </p>
        )}
      </>
    ) : (
      text(
        "ProfileUUID",
        "Folder page ID",
        "Choose an existing page in this profile. Create folder here makes a new folder and its link.",
      )
    );
  const noOptions: Record<string, string> = {
    "com.elgato.streamdeck.soundboard.stopaudioplay":
      "Stops all sounds started by Deckforge. No action-specific settings.",
    "com.elgato.streamdeck.page.next":
      "Moves to the next top-level page in this profile. No action-specific settings.",
    "com.elgato.streamdeck.page.previous":
      "Moves to the previous top-level page in this profile. No action-specific settings.",
    "com.elgato.streamdeck.page.indicator":
      "Displays the current page number. No action-specific settings.",
    "com.elgato.streamdeck.profile.backtoparent":
      "Returns from the current folder. Configure the folder default under Folder auto-exit.",
  };
  if (noOptions[uuid]) special = <p className="hint">{noOptions[uuid]}</p>;
  if (uuid.endsWith(".page.goto"))
    special = num("PageIndex", "Page number", 1, 1);
  if (uuid.endsWith(".multiactions.delay"))
    special = num("delay", "Delay (milliseconds)", 1000, 0, 86_400_000);
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
