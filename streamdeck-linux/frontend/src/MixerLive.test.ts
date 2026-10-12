import { expect, it, vi } from "vitest";
import {
  createMixerControlQueue,
  meterReading,
  reconcileMixerDraft,
  sameMixerRouting,
  sameMixerConfig,
  patchMixerControl,
} from "./MixerLive";
import { removeMix, type MixerConfig } from "./Mixer";
const config: MixerConfig = {
  enabled: true,
  channels: [{ id: "voice", name: "Voice", apps: [], sources: [] }],
  mixes: [
    {
      id: "monitor",
      name: "Monitor",
      outputs: ["speakers"],
      volume: 90,
      muted: false,
      output_controls: { speakers: { volume: 80, muted: false } },
    },
    { id: "stream", name: "Stream", outputs: [], volume: 100, muted: false },
  ],
  routes: [
    { channel: "voice", mix: "monitor", volume: 40, muted: false },
    { channel: "voice", mix: "stream", volume: 60, muted: false },
  ],
};
it("rebases live gains/mutes by identity while retaining unapplied routing/name edits", () => {
  const edited = structuredClone(config);
  edited.channels[0].name = "New voice name";
  edited.channels[0].sources = ["pending-mic"];
  edited.mixes[0].outputs = [];
  edited.routes = edited.routes.filter((route) => route.mix !== "stream");
  const latest = structuredClone(config);
  latest.routes[0].volume = 73;
  latest.routes[0].muted = true;
  latest.mixes[0].volume = 55;
  latest.mixes[0].output_controls!.speakers.volume = 25;
  const result = reconcileMixerDraft(
    { config: edited, base: config, dirty: true },
    latest,
  );
  expect(result.base).toEqual(latest);
  expect(result.config.channels[0]).toMatchObject({
    name: "New voice name",
    sources: ["pending-mic"],
  });
  expect(result.config.routes).toEqual([
    { channel: "voice", mix: "monitor", volume: 73, muted: true },
  ]);
  expect(result.config.mixes[0].outputs).toEqual([]);
  expect(result.config.mixes[0].volume).toBe(55);
  expect(result.dirty).toBe(true);
});
it("preserves authored offline gain values but updates untouched output gain fields", () => {
  const edited = structuredClone(config);
  edited.routes[0].volume = 12;
  edited.mixes[0].output_controls!.speakers.muted = true;
  const latest = structuredClone(config);
  latest.routes[0].volume = 66;
  latest.mixes[0].output_controls!.speakers.volume = 33;
  const result = reconcileMixerDraft(
    { config: edited, base: config, dirty: true },
    latest,
  );
  expect(result.config.routes[0].volume).toBe(12);
  expect(result.config.mixes[0].output_controls!.speakers).toEqual({
    volume: 33,
    muted: true,
  });
  expect(result.base).toEqual(latest);
});
it("keeps structural conflicts stale and normalizes only legacy defaults for comparison", () => {
  const draft = {
    base: config,
    config: { ...config, enabled: false },
    dirty: true,
  };
  const latest = structuredClone(config);
  latest.channels[0].sources = ["external-mic"];
  expect(reconcileMixerDraft(draft, latest)).toBe(draft);
  expect(
    sameMixerRouting(config, {
      ...config,
      fade_ms: 0,
      channels: config.channels.map((c) => ({ ...c, linked: false })),
    }),
  ).toBe(true);
  expect(sameMixerRouting(config, { ...config, fade_ms: 500 })).toBe(true);
});
it("distinguishes unavailable meters, measured silence, dBFS levels and full scale", () => {
  expect(meterReading(undefined)).toMatchObject({
    available: false,
    text: "Unavailable",
  });
  expect(meterReading({ available: true, peak: NaN }).available).toBe(false);
  expect(meterReading({ available: true, peak: 0 })).toMatchObject({
    available: true,
    text: "−∞ dBFS",
    percent: 0,
    clipping: false,
  });
  expect(meterReading({ available: true, peak: 0.5 }).db).toBeCloseTo(
    -6.0206,
    3,
  );
  expect(meterReading({ available: true, peak: 1 })).toMatchObject({
    available: true,
    percent: 100,
    clipping: true,
  });
});
it("coalesces latest values per control, retains mute changes, and permits independent targets", async () => {
  const completions: (() => void)[] = [];
  const send = vi.fn(
    () => new Promise<void>((resolve) => completions.push(resolve)),
  );
  const queue = createMixerControlQueue(send);
  const target = { kind: "route" as const, channel: "voice", mix: "monitor" };
  queue.push(target, { volume: 10 });
  queue.push(target, { volume: 20 });
  queue.push(target, { muted: true });
  queue.push(target, { volume: 30 });
  queue.push({ kind: "mix", mix: "stream" }, { volume: 80 });
  expect(send).toHaveBeenCalledTimes(1);
  expect(send.mock.calls[0]).toEqual([target, { volume: 10 }]);
  completions[0]();
  await Promise.resolve();
  await Promise.resolve();
  expect(send).toHaveBeenCalledTimes(2);
  expect(send.mock.calls[1]).toEqual([target, { volume: 30, muted: true }]);
  queue.dispose();
  completions[1]();
  await Promise.resolve();
  expect(send).toHaveBeenCalledTimes(2);
});
it("drops unsent controls on graph changes without inventing a cancellation of the in-flight command", async () => {
  let done: () => void = () => {};
  const send = vi.fn(() => new Promise<void>((resolve) => (done = resolve)));
  const queue = createMixerControlQueue(send);
  const target = { kind: "mix" as const, mix: "monitor" };
  queue.push(target, { volume: 50 });
  queue.push(target, { volume: 70 });
  queue.clear();
  done();
  await Promise.resolve();
  await Promise.resolve();
  expect(send).toHaveBeenCalledTimes(1);
});

it("preserves cross-target order because linked route deltas do not commute", async () => {
  const state = structuredClone(config);
  state.channels[0].linked = true;
  const values: number[][] = [];
  let current = state;
  let finish!: () => void;
  const gate = new Promise<void>((resolve) => {
    finish = resolve;
  });
  const queue = createMixerControlQueue(async (target, patch) => {
    await gate;
    current = patchMixerControl(current, target, patch);
    values.push(current.routes.map((r) => r.volume));
  });
  queue.push(
    { kind: "route", channel: "voice", mix: "monitor" },
    { volume: 80 },
  );
  queue.push(
    { kind: "route", channel: "voice", mix: "stream" },
    { volume: 50 },
  );
  queue.push(
    { kind: "route", channel: "voice", mix: "monitor" },
    { volume: 70 },
  );
  finish();
  for (let i = 0; i < 10; i++) await Promise.resolve();
  expect(values).toEqual([
    [80, 100],
    [30, 50],
    [70, 90],
  ]);
  expect(queue.pending).toBe(0);
});
it("rebases immediate preferences and link settings without discarding routing drafts", () => {
  const draft = structuredClone(config);
  draft.channels[0].name = "Voice pending";
  const incoming = structuredClone(config);
  incoming.fade_ms = 250;
  incoming.channels[0].linked = true;
  const result = reconcileMixerDraft(
    { config: draft, base: config, dirty: true },
    incoming,
  );
  expect(result.config.fade_ms).toBe(250);
  expect(result.config.channels[0]).toMatchObject({
    name: "Voice pending",
    linked: true,
  });
});

it("normalizes a missing catch-all flag as false but guards a changed default group", () => {
  const explicit = structuredClone(config);
  explicit.channels[0].capture_unassigned = false;
  expect(sameMixerConfig(config, explicit)).toBe(true);
  expect(sameMixerRouting(config, explicit)).toBe(true);
  explicit.channels[0].capture_unassigned = true;
  expect(sameMixerConfig(config, explicit)).toBe(false);
  expect(sameMixerRouting(config, explicit)).toBe(false);
});

it("rebases monitoring preferences without losing routing drafts or creating false structural conflicts", () => {
  const before = structuredClone(config),
    edited = structuredClone(config);
  edited.channels[0].name = "Voice draft";
  const incoming = {
    ...structuredClone(config),
    monitoring: { output: "headphones", mix: "monitor" },
  };
  const result = reconcileMixerDraft(
    { base: before, config: edited, dirty: true },
    incoming,
  );
  expect(result.config.monitoring).toEqual(incoming.monitoring);
  expect(result.config.channels[0].name).toBe("Voice draft");
  expect(sameMixerRouting(before, incoming)).toBe(true);
  expect(
    sameMixerConfig(before, {
      ...before,
      monitoring: { output: null, mix: null },
    }),
  ).toBe(true);
});
it("clears a removed monitored mix only in the structural draft and retains later live selections", () => {
  const before = {
    ...structuredClone(config),
    monitoring: { output: "headphones", mix: "monitor" },
  };
  const edited = removeMix(before, "monitor");
  expect(edited.monitoring).toEqual({ output: "headphones", mix: null });
  expect(before.monitoring.mix).toBe("monitor");
  let merged = reconcileMixerDraft(
    { base: before, config: edited, dirty: true },
    before,
  );
  expect(merged.config.monitoring).toEqual({ output: "headphones", mix: null });
  merged = reconcileMixerDraft(merged, {
    ...before,
    monitoring: { output: "headphones", mix: "stream" },
  });
  expect(merged.config.monitoring).toEqual({
    output: "headphones",
    mix: "stream",
  });
  expect(merged.config.mixes.map((m) => m.id)).toEqual(["stream"]);
});
it("patches explicit null monitor controls without resetting omitted values", () => {
  let next = patchMixerControl(
    config,
    { kind: "monitor" },
    { output: "headphones", mix: "monitor" },
  );
  next = patchMixerControl(next, { kind: "monitor" }, { mix: null });
  expect(next.monitoring).toEqual({ output: "headphones", mix: null });
  next = patchMixerControl(next, { kind: "monitor" }, { output: null });
  expect(next.monitoring).toEqual({ output: null, mix: null });
});
