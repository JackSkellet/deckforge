import { newId } from "./ids";
import { useEffect, useRef, useState } from "react";
import { Check, Field, SelectField } from "./Fields";
import { clone } from "./model";
import { getFormDraft, setFormDraft } from "./transient";
import type { Run } from "./Panels";

export interface AppSelector {
  field: "application_id" | "process_binary";
  value: string;
}
export interface MixerChannel {
  id: string;
  name: string;
  apps: AppSelector[];
  sources: string[];
}
export interface MixerMix {
  id: string;
  name: string;
  outputs: string[];
  volume: number;
  muted: boolean;
}
export interface MixerRoute {
  channel: string;
  mix: string;
  volume: number;
  muted: boolean;
}
export interface MixerConfig {
  enabled: boolean;
  channels: MixerChannel[];
  mixes: MixerMix[];
  routes: MixerRoute[];
}
export interface AudioDevice {
  name: string;
  description: string;
  owned: boolean;
  monitor: boolean;
}
export interface AudioApp {
  index: number;
  name: string;
  application_id?: string | null;
  process_binary?: string | null;
  sink: string;
}
export interface MixerSnapshot {
  config: MixerConfig;
  connected: boolean;
  active: boolean;
  sinks: AudioDevice[];
  sources: AudioDevice[];
  apps: AudioApp[];
  warnings: string[];
  error?: string | null;
  pending?: boolean;
  applied_config?: MixerConfig;
}
interface MixerDraft {
  config: MixerConfig;
  base: MixerConfig;
  dirty: boolean;
}
function canonical(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(canonical);
  if (value && typeof value === "object")
    return Object.fromEntries(
      Object.entries(value)
        .sort(([a], [b]) => a.localeCompare(b))
        .map(([key, entry]) => [key, canonical(entry)]),
    );
  return value;
}
export const sameMixerValue = (a: unknown, b: unknown) =>
  JSON.stringify(canonical(a)) === JSON.stringify(canonical(b));
const same = sameMixerValue;
export function editRoute(
  config: MixerConfig,
  channel: string,
  mix: string,
  patch: Partial<MixerRoute> | null,
): MixerConfig {
  const next = clone(config);
  const index = next.routes.findIndex(
    (r) => r.channel === channel && r.mix === mix,
  );
  if (patch === null) {
    next.routes = next.routes.filter(
      (r) => !(r.channel === channel && r.mix === mix),
    );
    return next;
  }
  const route = {
    channel,
    mix,
    volume: 100,
    muted: false,
    ...(index < 0 ? {} : next.routes[index]),
    ...patch,
  };
  if (index < 0) next.routes.push(route);
  else next.routes[index] = route;
  return next;
}
export function removeChannel(config: MixerConfig, id: string): MixerConfig {
  return {
    ...config,
    channels: config.channels.filter((c) => c.id !== id),
    routes: config.routes.filter((r) => r.channel !== id),
  };
}
export function removeMix(config: MixerConfig, id: string): MixerConfig {
  return {
    ...config,
    mixes: config.mixes.filter((m) => m.id !== id),
    routes: config.routes.filter((r) => r.mix !== id),
  };
}
export function appChoices(
  apps: AudioApp[],
): { selector: AppSelector; label: string }[] {
  const choices = new Map<string, { selector: AppSelector; label: string }>();
  for (const app of apps)
    for (const field of ["application_id", "process_binary"] as const) {
      const value = app[field];
      if (value) {
        const selector = { field, value };
        choices.set(JSON.stringify(selector), {
          selector,
          label: `${app.name} · ${field === "application_id" ? "app ID" : "process"}: ${value}`,
        });
      }
    }
  return [...choices.values()];
}
function Gain({
  label,
  value,
  onChange,
  muted,
  onMute,
}: {
  label: string;
  value: number;
  onChange: (v: number) => void;
  muted: boolean;
  onMute: (v: boolean) => void;
}) {
  return (
    <div className="mixer-gain">
      <Field label={`${label} · ${value}%`}>
        <input
          aria-label={`${label} volume`}
          type="range"
          min="0"
          max="100"
          step="1"
          value={value}
          onChange={(e) => onChange(Number(e.target.value))}
        />
      </Field>
      <button
        type="button"
        aria-label={`Mute ${label}`}
        aria-pressed={muted}
        className={muted ? "mute active" : "mute"}
        onClick={() => onMute(!muted)}
      >
        {muted ? "Muted" : "Mute"}
      </button>
    </div>
  );
}
function DeviceChoices({
  label,
  selected,
  devices,
  onChange,
  unavailableHint,
  assigned = [],
}: {
  label: string;
  selected: string[];
  devices: AudioDevice[];
  onChange: (names: string[]) => void;
  unavailableHint: string;
  assigned?: string[];
}) {
  const available = devices.filter(
    (device) => !device.owned && !device.name.startsWith("deckforge."),
  );
  const missing = selected.filter(
    (name) => !available.some((d) => d.name === name),
  );
  return (
    <fieldset className="mixer-device-choices">
      <legend>{label}</legend>
      {available.map((device) => (
        <Check
          disabled={
            !selected.includes(device.name) &&
            (assigned.includes(device.name) || selected.length >= 8)
          }
          key={device.name}
          label={`${device.description || device.name}${assigned.includes(device.name) && !selected.includes(device.name) ? " · assigned elsewhere" : ""}`}
          checked={selected.includes(device.name)}
          onChange={(checked) => {
            if (!checked)
              onChange(selected.filter((name) => name !== device.name));
            else if (!assigned.includes(device.name))
              onChange([...selected, device.name]);
          }}
        />
      ))}
      {!available.length && (
        <p className="hint">
          No available devices. Refresh the inventory after connecting one.
        </p>
      )}
      {missing.map((name) => (
        <div className="missing-audio" key={name}>
          <span>
            <strong>Unavailable</strong>
            <code>{name}</code>
            <small>{unavailableHint}</small>
          </span>
          <button
            type="button"
            aria-label={`Remove unavailable ${name}`}
            onClick={() => onChange(selected.filter((value) => value !== name))}
          >
            Remove
          </button>
        </div>
      ))}
    </fieldset>
  );
}
function ChannelInputs({
  channel,
  config,
  inventory,
  onChange,
}: {
  channel: MixerChannel;
  config: MixerConfig;
  inventory: MixerSnapshot;
  onChange: (channel: MixerChannel) => void;
}) {
  const [chosen, setChosen] = useState("");
  const [manualField, setManualField] =
    useState<AppSelector["field"]>("application_id");
  const [manualValue, setManualValue] = useState("");
  const choices = appChoices(inventory.apps);
  const used = config.channels
    .filter((c) => c.id !== channel.id)
    .flatMap((c) => c.apps);
  const available = choices.filter(
    ({ selector }) =>
      ![...channel.apps, ...used].some((a) => same(a, selector)),
  );
  const add = (selector: AppSelector) => {
    if (
      !channel.apps.some((a) => same(a, selector)) &&
      !used.some((a) => same(a, selector))
    )
      onChange({ ...channel, apps: [...channel.apps, selector] });
  };
  return (
    <details className="mixer-input-details">
      <summary>
        Edit inputs{" "}
        <span>
          {channel.apps.length} apps · {channel.sources.length} devices
        </span>
      </summary>
      <div className="mixer-input-fields">
        <Field label={`${channel.name} name`}>
          <input
            required
            maxLength={160}
            value={channel.name}
            onChange={(e) => onChange({ ...channel, name: e.target.value })}
          />
        </Field>
        <div className="selector-list">
          {channel.apps.map((selector) => (
            <div key={JSON.stringify(selector)}>
              <span>
                <code>{selector.value}</code>
                <small>
                  {selector.field === "application_id"
                    ? "Application ID"
                    : "Process name"}
                  {!choices.some((c) => same(c.selector, selector))
                    ? " · not currently playing"
                    : ""}
                </small>
              </span>
              <button
                type="button"
                aria-label={`Remove application ${selector.value}`}
                onClick={() =>
                  onChange({
                    ...channel,
                    apps: channel.apps.filter((a) => !same(a, selector)),
                  })
                }
              >
                ×
              </button>
            </div>
          ))}
        </div>
        <Field label={`Application for ${channel.name}`}>
          <select value={chosen} onChange={(e) => setChosen(e.target.value)}>
            <option value="">Choose a playing application…</option>
            {available.map(({ selector, label }) => (
              <option
                value={JSON.stringify(selector)}
                key={JSON.stringify(selector)}
              >
                {label}
              </option>
            ))}
          </select>
        </Field>
        <button
          type="button"
          disabled={!chosen || channel.apps.length >= 32}
          onClick={() => {
            if (chosen) {
              add(JSON.parse(chosen));
              setChosen("");
            }
          }}
        >
          Add application
        </button>
        <details className="manual-selector">
          <summary>Enter an application identity</summary>
          <SelectField
            label="Match using"
            value={manualField}
            options={[
              ["application_id", "Application ID"],
              ["process_binary", "Process name"],
            ]}
            onChange={(value) => setManualField(value as AppSelector["field"])}
          />
          <Field label="Exact identity">
            <input
              value={manualValue}
              onChange={(e) => setManualValue(e.target.value)}
              placeholder={
                manualField === "application_id"
                  ? "org.mozilla.firefox"
                  : "firefox"
              }
            />
          </Field>
          <button
            type="button"
            disabled={
              !manualValue.trim() ||
              used.some(
                (a) =>
                  a.field === manualField && a.value === manualValue.trim(),
              ) ||
              channel.apps.length >= 32
            }
            onClick={() => {
              add({ field: manualField, value: manualValue.trim() });
              setManualValue("");
            }}
          >
            Add identity
          </button>
        </details>
        <DeviceChoices
          label="Microphone or input device"
          selected={channel.sources}
          devices={inventory.sources.filter((d) => !d.monitor)}
          assigned={config.channels
            .filter((c) => c.id !== channel.id)
            .flatMap((c) => c.sources)}
          unavailableHint="The saved input stays assigned until you remove it or reconnect the device."
          onChange={(sources) => onChange({ ...channel, sources })}
        />
      </div>
    </details>
  );
}
export function AudioMixer({
  snapshot,
  run,
  error,
}: {
  snapshot: MixerSnapshot | undefined;
  run: Run;
  error?: string;
}) {
  const initial = snapshot?.config;
  const [draft, setDraft] = useState<MixerDraft | null>(() => {
    const retained = getFormDraft("audio-mixer");
    return (
      retained ??
      (initial
        ? { config: clone(initial), base: clone(initial), dirty: false }
        : null)
    );
  });
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const form = useRef<HTMLFormElement>(null);
  const signature = JSON.stringify(canonical(initial));
  useEffect(() => {
    void run({ command: "mixer_refresh" });
  }, [run]);
  useEffect(() => {
    if (initial)
      setDraft((previous) =>
        previous?.dirty
          ? previous
          : { config: clone(initial), base: clone(initial), dirty: false },
      );
  }, [signature]);
  useEffect(() => {
    if (draft) setFormDraft("audio-mixer", draft);
  }, [draft]);
  if (!snapshot || !draft)
    return (
      <div className="empty">
        <h3>Reading audio devices</h3>
        <p>The mixer inventory is not available yet.</p>
        <button onClick={() => run({ command: "mixer_refresh" })}>
          Refresh audio devices
        </button>
        {error && (
          <p role="alert" className="error">
            {error}
          </p>
        )}
      </div>
    );
  const config = draft.config;
  const stale = !same(draft.base, snapshot.config);
  const change = (config: MixerConfig) => {
    setDraft({ ...draft, config, dirty: true });
    setMessage("");
  };
  const channel = (next: MixerChannel) =>
    change({
      ...config,
      channels: config.channels.map((c) => (c.id === next.id ? next : c)),
    });
  const mix = (next: MixerMix) =>
    change({
      ...config,
      mixes: config.mixes.map((m) => (m.id === next.id ? next : m)),
    });
  async function apply() {
    if (
      !draft ||
      !draft.dirty ||
      stale ||
      !form.current?.reportValidity() ||
      busy
    )
      return;
    setBusy(true);
    setMessage("");
    try {
      if (
        await run({
          command: "mixer_config",
          settings: draft.config,
          expected: draft.base,
        })
      ) {
        setDraft({
          config: clone(draft.config),
          base: clone(draft.config),
          dirty: false,
        });
        setMessage(
          "Routing settings saved. Audio status below reflects the backend.",
        );
      } else setMessage("Changes were not applied. Your mixer draft is kept.");
    } finally {
      setBusy(false);
    }
  }
  return (
    <form
      className="audio-mixer"
      ref={form}
      onSubmit={(e) => {
        e.preventDefault();
        void apply();
      }}
    >
      <div className="mixer-intro">
        <div>
          <p>
            Separate what you hear from what you stream. Group applications and
            inputs, then balance each mix independently.
          </p>
          <small>
            Wave Link-style routing, implemented natively in Rust. Changes apply
            only when you choose Apply routes.
          </small>
        </div>
        <span className={"status " + (snapshot.active ? "good" : "")}>
          {snapshot.pending
            ? "Applying routing…"
            : snapshot.active
              ? "Routing active"
              : snapshot.connected
                ? "Audio connected · routing inactive"
                : "Audio disconnected"}
        </span>
      </div>
      {(snapshot.error || error) && (
        <p className="error" role="alert">
          {snapshot.error || error}
        </p>
      )}
      {stale && (
        <p className="warning" role="status">
          Routing settings changed outside this editor. Your draft is kept.
          Reload the current settings before applying it.
        </p>
      )}
      <fieldset className="mixer-editable" disabled={busy}>
        <div className="mixer-toolbar">
          <Check
            label="Enable audio routing"
            checked={config.enabled}
            onChange={(enabled) => change({ ...config, enabled })}
          />
          <div className="row">
            <button
              type="button"
              disabled={config.channels.length >= 12}
              onClick={() =>
                change({
                  ...config,
                  channels: [
                    ...config.channels,
                    {
                      id: newId(),
                      name: `Input ${config.channels.length + 1}`,
                      apps: [],
                      sources: [],
                    },
                  ],
                })
              }
            >
              ＋ Add input group
            </button>
            <button
              type="button"
              disabled={config.mixes.length >= 5}
              onClick={() =>
                change({
                  ...config,
                  mixes: [
                    ...config.mixes,
                    {
                      id: newId(),
                      name: `Mix ${config.mixes.length + 1}`,
                      outputs: [],
                      volume: 100,
                      muted: false,
                    },
                  ],
                })
              }
            >
              ＋ Add mix
            </button>
            <button
              type="button"
              onClick={() => run({ command: "mixer_refresh" })}
            >
              Refresh devices
            </button>
          </div>
        </div>
        <div className="mixer-matrix-scroll">
          <table className="mixer-matrix">
            <caption className="sr-only">
              Audio routing matrix. Each input group has a separate volume and
              mute for each mix.
            </caption>
            <thead>
              <tr>
                <th scope="col">
                  Input groups<span>Applications & devices</span>
                </th>
                {config.mixes.map((m) => (
                  <th scope="col" key={m.id}>
                    <Field label={`${m.name} mix name`}>
                      <input
                        required
                        maxLength={160}
                        value={m.name}
                        onChange={(e) => mix({ ...m, name: e.target.value })}
                      />
                    </Field>
                    <Gain
                      label={`${m.name} master`}
                      value={m.volume}
                      muted={m.muted}
                      onChange={(volume) => mix({ ...m, volume })}
                      onMute={(muted) => mix({ ...m, muted })}
                    />
                    <details className="mixer-output-details">
                      <summary>
                        {m.name} outputs{" "}
                        <span>{m.outputs.length || "Virtual output only"}</span>
                      </summary>
                      <DeviceChoices
                        label={`${m.name} outputs`}
                        selected={m.outputs}
                        devices={snapshot.sinks}
                        unavailableHint="This saved output is kept. No replacement device is chosen automatically."
                        onChange={(outputs) => mix({ ...m, outputs })}
                      />
                      <p className="hint">
                        The mix is also available as a virtual input to
                        recording and streaming apps.
                      </p>
                      <button
                        type="button"
                        className="danger quiet"
                        onClick={() => change(removeMix(config, m.id))}
                      >
                        Remove mix
                      </button>
                    </details>
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {config.channels.map((c) => (
                <tr key={c.id}>
                  <th scope="row">
                    <div className="mixer-channel-heading">
                      <strong>{c.name || "Unnamed input"}</strong>
                      <button
                        type="button"
                        aria-label={`Remove input group ${c.name}`}
                        className="quiet"
                        onClick={() => change(removeChannel(config, c.id))}
                      >
                        ×
                      </button>
                    </div>
                    <ChannelInputs
                      channel={c}
                      config={config}
                      inventory={snapshot}
                      onChange={channel}
                    />
                  </th>
                  {config.mixes.map((m) => {
                    const route = config.routes.find(
                      (r) => r.channel === c.id && r.mix === m.id,
                    );
                    return (
                      <td key={m.id}>
                        <Check
                          label={`Route ${c.name} to ${m.name}`}
                          checked={!!route}
                          onChange={(enabled) =>
                            change(
                              editRoute(
                                config,
                                c.id,
                                m.id,
                                enabled ? {} : null,
                              ),
                            )
                          }
                        />
                        {route ? (
                          <Gain
                            label={`${c.name} → ${m.name}`}
                            value={route.volume}
                            muted={route.muted}
                            onChange={(volume) =>
                              change(editRoute(config, c.id, m.id, { volume }))
                            }
                            onMute={(muted) =>
                              change(editRoute(config, c.id, m.id, { muted }))
                            }
                          />
                        ) : (
                          <p className="unrouted">Not routed</p>
                        )}
                      </td>
                    );
                  })}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        {config.channels.length === 0 && (
          <div className="mixer-empty">
            <h3>Add your first input group</h3>
            <p>
              For example: a game, music player, voice chat, or microphone. Then
              enable the routes you want to hear or stream.
            </p>
          </div>
        )}
        {config.mixes.length === 0 && (
          <p className="warning">
            Add a mix to choose where input groups are routed.
          </p>
        )}
      </fieldset>
      {snapshot.warnings.length > 0 && (
        <details className="mixer-warnings" open>
          <summary>Audio notices ({snapshot.warnings.length})</summary>
          <ul>
            {snapshot.warnings.map((warning, i) => (
              <li key={i}>{warning}</li>
            ))}
          </ul>
        </details>
      )}
      <footer className="mixer-footer">
        <div className="row">
          <button
            type="submit"
            className="primary"
            disabled={!draft.dirty || busy || stale}
          >
            {busy ? "Applying…" : "Apply routes"}
          </button>
          <button
            type="button"
            disabled={busy || (!draft.dirty && !stale)}
            onClick={() => {
              setDraft({
                config: clone(snapshot.config),
                base: clone(snapshot.config),
                dirty: false,
              });
              setMessage("Current routing settings loaded");
            }}
          >
            Reload current settings
          </button>
        </div>
        <span className="hint" role="status">
          {message ||
            (draft.dirty
              ? "Unapplied mixer changes"
              : config.enabled
                ? "Configured routing enabled"
                : "Routing is disabled")}
        </span>
      </footer>
    </form>
  );
}
