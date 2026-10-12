import type { MixerConfig } from "./Mixer";
export interface MixerLevel {
  peak: number;
  available: boolean;
}
export type MixerControlTarget =
  | { kind: "route"; channel: string; mix: string }
  | { kind: "mix"; mix: string }
  | { kind: "output"; mix: string; output: string }
  | { kind: "channel"; channel: string }
  | { kind: "preferences" }
  | { kind: "monitor" };
export type MixerControlPatch = {
  volume?: number;
  muted?: boolean;
  linked?: boolean;
  fade_ms?: number;
  output?: string | null;
  mix?: string | null;
};
export interface MixerDraftState {
  config: MixerConfig;
  base: MixerConfig;
  dirty: boolean;
}
const canonical = (value: any): any =>
  Array.isArray(value)
    ? value.map(canonical)
    : value && typeof value === "object"
      ? Object.fromEntries(
          Object.entries(value)
            .sort(([a], [b]) => a.localeCompare(b))
            .map(([key, value]) => [key, canonical(value)]),
        )
      : value;
const equal = (a: unknown, b: unknown) =>
  JSON.stringify(canonical(a)) === JSON.stringify(canonical(b));
/** Graph membership stays guarded; immediate control values can rebase independently. */
export function mixerRoutingShape(config: MixerConfig) {
  const { fade_ms: _fade, monitoring: _monitoring, ...rest } = config;
  return {
    ...rest,
    connections: config.connections ?? [],
    channels: config.channels.map(({ linked: _linked, ...channel }) => ({
      ...channel,
      capture_unassigned: channel.capture_unassigned ?? false,
    })),
    mixes: config.mixes.map(
      ({ volume: _volume, muted: _muted, output_controls: _outputs, ...mix }) =>
        mix,
    ),
    routes: config.routes.map(
      ({ volume: _volume, muted: _muted, ...route }) => route,
    ),
  };
}
export function sameMixerRouting(a: MixerConfig, b: MixerConfig) {
  return equal(mixerRoutingShape(a), mixerRoutingShape(b));
}
function controlValues(value: Partial<MixerControlPatch> | undefined) {
  return { volume: value?.volume ?? 100, muted: value?.muted ?? false };
}
function semanticConfig(config: MixerConfig) {
  return {
    ...config,
    connections: config.connections ?? [],
    fade_ms: config.fade_ms ?? 0,
    monitoring: {
      output: config.monitoring?.output ?? null,
      mix: config.monitoring?.mix ?? null,
    },
    channels: config.channels.map((channel) => ({
      ...channel,
      linked: channel.linked ?? false,
      capture_unassigned: channel.capture_unassigned ?? false,
    })),
    mixes: config.mixes.map((mix) => ({
      ...mix,
      output_controls: Object.fromEntries(
        mix.outputs.map((output) => [
          output,
          controlValues(mix.output_controls?.[output]),
        ]),
      ),
    })),
  };
}
export function sameMixerConfig(a: MixerConfig, b: MixerConfig) {
  return equal(semanticConfig(a), semanticConfig(b));
}
function mergeControl(
  base: MixerControlPatch,
  edited: MixerControlPatch,
  incoming: MixerControlPatch,
) {
  const result = { ...edited };
  for (const key of ["volume", "muted"] as const)
    if (edited[key] === base[key])
      Object.assign(result, { [key]: incoming[key] });
  return result;
}
/** Rebase live gains by stable identity. Preserve explicit offline gain edits and routing drafts. */
export function reconcileMixerDraft(
  previous: MixerDraftState | undefined | null,
  incoming: MixerConfig,
): MixerDraftState {
  if (!previous || !previous.dirty)
    return {
      config: structuredClone(incoming),
      base: structuredClone(incoming),
      dirty: false,
    };
  if (!sameMixerRouting(previous.base, incoming)) return previous;
  const config = structuredClone(previous.config),
    base = previous.base;
  if ((config.fade_ms ?? 0) === (base.fade_ms ?? 0))
    config.fade_ms = incoming.fade_ms;
  const beforeMonitor = {
    output: base.monitoring?.output ?? null,
    mix: base.monitoring?.mix ?? null,
  };
  const editedMonitor = {
    output: config.monitoring?.output ?? null,
    mix: config.monitoring?.mix ?? null,
  };
  const nextMonitor = {
    output: incoming.monitoring?.output ?? null,
    mix: incoming.monitoring?.mix ?? null,
  };
  if (editedMonitor.output === beforeMonitor.output)
    editedMonitor.output = nextMonitor.output;
  const implicitRemoval =
    !!beforeMonitor.mix &&
    !config.mixes.some((m) => m.id === beforeMonitor.mix) &&
    editedMonitor.mix === null;
  if (editedMonitor.mix === beforeMonitor.mix || implicitRemoval)
    editedMonitor.mix =
      nextMonitor.mix && config.mixes.some((m) => m.id === nextMonitor.mix)
        ? nextMonitor.mix
        : null;
  if (base.monitoring || incoming.monitoring || config.monitoring)
    config.monitoring = editedMonitor;
  for (const next of incoming.channels) {
    const before = base.channels.find((c) => c.id === next.id),
      edited = config.channels.find((c) => c.id === next.id);
    if (
      before &&
      edited &&
      (before.linked ?? false) === (edited.linked ?? false)
    )
      edited.linked = next.linked;
  }
  for (const next of incoming.routes) {
    const before = base.routes.find(
      (route) => route.channel === next.channel && route.mix === next.mix,
    );
    const edited = config.routes.find(
      (route) => route.channel === next.channel && route.mix === next.mix,
    );
    if (before && edited)
      Object.assign(edited, mergeControl(before, edited, next));
  }
  for (const next of incoming.mixes) {
    const before = base.mixes.find((mix) => mix.id === next.id),
      edited = config.mixes.find((mix) => mix.id === next.id);
    if (!before || !edited) continue;
    Object.assign(edited, mergeControl(before, edited, next));
    for (const output of next.outputs) {
      if (!before.outputs.includes(output) || !edited.outputs.includes(output))
        continue;
      const old = controlValues(before.output_controls?.[output]),
        current = controlValues(edited.output_controls?.[output]),
        value = controlValues(next.output_controls?.[output]);
      if (!equal(old, value))
        edited.output_controls = {
          ...edited.output_controls,
          [output]: mergeControl(old, current, value) as {
            volume: number;
            muted: boolean;
          },
        };
    }
  }
  const dirty = !sameMixerConfig(config, incoming);
  return {
    config: dirty ? config : structuredClone(incoming),
    base: structuredClone(incoming),
    dirty,
  };
}
export function meterReading(level: MixerLevel | undefined) {
  if (!level?.available || !Number.isFinite(level.peak) || level.peak < 0)
    return {
      available: false,
      db: null,
      percent: 0,
      clipping: false,
      text: "Unavailable",
    };
  const db = level.peak === 0 ? -Infinity : 20 * Math.log10(level.peak);
  return {
    available: true,
    db,
    percent: Math.max(0, Math.min(100, ((db + 60) / 60) * 100)),
    clipping: level.peak >= 1,
    text: db === -Infinity ? "−∞ dBFS" : `${db.toFixed(1)} dBFS`,
  };
}
export const controlKey = (target: MixerControlTarget) =>
  JSON.stringify(canonical(target));
export function savedControl(config: MixerConfig, target: MixerControlTarget) {
  switch (target.kind) {
    case "preferences":
    case "monitor":
      return true;
    case "channel":
      return config.channels.some((c) => c.id === target.channel);
    case "mix":
      return config.mixes.some((m) => m.id === target.mix);
    case "output":
      return config.mixes.some(
        (m) => m.id === target.mix && m.outputs.includes(target.output),
      );
    case "route":
      return config.routes.some(
        (r) => r.channel === target.channel && r.mix === target.mix,
      );
  }
}
/** Optimistic presentation only. Rust owns validation, linked deltas and persistence. */
export function patchMixerControl(
  config: MixerConfig,
  target: MixerControlTarget,
  patch: MixerControlPatch,
  saved: MixerConfig = config,
): MixerConfig {
  const next = structuredClone(config);
  if (target.kind === "monitor") {
    next.monitoring = {
      output: next.monitoring?.output ?? null,
      mix: next.monitoring?.mix ?? null,
    };
    if (patch.output !== undefined) next.monitoring.output = patch.output;
    if (patch.mix !== undefined) next.monitoring.mix = patch.mix;
    return next;
  }
  if (target.kind === "preferences") {
    next.fade_ms = patch.fade_ms;
    return next;
  }
  if (target.kind === "channel") {
    const channel = next.channels.find((c) => c.id === target.channel);
    if (channel && patch.linked !== undefined) channel.linked = patch.linked;
    return next;
  }
  let control: { volume: number; muted: boolean } | undefined;
  if (target.kind === "route") {
    control = next.routes.find(
      (r) => r.channel === target.channel && r.mix === target.mix,
    );
    if (
      control &&
      patch.volume !== undefined &&
      next.channels.find((c) => c.id === target.channel)?.linked
    ) {
      const delta = patch.volume - control.volume;
      next.routes
        .filter(
          (r) =>
            r.channel === target.channel &&
            r !== control &&
            saved.routes.some(
              (old) => old.channel === r.channel && old.mix === r.mix,
            ),
        )
        .forEach((r) => {
          r.volume = Math.max(0, Math.min(100, r.volume + delta));
        });
    }
  } else {
    const mix = next.mixes.find((m) => m.id === target.mix);
    if (target.kind === "mix") control = mix;
    else if (mix && mix.outputs.includes(target.output)) {
      mix.output_controls ??= {};
      control = mix.output_controls[target.output] ??= {
        volume: 100,
        muted: false,
      };
    }
  }
  if (control) {
    if (patch.volume !== undefined) control.volume = patch.volume;
    if (patch.muted !== undefined) control.muted = patch.muted;
  }
  return next;
}
export interface MixerControlIntent {
  target: MixerControlTarget;
  patch: MixerControlPatch;
}
/** One ordered RPC lane. Only consecutive unsent edits coalesce, preserving linked-fader order. */
export function createMixerControlQueue(
  send: (
    target: MixerControlTarget,
    patch: MixerControlPatch,
  ) => Promise<unknown>,
  onError: (error: unknown) => void = () => {},
  onChange: () => void = () => {},
) {
  const queue: MixerControlIntent[] = [];
  let running: MixerControlIntent | null = null,
    disposed = false;
  async function drain() {
    if (running || disposed) return;
    while (!disposed && queue.length) {
      running = queue.shift()!;
      onChange();
      try {
        await send(running.target, running.patch);
      } catch (error) {
        queue.length = 0;
        onError(error);
      }
      running = null;
      onChange();
    }
  }
  return {
    push(target: MixerControlTarget, patch: MixerControlPatch) {
      if (disposed) return;
      const last = queue.at(-1);
      if (last && controlKey(last.target) === controlKey(target))
        last.patch = { ...last.patch, ...patch };
      else queue.push({ target, patch });
      onChange();
      void drain();
    },
    clear() {
      queue.length = 0;
      onChange();
    },
    dispose() {
      disposed = true;
      queue.length = 0;
    },
    get pending() {
      return queue.length + (running ? 1 : 0);
    },
    get intents() {
      return [...(running ? [running] : []), ...queue];
    },
  };
}
