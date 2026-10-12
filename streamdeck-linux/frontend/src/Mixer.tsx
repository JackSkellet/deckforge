import { newId } from "./ids";
import {
  meterReading,
  reconcileMixerDraft,
  sameMixerRouting,
  sameMixerConfig,
  savedControl,
  patchMixerControl,
  createMixerControlQueue,
  type MixerControlTarget,
  type MixerControlPatch,
  type MixerLevel,
} from "./MixerLive";
import { request } from "./bridge";
import { useEffect, useRef, useState } from "react";
import { Check, Field, NumberField, SelectField } from "./Fields";
import { clone } from "./model";
import { getFormDraft, setFormDraft } from "./transient";
import type { Run } from "./Panels";

export interface AppSelector {
  field: "application_id" | "process_binary" | "desktop_application";
  value: string;
}
export interface MixerChannel {
  id: string;
  name: string;
  apps: AppSelector[];
  sources: string[];
  linked?: boolean;
  capture_unassigned?: boolean;
}
export interface MixerMix {
  id: string;
  name: string;
  outputs: string[];
  output_controls?: Record<string, { volume: number; muted: boolean }>;
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
  monitoring?: { output: string | null; mix: string | null };
  fade_ms?: number;
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
  muted?: boolean;
}
export interface AudioApp {
  index: number;
  name: string;
  application_id?: string | null;
  process_binary?: string | null;
  sink: string;
  desktop_application?: string | null;
}
export interface MixerSnapshot {
  recording_sources?: Record<string, string>;
  applications?: { id: string; name: string; detected: boolean }[];
  revision?: number;
  controls_unsaved?: boolean;
  save_error?: string | null;
  levels?: {
    channels: Record<string, { peak: number; available: boolean }>;
    mixes: Record<string, { peak: number; available: boolean }>;
  };
  fading?: boolean;
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
    ...(config.monitoring?.mix === id
      ? { monitoring: { ...config.monitoring, mix: null } }
      : {}),
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
      const matches = apps.filter((other) => other[field] === value);
      const identities = new Set(
        matches.map(
          (other) =>
            other.desktop_application || other.application_id || other.name,
        ),
      );
      if (value && !(field === "process_binary" && identities.size > 1)) {
        const selector = { field, value };
        choices.set(JSON.stringify(selector), {
          selector,
          label: `${app.name} · ${field === "application_id" ? "app ID" : "process"}: ${value}`,
        });
      }
    }
  return [...choices.values()];
}
/** Only merge aliases when the observed streams identify one installed application. */
export function sameApplication(
  a: AppSelector,
  b: AppSelector,
  apps: AudioApp[],
) {
  if (same(a, b)) return true;
  const desktop = (selector: AppSelector) => {
    if (selector.field === "desktop_application") return selector.value;
    const matched = apps.filter(
      (app) => app[selector.field] === selector.value,
    );
    if (!matched.length || matched.some((app) => !app.desktop_application))
      return undefined;
    const ids = new Set(matched.map((app) => app.desktop_application));
    return ids.size === 1 ? [...ids][0] : undefined;
  };
  const id = desktop(a);
  return !!id && id === desktop(b);
}
export function PeakMeter({
  label,
  level,
}: {
  label: string;
  level?: MixerLevel;
}) {
  const reading = meterReading(level);
  if (!reading.available)
    return (
      <div
        className="mixer-meter unavailable"
        role="img"
        aria-label={`${label}: meter unavailable`}
      >
        <span className="mixer-meter-track" />
        <span>Unavailable</span>
      </div>
    );
  return (
    <div
      className={`mixer-meter${reading.clipping ? " clipping" : ""}`}
      role="meter"
      aria-label={`${label} peak level`}
      aria-valuemin={-60}
      aria-valuemax={0}
      aria-valuenow={Math.max(-60, Math.min(0, reading.db ?? -60))}
      aria-valuetext={`${reading.text}${reading.clipping ? ", full scale" : ""}`}
    >
      <span className="mixer-meter-track">
        <i style={{ width: `${reading.percent}%` }} />
      </span>
      <span>
        {reading.text}
        {reading.clipping && (
          <strong title="Peak reached full scale"> · Clip</strong>
        )}
      </span>
    </div>
  );
}
function Gain({
  label,
  value,
  onChange,
  muted,
  onMute,
  mode,
  compact = false,
}: {
  label: string;
  value: number;
  onChange: (v: number) => void;
  muted: boolean;
  onMute: (v: boolean) => void;
  mode?: string;
  compact?: boolean;
}) {
  return (
    <div className={"mixer-gain" + (compact ? " compact" : "")} title={mode}>
      <Field
        label={`${label} · ${value}%${mode === "Apply routes to save" ? " · Draft" : ""}`}
      >
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
        {compact ? (
          <svg width="16" height="16" viewBox="0 0 20 20" aria-hidden="true">
            <path
              d="M3 7h3l4-3v12l-4-3H3z"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.5"
            />
            {muted ? (
              <path
                d="m13 7 5 6m0-6-5 6"
                stroke="currentColor"
                strokeWidth="1.5"
              />
            ) : (
              <path
                d="M13 6q5 4 0 8"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.5"
              />
            )}
          </svg>
        ) : muted ? (
          "Muted"
        ) : (
          "Mute"
        )}
      </button>
      {compact && <output aria-hidden="true">{value}%</output>}
    </div>
  );
}
function RecordingInput({
  name,
  sources,
}: {
  name?: string;
  sources: AudioDevice[];
}) {
  const source = name
    ? sources.find((source) => source.name === name)
    : undefined;
  return (
    <div className="mixer-recording-input">
      <span>Recording input</span>
      {source ? (
        <>
          <strong>{source.description || source.name}</strong>
          <code>{source.name}</code>
        </>
      ) : (
        <p className="hint">Enable routing to create this input.</p>
      )}
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
  assignmentLabel = "assigned elsewhere",
}: {
  label: string;
  selected: string[];
  devices: AudioDevice[];
  onChange: (names: string[]) => void;
  unavailableHint: string;
  assigned?: string[];
  assignmentLabel?: string;
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
          label={`${device.description || device.name}${assigned.includes(device.name) && !selected.includes(device.name) ? ` · ${assignmentLabel}` : ""}`}
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
  const [applicationSearch, setApplicationSearch] = useState("");
  const [installedChoice, setInstalledChoice] = useState("");
  const [manualField, setManualField] =
    useState<AppSelector["field"]>("application_id");
  const [manualValue, setManualValue] = useState("");
  const choices = appChoices(inventory.apps);
  const used = config.channels
    .filter((c) => c.id !== channel.id)
    .flatMap((c) => c.apps);
  const available = choices.filter(
    ({ selector }) =>
      ![...channel.apps, ...used].some((a) =>
        sameApplication(a, selector, inventory.apps),
      ),
  );
  const add = (selector: AppSelector) => {
    if (
      !channel.apps.some((a) => sameApplication(a, selector, inventory.apps)) &&
      !used.some((a) => sameApplication(a, selector, inventory.apps))
    )
      onChange({ ...channel, apps: [...channel.apps, selector] });
  };
  const installed = (inventory.applications ?? []).filter(
    (app) =>
      ![...channel.apps, ...used].some((selector) =>
        sameApplication(
          selector,
          { field: "desktop_application", value: app.id },
          inventory.apps,
        ),
      ),
  );
  const filtered = installed.filter((app) =>
    `${app.name} ${app.id}`
      .toLocaleLowerCase()
      .includes(applicationSearch.toLocaleLowerCase().trim()),
  );
  const selectedInstalled = installed.find((app) => app.id === installedChoice);
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
        <Check
          label={`Default for other apps · ${channel.name}`}
          checked={channel.capture_unassigned ?? false}
          onChange={(capture_unassigned) =>
            onChange({ ...channel, capture_unassigned })
          }
        />
        <p className="hint">
          The default group catches other playing apps. Explicit app assignments
          to any group take priority.
        </p>
        <div className="selector-list">
          {channel.apps.map((selector) => {
            const app =
              selector.field === "desktop_application"
                ? inventory.applications?.find(
                    (app) => app.id === selector.value,
                  )
                : undefined;
            const detected =
              app?.detected ??
              inventory.apps.some(
                (audio) => audio[selector.field] === selector.value,
              );
            return (
              <div key={JSON.stringify(selector)}>
                <span>
                  {app && <strong>{app.name}</strong>}
                  <code>{selector.value}</code>
                  <small>
                    {selector.field === "desktop_application"
                      ? "Installed application"
                      : selector.field === "application_id"
                        ? "Application ID"
                        : "Process name"}{" "}
                    · {detected ? "Detected audio" : "Pending audio"}
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
            );
          })}
        </div>
        {inventory.applications !== undefined && (
          <div className="installed-app-chooser">
            <Field label={`Search installed applications for ${channel.name}`}>
              <input
                type="search"
                value={applicationSearch}
                onChange={(e) => {
                  setApplicationSearch(e.target.value);
                  setInstalledChoice("");
                }}
                placeholder="Search apps by name"
              />
            </Field>
            <Field label={`Installed application for ${channel.name}`}>
              <select
                size={4}
                value={selectedInstalled?.id ?? ""}
                onChange={(e) => setInstalledChoice(e.target.value)}
              >
                <option value="" disabled>
                  {filtered.length
                    ? "Select an application…"
                    : "No matching applications"}
                </option>
                {filtered.slice(0, 50).map((app) => (
                  <option key={app.id} value={app.id}>
                    {app.name} · {app.detected ? "Detected" : "Not playing"}
                    {installed.filter((other) => other.name === app.name)
                      .length > 1
                      ? ` · ${app.id}`
                      : ""}
                  </option>
                ))}
              </select>
            </Field>
            {filtered.length > 50 && (
              <p className="hint">
                Showing 50 of {filtered.length} applications. Narrow your
                search.
              </p>
            )}
            {selectedInstalled && (
              <p className="hint">
                Exact identity: <code>{selectedInstalled.id}</code> ·{" "}
                {selectedInstalled.detected
                  ? "Detected audio"
                  : "Pending audio"}
              </p>
            )}
            <button
              type="button"
              disabled={!selectedInstalled || channel.apps.length >= 32}
              onClick={() => {
                if (selectedInstalled) {
                  add({
                    field: "desktop_application",
                    value: selectedInstalled.id,
                  });
                  setInstalledChoice("");
                  setApplicationSearch("");
                }
              }}
            >
              Add application
            </button>
            <p className="hint">
              Choose an app now; it will match when it plays audio. If it cannot
              be identified, use a playing application below.
            </p>
          </div>
        )}
        <details
          className="playing-app-chooser"
          open={inventory.applications === undefined || undefined}
        >
          <summary>Playing applications</summary>
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
            {inventory.applications === undefined
              ? "Add application"
              : "Add playing application"}
          </button>
        </details>
        <details className="manual-selector">
          <summary>Advanced: manual identity</summary>
          <SelectField
            label="Match using"
            value={manualField}
            options={[
              ["application_id", "Application ID"],
              ["process_binary", "Process name"],
              ["desktop_application", "Installed application ID"],
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
                  : manualField === "desktop_application"
                    ? "firefox.desktop"
                    : "firefox"
              }
            />
          </Field>
          <button
            type="button"
            disabled={
              !manualValue.trim() ||
              used.some((a) =>
                sameApplication(
                  a,
                  { field: manualField, value: manualValue.trim() },
                  inventory.apps,
                ),
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
  snapshot: incomingSnapshot,
  run,
  error,
}: {
  snapshot: MixerSnapshot | undefined;
  run: Run;
  error?: string;
}) {
  const [snapshot, setSnapshot] = useState(incomingSnapshot);
  const latest = useRef(incomingSnapshot);
  const [, redraw] = useState(0);
  const [controlError, setControlError] = useState("");
  const queue = useRef<ReturnType<typeof createMixerControlQueue> | null>(null);
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
  const [navigation, setNavigation] = useState<{
    view: "mixer" | "setup";
    mix: string;
  }>(
    () => getFormDraft("audio-mixer-navigation") ?? { view: "mixer", mix: "" },
  );
  useEffect(
    () => setFormDraft("audio-mixer-navigation", navigation),
    [navigation],
  );
  const [createOpen, setCreateOpen] = useState(false);
  const [createSearch, setCreateSearch] = useState("");
  const [createPlacement, setCreatePlacement] = useState({
    up: false,
    height: 380,
  });
  const createButton = useRef<HTMLButtonElement>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const form = useRef<HTMLFormElement>(null);
  if (!queue.current)
    queue.current = createMixerControlQueue(
      async (target, patch) => {
        const current = latest.current;
        if (current?.revision === undefined)
          throw new Error("Mixer controls are not available in this host.");
        const updated = await request<MixerSnapshot>("mixer_control", {
          revision: current.revision,
          target,
          ...patch,
        });
        latest.current = updated;
        setSnapshot(updated);
        setDraft((previous) => reconcileMixerDraft(previous, updated.config));
      },
      (error) => {
        setControlError(String(error));
        void run({ command: "mixer_refresh" });
      },
      () => redraw((value) => value + 1),
    );
  useEffect(() => {
    void run({ command: "mixer_refresh" });
  }, [run]);
  useEffect(() => {
    if (!incomingSnapshot || queue.current?.pending) return;
    latest.current = incomingSnapshot;
    setSnapshot(incomingSnapshot);
    setDraft((previous) =>
      previous && sameMixerConfig(previous.base, incomingSnapshot.config)
        ? previous
        : reconcileMixerDraft(previous, incomingSnapshot.config),
    );
  }, [incomingSnapshot]);
  // Closing the panel must not drop already-authored pending controls; the RPC lane drains.
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
  const config = (queue.current?.intents ?? []).reduce(
    (config, intent) =>
      patchMixerControl(config, intent.target, intent.patch, snapshot.config),
    draft.config,
  );
  const selectedMix = config.mixes.find((m) => m.id === navigation.mix);
  const showSetup = () => {
    setCreateOpen(false);
    setNavigation((current) => ({ ...current, view: "setup" }));
  };
  const liveConfig = (queue.current?.intents ?? []).reduce(
    (current, intent) =>
      patchMixerControl(current, intent.target, intent.patch),
    snapshot.config,
  );
  const monitoring = {
    output: liveConfig.monitoring?.output ?? null,
    mix: liveConfig.monitoring?.mix ?? null,
  };
  const listenDevices = snapshot.sinks.filter(
    (device) =>
      !device.owned && !device.monitor && !device.name.startsWith("deckforge."),
  );
  const missingListenOutput =
    !!monitoring.output &&
    !listenDevices.some((device) => device.name === monitoring.output);
  const listeningMix = snapshot.config.mixes.find(
    (m) => m.id === monitoring.mix,
  );
  const appliedMonitoring = {
    output: snapshot.applied_config?.monitoring?.output ?? null,
    mix: snapshot.applied_config?.monitoring?.mix ?? null,
  };
  const listenStatus = missingListenOutput
    ? "Unavailable output · saved selection kept"
    : snapshot.active && !same(monitoring, appliedMonitoring)
      ? "Switching listening output…"
      : !monitoring.output
        ? "No listening output"
        : !monitoring.mix
          ? "No mix selected"
          : snapshot.active
            ? `Listening to ${listeningMix?.name ?? monitoring.mix}`
            : `Saved: ${listeningMix?.name ?? monitoring.mix} · routing off`;
  const outputOwners = (output: string) => [
    ...new Set(
      [...snapshot.config.mixes, ...config.mixes]
        .filter((m) => m.outputs.includes(output))
        .map((m) => m.name),
    ),
  ];
  const pendingControls = queue.current?.pending ?? 0;
  const stale = !sameMixerRouting(draft.base, snapshot.config);
  function control(target: MixerControlTarget, patch: MixerControlPatch) {
    setControlError("");
    if (
      latest.current?.revision !== undefined &&
      savedControl(latest.current.config, target)
    ) {
      queue.current!.push(target, patch);
    } else {
      setDraft((previous) =>
        previous
          ? {
              ...previous,
              config: patchMixerControl(previous.config, target, patch),
              dirty: true,
            }
          : previous,
      );
    }
  }
  const change = (next: MixerConfig) => {
    const configWithoutPendingControls = reconcileMixerDraft(
      { config: next, base: config, dirty: true },
      draft.config,
    ).config;
    setDraft({
      ...draft,
      config: configWithoutPendingControls,
      dirty: !sameMixerConfig(configWithoutPendingControls, draft.base),
    });
    setMessage("");
  };
  const channel = (next: MixerChannel) =>
    change({
      ...config,
      channels: config.channels.map((c) =>
        c.id === next.id
          ? next
          : next.capture_unassigned
            ? { ...c, capture_unassigned: false }
            : c,
      ),
    });
  const mix = (next: MixerMix) =>
    change({
      ...config,
      mixes: config.mixes.map((m) => (m.id === next.id ? next : m)),
    });
  const controlMode = (target: MixerControlTarget) =>
    snapshot.revision !== undefined && savedControl(snapshot.config, target)
      ? snapshot.active
        ? "Live control"
        : "Saves immediately; routing stays inactive"
      : "Apply routes to save";
  function createChannel(
    app?: { id: string; name: string },
    capture_unassigned = false,
    source?: AudioDevice,
  ) {
    const channels = config.channels.map((channel) =>
      capture_unassigned ? { ...channel, capture_unassigned: false } : channel,
    );
    change({
      ...config,
      channels: [
        ...channels,
        {
          id: newId(),
          name:
            app?.name ??
            source?.description ??
            (capture_unassigned ? "All audio" : `Input ${channels.length + 1}`),
          apps: app ? [{ field: "desktop_application", value: app.id }] : [],
          sources: source ? [source.name] : [],
          capture_unassigned,
        },
      ],
    });
    setCreateOpen(false);
    setCreateSearch("");
    createButton.current?.focus();
  }
  const createChoices = (snapshot.applications ?? [])
    .filter(
      (app) =>
        !config.channels.some((channel) =>
          channel.apps.some((selector) =>
            sameApplication(
              selector,
              { field: "desktop_application", value: app.id },
              snapshot.apps,
            ),
          ),
        ),
    )
    .filter((app) =>
      `${app.name} ${app.id}`
        .toLocaleLowerCase()
        .includes(createSearch.toLocaleLowerCase().trim()),
    );
  const createSources = snapshot.sources
    .filter(
      (source) =>
        !source.owned &&
        !source.monitor &&
        !source.name.startsWith("deckforge.") &&
        !config.channels.some((channel) =>
          channel.sources.includes(source.name),
        ),
    )
    .filter((source) =>
      `${source.description} ${source.name}`
        .toLocaleLowerCase()
        .includes(createSearch.toLocaleLowerCase().trim()),
    );
  async function apply() {
    if (
      !draft ||
      !draft.dirty ||
      stale ||
      !form.current?.reportValidity() ||
      busy ||
      pendingControls > 0
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
      <div className="mixer-navigation" aria-label="Mixer views">
        <div className="segmented" role="group" aria-label="Mixer view">
          <button
            type="button"
            aria-pressed={navigation.view === "mixer"}
            onClick={() =>
              setNavigation((current) => ({ ...current, view: "mixer" }))
            }
          >
            Mixer
          </button>
          <button
            type="button"
            aria-pressed={navigation.view === "setup"}
            onClick={showSetup}
          >
            Routing setup
          </button>
        </div>
        <span className={"status " + (snapshot.active ? "good" : "")}>
          {snapshot.pending
            ? "Applying routing…"
            : snapshot.active
              ? "Routing active"
              : "Routing inactive"}
        </span>
      </div>
      {(snapshot.error || error) && (
        <p className="error" role="alert">
          {snapshot.error || error}
        </p>
      )}
      {controlError && (
        <p className="error" role="alert">
          {controlError}
        </p>
      )}
      {snapshot.save_error && (
        <p className="error" role="alert">
          Audio controls changed, but saving failed: {snapshot.save_error}
        </p>
      )}
      {stale && (
        <p className="warning" role="status">
          Routing settings changed outside this editor. Your draft is kept.
          Reload the current settings before applying it.
        </p>
      )}
      <div className="mixer-live-progress" role="status">
        {pendingControls
          ? "Sending controls…"
          : snapshot.fading
            ? "Fading…"
            : snapshot.controls_unsaved
              ? "Saving controls…"
              : ""}
      </div>
      <fieldset className="mixer-editable" disabled={busy}>
        {navigation.view === "mixer" ? (
          <section className="mixer-daily" aria-label="Mixer controls">
            {!snapshot.active && (
              <div className="mixer-off-banner">
                <div>
                  <strong>Audio routing is off</strong>
                  <p>
                    Levels are saved for later. Set up your apps and outputs,
                    then enable routing to hear the mixes.
                  </p>
                </div>
                <button type="button" onClick={showSetup}>
                  Configure routing
                </button>
              </div>
            )}
            <div className="mixer-listening-toolbar">
              <Field label="Listen on">
                <select
                  disabled={snapshot.revision === undefined}
                  value={monitoring.output ?? ""}
                  onChange={(e) =>
                    control(
                      { kind: "monitor" },
                      { output: e.target.value || null },
                    )
                  }
                >
                  <option value="">No output</option>
                  {missingListenOutput && (
                    <option value={monitoring.output!}>
                      Unavailable · {monitoring.output}
                    </option>
                  )}
                  {listenDevices.map((device) => {
                    const owners = outputOwners(device.name);
                    return (
                      <option
                        key={device.name}
                        value={device.name}
                        disabled={owners.length > 0}
                      >
                        {device.description || device.name}
                        {owners.length
                          ? ` · fixed output of ${owners.join(", ")}`
                          : ""}
                      </option>
                    );
                  })}
                </select>
              </Field>
              <span className="hint" role="status">
                {listenStatus}
                {monitoring.output &&
                listenDevices.find(
                  (device) => device.name === monitoring.output,
                )?.muted
                  ? " · Listening device is muted"
                  : ""}
              </span>
              {monitoring.mix && (
                <button
                  type="button"
                  className="quiet"
                  onClick={() => control({ kind: "monitor" }, { mix: null })}
                >
                  Stop listening
                </button>
              )}
            </div>
            <div className="mixer-console-scroll">
              <table
                className="mixer-console"
                style={{ minWidth: 220 + config.mixes.length * 180 }}
              >
                <caption className="sr-only">
                  Mix volumes. Input groups are rows, mixes are columns.
                </caption>
                <thead>
                  <tr>
                    <th scope="col">Inputs</th>
                    {config.mixes.map((m) => (
                      <th key={m.id} scope="col">
                        <div className="mixer-console-heading">
                          <strong>{m.name}</strong>
                          <button
                            type="button"
                            className="mixer-listen-toggle"
                            aria-label={`Listen to ${m.name}`}
                            aria-pressed={monitoring.mix === m.id}
                            disabled={
                              snapshot.revision === undefined ||
                              !snapshot.config.mixes.some(
                                (saved) => saved.id === m.id,
                              )
                            }
                            title={
                              !snapshot.config.mixes.some(
                                (saved) => saved.id === m.id,
                              )
                                ? "Apply this mix before listening"
                                : monitoring.mix === m.id
                                  ? "Stop listening to this mix"
                                  : "Listen to this mix"
                            }
                            onClick={() =>
                              control(
                                { kind: "monitor" },
                                { mix: monitoring.mix === m.id ? null : m.id },
                              )
                            }
                          >
                            <svg
                              width="17"
                              height="17"
                              viewBox="0 0 20 20"
                              aria-hidden="true"
                            >
                              <path
                                d="M3 12V9a7 7 0 0 1 14 0v3M3 10h3v7H3zM14 10h3v7h-3z"
                                fill="none"
                                stroke="currentColor"
                                strokeWidth="1.5"
                              />
                            </svg>
                          </button>
                          <button
                            type="button"
                            aria-label={`${m.name} mix settings`}
                            aria-expanded={navigation.mix === m.id}
                            onClick={() =>
                              setNavigation((current) => ({
                                ...current,
                                mix: current.mix === m.id ? "" : m.id,
                              }))
                            }
                          >
                            ⚙
                          </button>
                        </div>
                        <PeakMeter
                          label={m.name}
                          level={snapshot.levels?.mixes[m.id]}
                        />
                      </th>
                    ))}
                  </tr>
                </thead>
                <tbody>
                  {config.channels.map((c) => (
                    <tr key={c.id}>
                      <th scope="row">
                        <div className="mixer-console-heading">
                          <strong>
                            {c.name}
                            {c.capture_unassigned && (
                              <small className="mixer-default-badge">
                                Default
                              </small>
                            )}
                          </strong>
                          <button
                            type="button"
                            aria-label={`Configure ${c.name}`}
                            onClick={showSetup}
                          >
                            …
                          </button>
                        </div>
                        <PeakMeter
                          label={c.name}
                          level={snapshot.levels?.channels[c.id]}
                        />
                        <label
                          className="check compact-link"
                          title="Move mix levels together; mutes remain independent"
                        >
                          <input
                            type="checkbox"
                            aria-label={`Link ${c.name} mix levels`}
                            checked={c.linked ?? false}
                            onChange={(e) =>
                              control(
                                { kind: "channel", channel: c.id },
                                { linked: e.target.checked },
                              )
                            }
                          />
                          <span>Link mix levels</span>
                        </label>
                      </th>
                      {config.mixes.map((m) => {
                        const route = config.routes.find(
                          (r) => r.channel === c.id && r.mix === m.id,
                        );
                        return (
                          <td key={m.id}>
                            {route ? (
                              <Gain
                                compact
                                label={`${c.name} → ${m.name}`}
                                mode={controlMode({
                                  kind: "route",
                                  channel: c.id,
                                  mix: m.id,
                                })}
                                value={route.volume}
                                muted={route.muted}
                                onChange={(volume) =>
                                  control(
                                    { kind: "route", channel: c.id, mix: m.id },
                                    { volume },
                                  )
                                }
                                onMute={(muted) =>
                                  control(
                                    { kind: "route", channel: c.id, mix: m.id },
                                    { muted },
                                  )
                                }
                              />
                            ) : (
                              <button
                                className="mixer-add-route"
                                type="button"
                                aria-label={`Add route ${c.name} to ${m.name}`}
                                onClick={() =>
                                  change(editRoute(config, c.id, m.id, {}))
                                }
                              >
                                ＋ Add route
                              </button>
                            )}
                            {route &&
                              controlMode({
                                kind: "route",
                                channel: c.id,
                                mix: m.id,
                              }) === "Apply routes to save" && (
                                <span className="mixer-draft-label">Draft</span>
                              )}
                          </td>
                        );
                      })}
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            <div
              className="mixer-create-channel"
              onKeyDown={(e) => {
                if (e.key === "Escape" && createOpen) {
                  e.stopPropagation();
                  setCreateOpen(false);
                  createButton.current?.focus();
                }
              }}
            >
              <button
                ref={createButton}
                type="button"
                disabled={config.channels.length >= 12}
                aria-expanded={createOpen}
                aria-controls="mixer-create-menu"
                onClick={() => {
                  const rect = createButton.current!.getBoundingClientRect(),
                    dialog = createButton
                      .current!.closest("dialog")
                      ?.getBoundingClientRect();
                  const above = rect.top - (dialog?.top ?? 0) - 90,
                    below =
                      (dialog?.bottom ?? window.innerHeight) - rect.bottom - 70;
                  setCreatePlacement({
                    up: below < 320 && above > below,
                    height: Math.max(
                      180,
                      Math.min(380, Math.max(above, below)),
                    ),
                  });
                  setCreateOpen((value) => !value);
                }}
              >
                ＋ Create channel
              </button>
              {createOpen && (
                <div
                  id="mixer-create-menu"
                  className={
                    "mixer-create-menu" +
                    (createPlacement.up ? " opens-up" : "")
                  }
                  style={{ maxHeight: createPlacement.height }}
                  aria-label="Create channel"
                >
                  <Field label="Search apps and inputs">
                    <input
                      autoFocus
                      type="search"
                      value={createSearch}
                      onChange={(e) => setCreateSearch(e.target.value)}
                      placeholder="Find an app or input device"
                    />
                  </Field>
                  <div className="mixer-create-results">
                    {createSources.length > 0 && (
                      <>
                        <h4>Microphones &amp; audio inputs</h4>
                        {createSources.slice(0, 20).map((source) => (
                          <button
                            type="button"
                            key={source.name}
                            title={source.name}
                            onClick={() =>
                              createChannel(undefined, false, source)
                            }
                          >
                            {source.description || source.name}
                          </button>
                        ))}
                      </>
                    )}
                    {[true, false].map((detected) => {
                      const apps = createChoices
                        .slice(0, 40)
                        .filter((app) => app.detected === detected);
                      return apps.length ? (
                        <div key={String(detected)}>
                          <h4>{detected ? "Apps" : "Apps (not playing)"}</h4>
                          {apps.map((app) => (
                            <button
                              type="button"
                              key={app.id}
                              title={app.id}
                              onClick={() => createChannel(app)}
                            >
                              {app.name}
                              <small>
                                {app.detected ? "Detected" : "Not playing"}
                              </small>
                            </button>
                          ))}
                        </div>
                      ) : null;
                    })}
                    {createChoices.length === 0 &&
                      createSources.length === 0 && (
                        <p className="hint">
                          No matching apps or input devices.
                        </p>
                      )}
                    {createChoices.length > 40 && (
                      <p className="hint">Narrow your search for more apps.</p>
                    )}
                  </div>
                  <button
                    type="button"
                    onClick={() => createChannel(undefined, true)}
                  >
                    All audio
                    <small>
                      System sounds and apps without an explicit assignment
                    </small>
                  </button>
                  <button type="button" onClick={() => createChannel()}>
                    Empty channel
                  </button>
                </div>
              )}
            </div>
            {config.channels.length === 0 && (
              <p className="mixer-setup-empty">
                Create a channel from an app, or choose All audio to catch the
                rest. Add its routes, then Apply.
              </p>
            )}
            {navigation.mix && selectedMix && (
              <aside
                className="mixer-output-panel"
                aria-label={`${selectedMix.name} master controls`}
              >
                <div className="mixer-console-heading">
                  <h3>{selectedMix.name} levels</h3>
                  <button
                    type="button"
                    aria-label="Close mix settings"
                    onClick={() =>
                      setNavigation((current) => ({ ...current, mix: "" }))
                    }
                  >
                    ×
                  </button>
                </div>
                <div className="mixer-output-panel-controls">
                  <Gain
                    label={`${selectedMix.name} master`}
                    mode={controlMode({ kind: "mix", mix: selectedMix.id })}
                    value={selectedMix.volume}
                    muted={selectedMix.muted}
                    onChange={(volume) =>
                      control({ kind: "mix", mix: selectedMix.id }, { volume })
                    }
                    onMute={(muted) =>
                      control({ kind: "mix", mix: selectedMix.id }, { muted })
                    }
                  />
                  {selectedMix.outputs.map((output) => {
                    const values = selectedMix.output_controls?.[output] ?? {
                      volume: 100,
                      muted: false,
                    };
                    const name =
                      snapshot.sinks.find((device) => device.name === output)
                        ?.description || output;
                    return (
                      <Gain
                        key={output}
                        label={`${selectedMix.name} output · ${name}`}
                        mode={controlMode({
                          kind: "output",
                          mix: selectedMix.id,
                          output,
                        })}
                        value={values.volume}
                        muted={values.muted}
                        onChange={(volume) =>
                          control(
                            { kind: "output", mix: selectedMix.id, output },
                            { volume },
                          )
                        }
                        onMute={(muted) =>
                          control(
                            { kind: "output", mix: selectedMix.id, output },
                            { muted },
                          )
                        }
                      />
                    );
                  })}
                </div>
                <p className="hint">
                  {selectedMix.outputs.length
                    ? "Device levels affect only this mix’s output to each device."
                    : "No fixed output. Use Listen on above, or assign a fixed output in Routing setup."}
                </p>
              </aside>
            )}
          </section>
        ) : (
          <section className="mixer-setup" aria-label="Routing setup">
            <div className="mixer-toolbar">
              <div>
                <Check
                  label="Enable audio routing"
                  checked={config.enabled}
                  onChange={(enabled) => change({ ...config, enabled })}
                />
                <p className="hint">
                  Setup changes take effect when you choose Apply routes.
                </p>
              </div>
              <button
                type="button"
                onClick={() => run({ command: "mixer_refresh" })}
              >
                Refresh devices
              </button>
            </div>
            <fieldset className="mixer-global-settings">
              <legend>Global controls</legend>
              <NumberField
                label="Default fade (ms)"
                value={config.fade_ms ?? 0}
                min={0}
                max={10000}
                step={1}
                onChange={(fade_ms) => {
                  if (
                    Number.isInteger(fade_ms) &&
                    fade_ms >= 0 &&
                    fade_ms <= 10000
                  )
                    control({ kind: "preferences" }, { fade_ms });
                }}
              />
              <p className="hint">
                Fade time is saved immediately and applies to subsequent level
                changes.
              </p>
            </fieldset>
            <div className="mixer-section-heading">
              <div>
                <h3>1. Input groups</h3>
                <p className="hint">
                  Group your apps and devices, then choose which mixes receive
                  them.
                </p>
              </div>
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
            </div>
            <div className="mixer-setup-groups">
              {config.channels.map((c) => (
                <section
                  className="mixer-setup-card"
                  key={c.id}
                  aria-label={`${c.name} setup`}
                >
                  <div className="mixer-channel-heading">
                    <h3>
                      {c.name || "Unnamed input"}
                      {c.capture_unassigned && (
                        <small className="mixer-default-badge">Default</small>
                      )}
                    </h3>
                    <button
                      type="button"
                      className="quiet"
                      aria-label={`Remove input group ${c.name}`}
                      onClick={() => change(removeChannel(config, c.id))}
                    >
                      Remove
                    </button>
                  </div>
                  <ChannelInputs
                    channel={c}
                    config={config}
                    inventory={snapshot}
                    onChange={channel}
                  />
                  <fieldset className="mixer-route-choices">
                    <legend>Send to mixes</legend>
                    {config.mixes.map((m) => (
                      <Check
                        key={m.id}
                        label={`Route ${c.name} to ${m.name}`}
                        checked={config.routes.some(
                          (r) => r.channel === c.id && r.mix === m.id,
                        )}
                        onChange={(enabled) =>
                          change(
                            editRoute(config, c.id, m.id, enabled ? {} : null),
                          )
                        }
                      />
                    ))}
                  </fieldset>
                </section>
              ))}
            </div>
            {config.channels.length === 0 && (
              <p className="mixer-setup-empty">
                Add an input group to choose apps or a microphone. Make one
                group the default to catch all other apps.
              </p>
            )}
            <div className="mixer-section-heading">
              <div>
                <h3>2. Mixes & output devices</h3>
                <p className="hint">
                  Each mix has its own balance. Send it to headphones, speakers,
                  or a recording app.
                </p>
              </div>
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
            </div>
            <div className="mixer-setup-mixes">
              {config.mixes.map((m) => (
                <section
                  className="mixer-setup-card"
                  key={m.id}
                  aria-label={`${m.name} output setup`}
                >
                  <Field label={`${m.name} mix name`}>
                    <input
                      required
                      maxLength={160}
                      value={m.name}
                      onChange={(e) => mix({ ...m, name: e.target.value })}
                    />
                  </Field>
                  <DeviceChoices
                    label={`${m.name} outputs`}
                    assigned={monitoring.output ? [monitoring.output] : []}
                    assignmentLabel="used by Listen on"
                    selected={m.outputs}
                    devices={snapshot.sinks}
                    unavailableHint="This saved output is kept. No replacement device is chosen automatically."
                    onChange={(outputs) => mix({ ...m, outputs })}
                  />
                  <RecordingInput
                    name={snapshot.recording_sources?.[m.id]}
                    sources={snapshot.sources}
                  />
                  <button
                    className="danger quiet"
                    type="button"
                    onClick={() => change(removeMix(config, m.id))}
                  >
                    Remove mix
                  </button>
                </section>
              ))}
            </div>
          </section>
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
            disabled={!draft.dirty || busy || stale || pendingControls > 0}
          >
            {busy ? "Applying…" : "Apply routes"}
          </button>
          <button
            type="button"
            disabled={busy || pendingControls > 0 || (!draft.dirty && !stale)}
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
