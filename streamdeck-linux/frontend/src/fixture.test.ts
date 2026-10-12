// @vitest-environment jsdom
import { afterEach, beforeEach, it, expect, vi } from "vitest";
import { installFixture, applyFixtureSettingsPatch } from "./fixture";
import type { Snapshot } from "./model";
import { editRoute } from "./Mixer";
beforeEach(() => installFixture());
afterEach(() => {
  delete window.deckforge;
});
it("keeps mixer inventory refresh read-only and broadcasts current snapshots", async () => {
  const bridge = window.deckforge!;
  const before = await bridge.request<Snapshot>("snapshot");
  const listener = vi.fn();
  bridge.onSnapshot(listener);
  await bridge.request("command", { command: "mixer_refresh" });
  const after = await bridge.request<Snapshot>("snapshot");
  expect(after.mixer.config).toEqual(before.mixer.config);
  expect(after.mixer.active).toBe(false);
  expect(listener).toHaveBeenCalledWith(after);
});
it("applies independent mix gains, accepts reordered expected keys, and rejects stale drafts", async () => {
  const bridge = window.deckforge!;
  const before = await bridge.request<Snapshot>("snapshot");
  const settings = structuredClone(before.mixer.config);
  settings.enabled = true;
  settings.channels.push({ id: "music", name: "Music", apps: [], sources: [] });
  settings.routes.push(
    { channel: "music", mix: "monitor", volume: 30, muted: false },
    { channel: "music", mix: "stream", volume: 70, muted: true },
  );
  const expected = {
    routes: before.mixer.config.routes,
    mixes: before.mixer.config.mixes,
    channels: before.mixer.config.channels,
    enabled: before.mixer.config.enabled,
  };
  await bridge.request("command", {
    command: "mixer_config",
    settings,
    expected,
  });
  let observed = await bridge.request<Snapshot>("snapshot");
  expect(observed.mixer.config).toEqual(settings);
  expect(observed.mixer.active).toBe(true);
  expect(observed.mixer.pending).toBe(false);
  expect(observed.mixer.applied_config).toEqual(settings);
  const next = editRoute(settings, "music", "monitor", { volume: 36 });
  await bridge.request("command", {
    command: "mixer_config",
    settings: next,
    expected: settings,
  });
  observed = await bridge.request<Snapshot>("snapshot");
  expect(observed.mixer.config.routes).toEqual([
    { channel: "music", mix: "monitor", volume: 36, muted: false },
    { channel: "music", mix: "stream", volume: 70, muted: true },
  ]);
  await expect(
    bridge.request("command", {
      command: "mixer_config",
      settings: { ...settings, enabled: false },
      expected: before.mixer.config,
    }),
  ).rejects.toThrow("changed outside");
  expect((await bridge.request<Snapshot>("snapshot")).mixer.config).toEqual(
    next,
  );
  next.routes[0].volume = 99;
  expect(
    (await bridge.request<Snapshot>("snapshot")).mixer.config.routes[0].volume,
  ).toBe(36);
});
it("models saved-action copy, swap-on-move, cut cancellation and exact destinations", async () => {
  const bridge = window.deckforge!;
  const before = await bridge.request<Snapshot>("snapshot");
  const source = before.slots.find(
    (s) => s.controller === "Keypad" && s.index === 0,
  )!;
  const occupied = before.slots.find(
    (s) => s.controller === "Keypad" && s.index === 1,
  )!;
  await bridge.request("command", {
    command: "slot_transfer",
    context: source.context,
    controller: "Keypad",
    index: 1,
    copy: false,
    expected_target: occupied.context,
  });
  let observed = await bridge.request<Snapshot>("snapshot");
  expect(observed.slots.find((s) => s.context === source.context)?.index).toBe(
    1,
  );
  expect(
    observed.slots.find((s) => s.context === occupied.context)?.index,
  ).toBe(0);
  await bridge.request("command", {
    command: "slot_copy",
    context: source.context,
  });
  await bridge.request("command", {
    command: "settings",
    context: source.context,
    settings: { count: 99, incrementBy: 10 },
  });
  await bridge.request("command", {
    command: "slot_paste",
    controller: "Keypad",
    index: 0,
    expected_target: occupied.context,
  });
  observed = await bridge.request<Snapshot>("snapshot");
  const copied = observed.slots.find(
    (s) => s.controller === "Keypad" && s.index === 0,
  )!;
  expect(copied.context).not.toBe(source.context);
  expect(copied.settings).toEqual(source.settings);
  expect(
    observed.slots.find((s) => s.context === source.context)?.settings.count,
  ).toBe(99);
  await bridge.request("command", {
    command: "slot_cut",
    context: copied.context,
  });
  await bridge.request("command", { command: "slot_clipboard_clear" });
  observed = await bridge.request<Snapshot>("snapshot");
  expect(observed.clipboard).toBeNull();
  expect(observed.slots.some((s) => s.context === copied.context)).toBe(true);
  await bridge.request("command", {
    command: "slot_copy",
    context: source.context,
  });
  await expect(
    bridge.request("command", {
      command: "slot_paste",
      controller: "Keypad",
      index: 0,
      expected_target: "stale",
    }),
  ).rejects.toThrow("destination changed");
  expect((await bridge.request<Snapshot>("snapshot")).slots).toEqual(
    observed.slots,
  );
});

it("three-way settings patches preserve live values, recurse objects, replace edited arrays and honor deletions", () => {
  const base = {
    count: 10,
    incrementBy: 2,
    nested: { a: 1, b: 2 },
    array: [1, 2],
    deleted: true,
  };
  const edited = {
    count: 10,
    incrementBy: 7,
    nested: { a: 3, b: 2 },
    array: [1, 2],
  };
  const latest = {
    count: 12,
    incrementBy: 2,
    nested: { a: 1, b: 9, newField: "SDK" },
    array: [9],
    deleted: false,
    added: "SDK",
  };
  expect(applyFixtureSettingsPatch(base, edited, latest)).toEqual({
    count: 12,
    incrementBy: 7,
    nested: { a: 3, b: 9, newField: "SDK" },
    array: [9],
    added: "SDK",
  });
  expect(
    applyFixtureSettingsPatch(
      base,
      { ...edited, count: 99, array: [3, 4] },
      latest,
    ),
  ).toEqual({
    count: 99,
    incrementBy: 7,
    nested: { a: 3, b: 9, newField: "SDK" },
    array: [3, 4],
    added: "SDK",
  });
  expect(latest.count).toBe(12);
});
it("the synthetic command handler merges stale drafts while legacy settings still replaces", async () => {
  const bridge = window.deckforge!;
  const initial = await bridge.request<Snapshot>("snapshot");
  const slot = initial.slots[0];
  const base = structuredClone(slot.settings);
  await bridge.request("command", {
    command: "settings",
    context: slot.context,
    settings: { ...base, count: 99, sdkField: true },
  });
  await bridge.request("command", {
    command: "settings_patch",
    context: slot.context,
    base,
    settings: { ...base, incrementBy: 7 },
  });
  const saved = (await bridge.request<Snapshot>("snapshot")).slots[0].settings;
  expect(saved).toEqual({ ...base, count: 99, incrementBy: 7, sdkField: true });
  await bridge.request("command", {
    command: "settings",
    context: slot.context,
    settings: { count: 0 },
  });
  expect(
    (await bridge.request<Snapshot>("snapshot")).slots[0].settings,
  ).toEqual({ count: 0 });
});

it("returns authoritative control snapshots without enabling synthetic routing and guards graph revisions", async () => {
  const bridge = window.deckforge!;
  const before = (await bridge.request<Snapshot>("snapshot")).mixer;
  const listener = vi.fn();
  bridge.onSnapshot(listener);
  const changed = await bridge.request<typeof before>("mixer_control", {
    revision: before.revision,
    target: { kind: "mix", mix: "monitor" },
    volume: 42,
  });
  expect(changed.config.mixes[0].volume).toBe(42);
  expect(changed.config.enabled).toBe(false);
  expect(changed.active).toBe(false);
  expect(changed.revision).toBe(before.revision);
  expect(listener).toHaveBeenLastCalledWith(
    expect.objectContaining({ mixer: changed }),
  );
  const preferences = await bridge.request<typeof before>("mixer_control", {
    revision: changed.revision,
    target: { kind: "preferences" },
    fade_ms: 350,
  });
  expect(preferences.revision).toBe(changed.revision! + 1);
  await expect(
    bridge.request("mixer_control", {
      revision: changed.revision,
      target: { kind: "mix", mix: "monitor" },
      volume: 5,
    }),
  ).rejects.toThrow("routing changed");
  await expect(
    bridge.request("mixer_control", {
      revision: preferences.revision,
      target: { kind: "route", channel: "unsaved", mix: "monitor" },
      volume: 5,
    }),
  ).rejects.toThrow("Save the route");
});
