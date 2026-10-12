// @vitest-environment jsdom
import { afterEach, beforeEach, it, expect, vi } from "vitest";
import { installFixture } from "./fixture";
import type { Snapshot } from "./model";
import { editRoute } from "./Mixer";
beforeEach(installFixture);
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
