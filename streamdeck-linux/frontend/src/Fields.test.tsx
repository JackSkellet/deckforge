// @vitest-environment jsdom
import { afterEach, it, expect, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { SettingsFields } from "./Fields";
import type { MixerConfig } from "./Mixer";
const mixer: MixerConfig = {
  enabled: false,
  channels: [{ id: "music", name: "Music", apps: [], sources: [] }],
  mixes: [
    { id: "monitor", name: "Monitor", outputs: [], volume: 100, muted: false },
    { id: "stream", name: "Stream", outputs: [], volume: 100, muted: false },
  ],
  routes: [],
};
afterEach(cleanup);
it("edits a mixer dial target while preserving unknown settings and unavailable identities", async () => {
  const user = userEvent.setup();
  const onChange = vi.fn();
  render(
    <SettingsFields
      uuid="com.deckforge.mixer.channel"
      controller="Encoder"
      settings={{
        channel: "missing",
        mix: "monitor",
        step: 5,
        Future: { keep: true },
      }}
      mixer={mixer}
      onChange={onChange}
      onError={vi.fn()}
    />,
  );
  expect(
    (screen.getByRole("combobox", { name: "Input group" }) as HTMLSelectElement)
      .value,
  ).toBe("missing");
  await user.selectOptions(
    screen.getByRole("combobox", { name: "Input group" }),
    "music",
  );
  expect(onChange).toHaveBeenCalledWith({
    channel: "music",
    mix: "monitor",
    step: 5,
    Future: { keep: true },
  });
  expect(
    screen.getByRole("spinbutton", { name: "Volume change per dial tick (%)" }),
  ).toBeTruthy();
  expect(screen.queryByRole("combobox", { name: "Key action" })).toBeNull();
});
it("shows key set-volume controls without changing the route configuration", () => {
  const onChange = vi.fn();
  render(
    <SettingsFields
      uuid="com.deckforge.mixer.mix"
      controller="Keypad"
      settings={{ mix: "stream", mode: "set_volume", volume: 32, step: 5 }}
      mixer={mixer}
      onChange={onChange}
      onError={vi.fn()}
    />,
  );
  expect(
    (screen.getByRole("spinbutton", { name: "Volume (%)" }) as HTMLInputElement)
      .value,
  ).toBe("32");
  expect(
    screen.queryByRole("spinbutton", {
      name: "Volume change per dial tick (%)",
    }),
  ).toBeNull();
  expect(onChange).not.toHaveBeenCalled();
  expect(mixer.enabled).toBe(false);
});
