import { it, expect } from "vitest";
import { scopedCommand } from "./bridge";
it("binds positional, profile and context mutations to the supplied capture, without global mutable state", () => {
  for (const payload of [
    { command: "assign", controller: "Keypad", index: 0, uuid: "counter" },
    { command: "profile_create", name: "New" },
    { command: "settings", context: "owned", settings: { count: 2 } },
    { command: "device_controls", settings: { brightness: 40 } },
    { command: "soundboard_refresh" },
    {
      command: "slot_paste",
      controller: "Keypad",
      index: 0,
      expected_target: null,
    },
  ])
    expect(scopedCommand(payload, "device-A")).toEqual({
      ...payload,
      device: "device-A",
    });
  const payload = {
    command: "screensaver",
    device: "device-A",
    after_minutes: 15,
  };
  expect(scopedCommand(payload, "device-A")).toEqual(payload);
  expect(() => scopedCommand(payload, "device-B")).toThrow("different device");
});
it("keeps device selection, plugins and mixer commands global and preserves older unbound callers", () => {
  for (const payload of [
    { command: "device_select", device: "device-B" },
    { command: "install_plugin", path: "/package" },
    { command: "restart_plugin", id: "plugin" },
    { command: "reference_app", path: "/reference" },
    { command: "mixer_refresh" },
    { command: "mixer_config", settings: { enabled: false } },
  ])
    expect(scopedCommand(payload, "device-A")).toBe(payload);
  const legacy = { command: "settings", context: "one", settings: {} };
  expect(scopedCommand(legacy)).toBe(legacy);
});
