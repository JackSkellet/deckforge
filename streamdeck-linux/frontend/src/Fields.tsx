import { useEffect, useId, useState, type ReactNode } from "react";
import { pick, command, request } from "./bridge";
import { clone, type RecordValue, type Controller } from "./model";
import { getTextDraft, setTextDraft } from "./transient";
import type { MixerConfig } from "./Mixer";
import { ApplicationIcon, ApplicationPicker, type Application } from "./ApplicationPicker";
import { CommandEnvironment } from "./CommandEnvironment";
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
function OpenApplication({
  settings,
  onChange,
  onError,
}: {
  settings: RecordValue;
  onChange: (value: RecordValue) => void;
  onError: (error: unknown) => void;
}) {
  const [apps, setApps] = useState<Application[] | null>(null);
  const [search, setSearch] = useState("");
  const manual = (exec: string) =>
    onChange({
      ...settings,
      exec,
      source: "",
      bundle_id: "",
      bundle_path: "",
      is_bundle: false,
    });
  return (
    <>
      <Field label="Application executable">
        <input
          value={settings.exec ?? ""}
          onChange={(e) => manual(e.target.value)}
        />
      </Field>
      <div className="row">
        <button
          type="button"
          onClick={() =>
            request("application_catalog").then(setApps).catch(onError)
          }
        >
          Browse applications…
        </button>
        <button
          type="button"
          onClick={() =>
            pick("file")
              .then((path) => {
                if (path) manual(path);
              })
              .catch(onError)
          }
        >
          Choose executable…
        </button>
      </div>
      {apps && (
        <>
          <Field label="Search applications">
            <input value={search} onChange={(e) => setSearch(e.target.value)} />
          </Field>
          <ApplicationPicker applications={apps} search={search} value={settings.source ?? ""}
            onSelect={(app) => onChange({
              ...settings, exec: app.executable, source: app.desktop,
              bundle_id: app.desktop, bundle_path: app.desktop,
              is_bundle: true, app_name: app.name,
            })} />
        </>
      )}
      {typeof settings.source === "string" && settings.source.endsWith(".desktop") && (
        <p className="hint application-selection">
          <ApplicationIcon desktop={settings.source} name={settings.app_name || "Application"} />
          Using {settings.app_name || settings.source}'s desktop launcher.
        </p>
      )}
      <Field
        label="Arguments"
        hint={typeof settings.source === "string" && settings.source.endsWith(".desktop")
          ? "Extra launch arguments. Running apps are matched using the desktop launcher's fixed arguments. Shell substitutions are not evaluated."
          : "Quote arguments containing spaces. These also identify running apps for activation and held Close. Shell substitutions are not evaluated."}
      >
        <input
          value={settings.args ?? ""}
          onChange={(e) => onChange({ ...settings, args: e.target.value })}
        />
      </Field>
      <Check
        label="Bring running application to front"
        checked={settings.bring_to_front ?? true}
        onChange={(v) => onChange({ ...settings, bring_to_front: v })}
      />
      <Field label="When held">
        <select
          value={settings.long_press ?? "quit"}
          onChange={(e) =>
            onChange({ ...settings, long_press: e.target.value })
          }
        >
          <option value="quit">Close application</option>
          <option value="force_kill">Force close</option>
          <option value="none">Do nothing</option>
        </select>
      </Field>
      <p className="hint">
        Hold for half a second. Window activation supports KDE; other desktops
        need an application with D-Bus activation. Windows and macOS
        applications need a Linux replacement.
      </p>
    </>
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
  toggle = false,
  clearable = true,
}: {
  settings: RecordValue;
  set: (key: string, value: any) => void;
  controller: Controller;
  toggle?: boolean;
  clearable?: boolean;
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
      {(toggle
        ? ["First shortcut", "Second shortcut"]
        : controller === "Encoder"
          ? ["Turn left", "Turn right", "Press or tap"]
          : ["Shortcut"]
      ).map((label, index) => {
        const chord = entries[index] ?? {};
        if (typeof chord !== "object" || chord === null || Array.isArray(chord))
          return (
            <p key={index}>Imported shortcut requires advanced settings.</p>
          );
        const legacy = chord.KeyModifiers === undefined || chord.KeyModifiers === 0x2000;
        const flags = legacy
          ? modifiers.reduce((value, [field, , bit]) => value | (chord[field] === true ? bit : 0), 0)
          : chord.KeyModifiers;
        const sided = !!(flags & 0x10000);
        const identityFlags = flags & ~0x10fff;
        const enabled = (bit: number) =>
          !!(flags & (sided ? (bit << 4) | (bit << 8) : bit));
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
            <SelectField label="Key mapping" value={chord.linuxKeyMode ?? "physical"}
              options={[["physical", "Physical key positions"], ["logical", "Follow desktop layout"]]}
              onChange={(value) => update({linuxKeyMode:value})} />
            {chord.linuxKeyMode === "logical" && <p className="hint">Matches the selected character in the active X11 or KDE Wayland layout. Releases use the original pressed key even if the layout changes. Keypad and nonprinting keys keep their physical identity.</p>}
            <Check label="Choose modifier sides" checked={sided} onChange={(value) => {
              let nextFlags = flags & ~0x10fff;
              const patch: RecordValue = {};
              for (const [field, , bit] of modifiers) {
                patch[field] = enabled(bit);
                if (patch[field]) nextFlags |= bit | (value ? bit << 4 : 0);
              }
              update({ ...patch, KeyModifiers: nextFlags | (value ? 0x10000 : 0) });
            }} />
            <div className="row">
              {modifiers.map(([field, name, bit]) => (
                sided ? <SelectField key={field} label={`${name} side`}
                  value={(flags & (bit << 4) ? 1 : 0) | (flags & (bit << 8) ? 2 : 0)}
                  options={[[0, "Off"], [1, "Left"], [2, name === "Alt" ? "Right / AltGr" : "Right"], [3, "Both"]]}
                  onChange={(value) => {
                    const side = Number(value);
                    update({ [field]: side !== 0, KeyModifiers: (flags & ~(bit | (bit << 4) | (bit << 8))) | (side ? bit : 0) | (side & 1 ? bit << 4 : 0) | (side & 2 ? bit << 8 : 0) });
                  }} /> : <Check
                  key={field}
                  label={name}
                  checked={enabled(bit)}
                  onChange={(value) =>
                    update({
                      [field]: value,
                      KeyModifiers: value
                        ? flags | bit
                        : flags & ~bit,
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
                  KeyModifiers: (key?.[3] ?? 0) | (flags & 0x10ff0),
                };
                if (key?.[4] !== undefined) patch.NativeCode = key[4];
                for (const [field, , bit] of modifiers) {
                  patch[field] =
                    enabled(bit);
                  if (patch[field]) patch.KeyModifiers |= bit;
                }
                update(patch, true);
              }}
            />
            {clearable && <button
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
            </button>}
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
  if (uuid === "com.deckforge.system.clipboard") {
    for (const key of ["operation", "text", "terminal_shortcuts", "linuxKeyMode"]) handled.add(key);
    const operation = settings.operation ?? "copy_text";
    special = <>
      {select("operation", "Clipboard operation", [["copy_text", "Copy saved text"], ["copy", "Copy selection"], ["cut", "Cut selection"], ["paste", "Paste clipboard"]], "copy_text")}
      {operation === "copy_text" ? <>
        <Field label="Clipboard text"><textarea rows={5} value={settings.text ?? ""} onChange={(event) => set("text", event.target.value)} /></Field>
        <p className="hint">Copies this text when pressed. Replaces the desktop clipboard without typing or pasting.</p>
      </> : <>
        {(operation === "copy" || operation === "paste") && check("terminal_shortcuts", "Terminal shortcuts (Ctrl+Shift)", false)}
        {select("linuxKeyMode", "Shortcut key mapping", [["physical", "Physical key positions"], ["logical", "Follow desktop layout (X11 / KDE Wayland)"]], "physical")}
        {operation === "cut" && settings.terminal_shortcuts === true && <button type="button" onClick={() => set("terminal_shortcuts", false)}>Use standard Cut shortcut</button>}
        <p className="hint">Sends {operation === "cut" ? "Ctrl+X" : `Ctrl+${settings.terminal_shortcuts === true ? "Shift+" : ""}${operation === "copy" ? "C" : "V"}`} to the focused application. Uses the desktop clipboard.</p>
      </>}
    </>;
  }
  if (uuid === "com.deckforge.system.command") {
    for (const key of ["mode", "program", "args", "script", "directory", "terminal", "terminal_program", "terminal_args", "output", "timeout_seconds", "environment"]) handled.add(key);
    special = <>
      {select("mode", "Command mode", [["program", "Program and arguments"], ["shell", "Shell script"]], "program")}
      {settings.mode === "shell" ? <>
        <Field label="Shell script"><textarea rows={5} value={settings.script ?? ""} onChange={(event) => set("script", event.target.value)} /></Field>
        <p className="hint">Runs with /bin/sh. Supports pipes, variables, and redirection.</p>
      </> : <>
        {text("program", "Program", "Executable name or absolute Linux path.")}
        {choose("program", "file", "Choose program…")}
        {text("args", "Arguments", "Quote arguments containing spaces. Variables and shell expressions are passed literally.")}
      </>}
      {text("directory", "Working directory", "Leave empty to use your home folder.")}
      {choose("directory", "directory", "Choose working folder…")}
      <CommandEnvironment value={settings.environment} onChange={(rows) => set("environment", rows)} />
      {check("terminal", "Run in terminal window", false)}
      {settings.terminal === true ? <>
        {text("terminal_program", "Terminal program", "Defaults to Konsole. Enter another installed terminal if needed.")}
        {text("terminal_args", "Terminal arguments", "Arguments before the command. Konsole: --separate --hold -e")}
        <p className="hint">The command and its arguments follow these terminal arguments. Konsole keeps the window open after completion.</p>
      </> : <>
        {select("output", "Command output", [["discard", "Discard output"], ["show", "Show result"], ["clipboard", "Copy successful output to clipboard"]], "discard")}
        {settings.output === "show" || settings.output === "clipboard" ? <>
          {num("timeout_seconds", "Timeout (seconds)", 30, 1, 3600, 1)}
          <p className="hint">Captures stdout and stderr. Stop ends the command and its process group. Captured commands stop when Deckforge quits. Results stay in memory.</p>
          {settings.output === "clipboard" && <p className="hint">Copies complete successful UTF-8 stdout, up to 1 MiB. Errors, timeouts and oversized output leave the clipboard unchanged.</p>}
        </> : <p className="hint">Runs without capturing output or a timeout. Use shell redirection to save output. The command continues if Deckforge closes.</p>}
      </>}
    </>;
  }
  if (uuid.endsWith(".system.openapp")) {
    for (const key of [
      "exec",
      "source",
      "app_name",
      "args",
      "bring_to_front",
      "long_press",
      "bundle_id",
      "bundle_path",
      "is_bundle",
    ])
      handled.add(key);
    special = (
      <OpenApplication
        settings={settings}
        onChange={onChange}
        onError={onError}
      />
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
  if (uuid.endsWith(".system.close")) {
    handled.add("linuxCloseMethod");
    special = (
      <>
        {text("path", "Application executable")}
        {choose("path", "file", "Choose application…")}
        {text("linuxMatchArgs", "Match arguments", "Optional exact argument prefix. Quote values containing spaces; no shell expansion.")}
        {check("forceQuit", "Force close", false)}
        {settings.forceQuit !== true && select("linuxCloseMethod", "Close behavior", [["windows", "Close windows only"], ["terminate", "Send SIGTERM"], ["automatic", "Close windows; stop background processes"]], "automatic")}
        <p className="hint">{settings.forceQuit === true ? "Immediately kills matching processes without a save prompt." : settings.linuxCloseMethod === "terminate" ? "Sends SIGTERM to matching processes. Applications may exit without a save prompt." : (settings.linuxCloseMethod ?? "automatic") === "automatic" ? "On X11 or KDE Wayland, asks windows to close. Sends SIGTERM only if no matching windows are found; desktop errors never trigger a signal fallback." : "Asks matching windows to close so applications can prompt to save. Window closing requires a local X11 window manager or KDE Wayland. Choose Send SIGTERM explicitly for background processes."}</p>
      </>
    );
  }
  if (uuid.endsWith(".system.mouse-wheel"))
    special = (
      <p className="hint">
        Turn to scroll vertically. Hold the dial while turning to scroll
        horizontally. Uses your desktop's scroll settings.
      </p>
    );
  if (uuid.endsWith(".system.timer"))
    special = (
      <>
        {num("duration", "Duration (seconds)", 30, 1, 604800, 1)}
        {select(
          "actionIdx",
          "Alarm sound",
          [
            [0, "Tone 1"],
            [1, "Tone 2"],
            [2, "Tone 3"],
            [3, "Tone 4"],
            [4, "Custom sound"],
            [6, "Silent"],
          ],
          0,
        )}
        {settings.actionIdx === 4 && (
          <>
            {text("lastUserFile", "Sound file")}
            {choose("lastUserFile", "file", "Choose sound…")}
          </>
        )}
        {settings.actionIdx !== 6 &&
          num("linuxAlarmVolume", "Alarm volume (%)", 50, 0, 100, 1)}
        <p className="hint">
          Press to start the countdown. Press again to cancel. Built-in sounds
          use Deckforge tones.
        </p>
      </>
    );
  if (uuid.endsWith(".system.text")) {
    handled.add("pastedText");
    handled.add("isTypingMode");
    handled.add("Hotkey");
    handled.add("linuxKeyMode");
    const customPaste = settings.Hotkey !== null && typeof settings.Hotkey === "object" && !Array.isArray(settings.Hotkey);
    const mode = settings.isTypingMode === true ? "typing" : settings.isTypingMode === false || settings.isTypingMode === undefined ? "paste" : "invalid";
    special = (
      <>
        <Field label="Text mode"><select value={mode} aria-invalid={mode === "invalid" || undefined} aria-describedby={mode === "invalid" ? `${id}-text-mode-error` : undefined} onChange={(event) => set("isTypingMode", event.target.value === "typing")}>
          {mode === "invalid" && <option value="invalid" disabled>Unsupported saved mode</option>}
          <option value="paste">Paste from clipboard</option><option value="typing">Type without clipboard</option>
        </select></Field>
        <Field label="Text">
          <textarea
            value={settings.pastedText ?? ""}
            rows={5}
            onChange={(event) => set("pastedText", event.target.value)}
          />
        </Field>
        {check("isSendingEnter", "Send Enter after text", false)}
        {mode === "invalid" && <p id={`${id}-text-mode-error`} className="warning">The saved text mode is not a boolean. Choose a mode to replace it.</p>}
        {mode === "paste" && <>
          <p className="hint">Pastes Unicode text using the clipboard and the selected shortcut. This replaces the clipboard contents.</p>
          <details>
            <summary>Paste shortcut</summary>
            <Hotkeys settings={{Hotkeys:[customPaste ? settings.Hotkey : {QTKeyCode:86,VKeyCode:86,KeyModifiers:2,linuxKeyMode:settings.linuxKeyMode ?? "physical"}]}} controller="Keypad" clearable={false} set={(_key,value) => set("Hotkey",value[0])} />
            {settings.Hotkey != null && !customPaste && <p className="warning">The saved custom shortcut is not an object. Standard Ctrl+V is used until you replace it.</p>}
            {settings.Hotkey !== undefined && <button type="button" onClick={() => {const next={...settings};delete next.Hotkey;onChange(next);}}>Reset paste shortcut</button>}
            <p className="hint">For terminals, choose Ctrl+Shift+V. Reset or an empty shortcut uses Ctrl+V.</p>
          </details>
        </>}
        {settings.isTypingMode === true && (
          <>
            <p className="hint">Types into the focused application without changing the clipboard. Enable keyboard access for this app session, then focus your destination before pressing the key.</p>
          </>
        )}
      </>
    );
  }
  if (
    uuid.endsWith(".system.hotkey") ||
    uuid.endsWith(".system.hotkeyswitch")
  ) {
    handled.add("Hotkeys");
    handled.add("Coalesce");
    special = (
      <>
        <Hotkeys
          settings={settings}
          set={set}
          controller={controller}
          toggle={uuid.endsWith(".system.hotkeyswitch")}
        />
        <p className="hint">
          Key mapping selects physical positions or the desktop layout. An empty shortcut sends no keys.
        </p>
        {controller === "Encoder" && uuid.endsWith(".system.hotkey") && (
          <>
            {check("Coalesce", "Limit queued dial shortcuts", true)}
            <p className="hint">
              Keep up to three shortcuts queued, or five when disabled.
              Reversing direction replaces pending turns. Taps use the same
              queue.
            </p>
          </>
        )}
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
      "Press to return one folder level. Hold for more than half a second to return to the root page. Configure the folder default under Folder auto-exit.",
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
