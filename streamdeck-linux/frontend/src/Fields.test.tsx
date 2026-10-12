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

it("offers every key/dial hotkey gesture, clears one chord, and preserves imported fields", async () => {
  const user = userEvent.setup(),
    onChange = vi.fn();
  const original = {
    Coalesce: true,
    Future: { keep: true },
    Hotkeys: [
      {
        QTKeyCode: 48,
        VKeyCode: 96,
        NativeCode: 96,
        KeyModifiers: 0x4002,
        KeyCtrl: true,
        FutureChord: 9,
      },
      { QTKeyCode: 66, VKeyCode: 66, KeyModifiers: 0 },
      { QTKeyCode: 67, VKeyCode: 67, KeyModifiers: 0 },
    ],
  };
  render(
    <SettingsFields
      uuid="com.elgato.streamdeck.system.hotkey"
      controller="Encoder"
      settings={original}
      onChange={onChange}
      onError={vi.fn()}
    />,
  );
  expect(screen.getByRole("group", { name: "Turn left" })).toBeTruthy();
  expect(screen.getByRole("group", { name: "Turn right" })).toBeTruthy();
  expect(screen.getByRole("group", { name: "Press or tap" })).toBeTruthy();
  expect(
    screen
      .getAllByRole("option", { name: "Numpad 0" })[0]
      .getAttribute("selected"),
  ).toBeNull();
  const keys = screen.getAllByRole("combobox", {
    name: "Key",
  }) as HTMLSelectElement[];
  expect(keys[0].selectedOptions[0].textContent).toBe("Numpad 0");
  await user.selectOptions(
    keys[0],
    screen.getAllByRole("option", { name: "A" })[0],
  );
  const changed = onChange.mock.calls.at(-1)![0];
  expect(changed.Hotkeys[0]).toEqual({
    QTKeyCode: 65,
    VKeyCode: 65,
    KeyModifiers: 2,
    KeyCtrl: true,
    KeyShift: false,
    KeyOption: false,
    KeyCmd: false,
    FutureChord: 9,
  });
  expect(changed.Hotkeys.slice(1)).toEqual(original.Hotkeys.slice(1));
  expect(changed.Coalesce).toBe(true);
  expect(changed.Future).toEqual({ keep: true });
  await user.click(screen.getByRole("button", { name: "Clear turn left" }));
  const cleared = onChange.mock.calls.at(-1)![0].Hotkeys[0];
  expect(cleared.QTKeyCode).toBe(0x01ffffff);
  expect(cleared.VKeyCode).toBe(-1);
  expect(cleared.KeyModifiers).toBe(0);
  expect(cleared.NativeCode).toBeUndefined();
  expect(cleared.FutureChord).toBe(9);
});

it("supports modifier-only shortcuts and typed punctuation without normalizing untouched imports", async () => {
  const user = userEvent.setup(),
    onChange = vi.fn();
  const settings = {
    Hotkeys: [
      {
        QTKeyCode: 65,
        VKeyCode: 65,
        KeyModifiers: 2,
        KeyCtrl: true,
        Future: "keep",
      },
    ],
  };
  render(
    <SettingsFields
      uuid="com.elgato.streamdeck.system.hotkey"
      controller="Keypad"
      settings={settings}
      onChange={onChange}
      onError={vi.fn()}
    />,
  );
  expect(onChange).not.toHaveBeenCalled();
  await user.selectOptions(
    screen.getByRole("combobox", { name: "Key" }),
    "none",
  );
  expect(onChange.mock.calls.at(-1)![0].Hotkeys[0]).toMatchObject({
    QTKeyCode: 0x01ffffff,
    VKeyCode: -1,
    KeyModifiers: 2,
    KeyCtrl: true,
    Future: "keep",
  });
  expect(screen.getByRole("option", { name: "Semicolon" })).toBeTruthy();
  expect(screen.getByRole("option", { name: "Numpad divide" })).toBeTruthy();
});

it("requires explicit Linux output choice for imported soundboard routes and hides inactive fade settings", async () => {
  const user = userEvent.setup(),
    onChange = vi.fn();
  const settings = {
    path: "/audio.wav",
    fadeType: 0,
    fadeLen: 2,
    outputType: "windows-device-id",
    Future: 42,
  };
  render(
    <SettingsFields
      uuid="com.elgato.streamdeck.soundboard.playaudio"
      controller="Keypad"
      settings={settings}
      outputs={[["sink-a", "Speakers"]]}
      onChange={onChange}
      onError={vi.fn()}
    />,
  );
  expect(screen.getByText(/imported output has no Linux mapping/)).toBeTruthy();
  expect(screen.queryByRole("spinbutton", { name: "fadeLen" })).toBeNull();
  expect(screen.queryByRole("textbox", { name: "outputType" })).toBeNull();
  await user.selectOptions(
    screen.getByRole("combobox", { name: "Audio output" }),
    "",
  );
  expect(onChange).toHaveBeenLastCalledWith({ ...settings, linuxOutput: "" });
});

it("provides folder targets on empty settings and preserves unavailable imported targets", async () => {
  const user = userEvent.setup(),
    onChange = vi.fn();
  render(
    <SettingsFields
      uuid="com.elgato.streamdeck.profile.openchild"
      controller="Keypad"
      settings={{ ProfileUUID: "missing", Future: true }}
      folderPages={[["folder-id", "My folder"]]}
      onChange={onChange}
      onError={vi.fn()}
    />,
  );
  const select = screen.getByRole("combobox", {
    name: "Folder page",
  }) as HTMLSelectElement;
  expect(select.value).toBe("missing");
  await user.selectOptions(select, "folder-id");
  expect(onChange).toHaveBeenLastCalledWith({
    ProfileUUID: "folder-id",
    Future: true,
  });
});

it("shows honest no-option guidance, bounded delay, and fractional mixer values", () => {
  const props = {
    controller: "Keypad" as const,
    settings: {},
    onChange: vi.fn(),
    onError: vi.fn(),
  };
  const view = render(
    <SettingsFields
      {...props}
      uuid="com.elgato.streamdeck.soundboard.stopaudioplay"
    />,
  );
  expect(
    screen.getByText(/Stops all sounds started by Deckforge/),
  ).toBeTruthy();
  view.rerender(
    <SettingsFields
      {...props}
      uuid="com.elgato.streamdeck.multiactions.delay"
    />,
  );
  expect(
    (
      screen.getByRole("spinbutton", {
        name: "Delay (milliseconds)",
      }) as HTMLInputElement
    ).max,
  ).toBe("86400000");
  view.rerender(
    <SettingsFields
      {...props}
      uuid="com.deckforge.mixer.mix"
      settings={{ mode: "set_volume", volume: 12.5 }}
      mixer={mixer}
    />,
  );
  expect(
    (screen.getByRole("spinbutton", { name: "Volume (%)" }) as HTMLInputElement)
      .step,
  ).toBe("0.1");
  view.rerender(
    <SettingsFields {...props} uuid="com.elgato.streamdeck.system.sleep" />,
  );
  expect(screen.getByText(/device containing this action/)).toBeTruthy();
  expect(screen.queryByText(/all connected devices/)).toBeNull();
  view.rerender(
    <SettingsFields
      {...props}
      uuid="com.elgato.streamdeck.system.website"
      settings={{
        path: "https://example.test",
        openInBrowser: false,
        browser: "kept",
      }}
    />,
  );
  expect(screen.queryByRole("textbox", { name: "browser" })).toBeNull();
  expect(props.onChange).not.toHaveBeenCalled();
});

it("exposes official sample controls even with empty settings and hides runtime-only fields", async () => {
  const user = userEvent.setup(),
    onChange = vi.fn();
  const props = { controller: "Keypad" as const, onChange, onError: vi.fn() };
  const view = render(
    <SettingsFields
      {...props}
      uuid="com.elgato.counter.action"
      settings={{}}
    />,
  );
  const increment = screen.getByRole("spinbutton", {
    name: "Increment by",
  }) as HTMLInputElement;
  expect(increment.value).toBe("1");
  expect(increment.min).toBe("1");
  expect(increment.max).toBe("5");
  await user.clear(increment);
  await user.type(increment, "3");
  expect(onChange).toHaveBeenLastCalledWith({ incrementBy: 3 });
  view.rerender(
    <SettingsFields
      {...props}
      uuid="com.elgato.counter.action"
      settings={{ count: 7, incrementBy: 3, Future: true }}
    />,
  );
  expect(screen.queryByRole("spinbutton", { name: "count" })).toBeNull();
  view.rerender(
    <SettingsFields
      {...props}
      uuid="com.elgato.hello-world.newstring"
      settings={{ titleKey: "clickme", language: "de", Future: 9 }}
    />,
  );
  expect(screen.queryByRole("textbox", { name: "titleKey" })).toBeNull();
  await user.selectOptions(
    screen.getByRole("combobox", { name: "Sample language" }),
    "",
  );
  expect(onChange).toHaveBeenLastCalledWith({ titleKey: "clickme", Future: 9 });
  await user.selectOptions(
    screen.getByRole("combobox", { name: "Sample language" }),
    "fr",
  );
  expect(onChange).toHaveBeenLastCalledWith({
    titleKey: "clickme",
    language: "fr",
    Future: 9,
  });
  view.rerender(
    <SettingsFields
      {...props}
      uuid="com.elgato.layouts.feedback"
      controller="Encoder"
      settings={{ value: 50 }}
    />,
  );
  expect(screen.getByText(/This sample has no authored settings/)).toBeTruthy();
  expect(screen.queryByRole("spinbutton", { name: "value" })).toBeNull();
});

it("retains out-of-range imported Counter increments without blocking unrelated edits", async () => {
  const user = userEvent.setup(),
    onChange = vi.fn();
  render(
    <form>
      <SettingsFields
        uuid="com.elgato.counter.action"
        controller="Keypad"
        settings={{ incrementBy: 7, count: 42, Future: true }}
        onChange={onChange}
        onError={vi.fn()}
      />
    </form>,
  );
  expect(screen.getByText(/Custom increment: 7/)).toBeTruthy();
  expect(document.querySelector("form")!.checkValidity()).toBe(true);
  expect(onChange).not.toHaveBeenCalled();
  await user.click(screen.getByRole("checkbox", { name: "Future" }));
  expect(onChange).toHaveBeenLastCalledWith({
    incrementBy: 7,
    count: 42,
    Future: false,
  });
  await user.click(
    screen.getByRole("button", { name: "Use standard increment" }),
  );
  expect(onChange).toHaveBeenLastCalledWith({
    incrementBy: 1,
    count: 42,
    Future: true,
  });
});
