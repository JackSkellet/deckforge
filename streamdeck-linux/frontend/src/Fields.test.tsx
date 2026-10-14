// @vitest-environment jsdom
import { afterEach, it, expect, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { SettingsFields } from "./Fields";
import type { MixerConfig } from "./Mixer";
import type { RecordValue } from "./model";
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
it("edits terminal Text paste shortcuts and preserves them while typing mode is selected", async () => {
  const user=userEvent.setup();let current:any;
  function Harness(){const [settings,setSettings]=useState<RecordValue>({pastedText:"hello",isTypingMode:false,linuxKeyMode:"logical",Future:7});current=settings;return <SettingsFields uuid="com.elgato.streamdeck.system.text" controller="Keypad" settings={settings} onChange={setSettings} onError={vi.fn()} />;}
  render(<Harness />);
  await user.click(screen.getByText("Paste shortcut"));
  await user.click(screen.getByLabelText("Shift"));
  expect(current.Hotkey).toMatchObject({QTKeyCode:86,KeyModifiers:3,linuxKeyMode:"logical"});
  const shortcut=current.Hotkey;
  await user.selectOptions(screen.getByLabelText("Text mode"),"typing");
  expect(screen.queryByText("Paste shortcut")).toBeNull();
  expect(current.Hotkey).toEqual(shortcut);
  await user.selectOptions(screen.getByLabelText("Text mode"),"paste");
  await user.click(screen.getByText("Paste shortcut"));
  expect((screen.getByLabelText("Shift") as HTMLInputElement).checked).toBe(true);
  await user.click(screen.getByRole("button",{name:"Reset paste shortcut"}));
  expect(current.Hotkey).toBeUndefined();
  expect(current.linuxKeyMode).toBe("logical");
  expect(current.Future).toBe(7);
});
it("renders legacy modifier markers and normalizes only the edited chord", async () => {
  const user=userEvent.setup();let current:any;
  function Harness(){const [settings,setSettings]=useState<RecordValue>({Hotkeys:[{QTKeyCode:65,KeyModifiers:8192,KeyCtrl:true,Future:9}],Future:7});current=settings;return <SettingsFields uuid="com.elgato.streamdeck.system.hotkey" controller="Keypad" settings={settings} onChange={setSettings} onError={vi.fn()} />;}
  render(<Harness />);
  expect((screen.getByLabelText("Key") as HTMLSelectElement).selectedOptions[0].textContent).toBe("A");
  expect((screen.getByLabelText("Ctrl") as HTMLInputElement).checked).toBe(true);
  expect(current.Hotkeys[0].KeyModifiers).toBe(8192);
  await user.click(screen.getByLabelText("Shift"));
  expect(current).toEqual({Hotkeys:[{QTKeyCode:65,KeyModifiers:3,KeyCtrl:true,KeyShift:true,Future:9}],Future:7});
});
it("shows the authoritative modern mask despite stale legacy fields", async () => {
  const onChange=vi.fn();
  render(<SettingsFields uuid="com.elgato.streamdeck.system.hotkey" controller="Keypad" settings={{Hotkeys:[{QTKeyCode:65,KeyModifiers:2,KeyCtrl:false,KeyShift:true,Future:9}]}} onChange={onChange} onError={vi.fn()} />);
  expect((screen.getByLabelText("Ctrl") as HTMLInputElement).checked).toBe(true);
  expect((screen.getByLabelText("Shift") as HTMLInputElement).checked).toBe(false);
  expect(onChange).not.toHaveBeenCalled();
  await userEvent.selectOptions(screen.getByLabelText("Key"),"1");
  expect(onChange.mock.calls.at(-1)?.[0].Hotkeys[0]).toMatchObject({QTKeyCode:66,KeyModifiers:2,KeyCtrl:true,KeyShift:false,Future:9});
});
it("retains modifier sides across key selection and switches explicitly to generic modifiers", async () => {
  const user = userEvent.setup(); let current: any;
  function Harness() {
    const [settings, setSettings] = useState<RecordValue>({Hotkeys:[{QTKeyCode:65,VKeyCode:65,KeyModifiers:0x10404,Future:9}],Future:7});
    current=settings;
    return <SettingsFields uuid="com.elgato.streamdeck.system.hotkey" controller="Keypad" settings={settings} onChange={setSettings} onError={vi.fn()} />;
  }
  render(<Harness />);
  expect((screen.getByLabelText("Alt side") as HTMLSelectElement).value).toBe("2");
  await user.selectOptions(screen.getByLabelText("Ctrl side"),"3");
  await user.selectOptions(screen.getByLabelText("Key"),"16");
  expect(current.Hotkeys[0]).toMatchObject({QTKeyCode:81,KeyModifiers:0x10626,KeyCtrl:true,KeyOption:true,Future:9});
  await user.selectOptions(screen.getByLabelText("Key mapping"),"logical");
  expect(current.Hotkeys[0].linuxKeyMode).toBe("logical");
  await user.click(screen.getByLabelText("Choose modifier sides"));
  expect(current.Hotkeys[0].KeyModifiers).toBe(6);
  await user.click(screen.getByLabelText("Choose modifier sides"));
  expect(current.Hotkeys[0].KeyModifiers).toBe(0x10066);
  expect(current.Future).toBe(7);
});
it("switches clipboard operations without discarding saved Unicode text", async () => {
  const user = userEvent.setup(); let current: any;
  function Harness() {
    const [settings, setSettings] = useState<RecordValue>({ operation: "copy_text", text: "Hei 🌍", terminal_shortcuts: true, Future: 7 });
    current = settings;
    return <SettingsFields uuid="com.deckforge.system.clipboard" controller="Keypad" settings={settings} onChange={setSettings} onError={vi.fn()} />;
  }
  render(<Harness />);
  await user.selectOptions(screen.getByLabelText("Clipboard operation"), "paste");
  expect(screen.getByText(/Sends Ctrl\+Shift\+V/)).toBeTruthy();
  await user.selectOptions(screen.getByLabelText("Clipboard operation"), "cut");
  await user.click(screen.getByRole("button", { name: "Use standard Cut shortcut" }));
  expect(current).toEqual({ operation: "cut", text: "Hei 🌍", terminal_shortcuts: false, Future: 7 });
  await user.selectOptions(screen.getByLabelText("Clipboard operation"), "copy_text");
  expect((screen.getByLabelText("Clipboard text") as HTMLTextAreaElement).value).toBe("Hei 🌍");
});
it("retains program arguments while editing an explicit shell command and terminal", async () => {
  const user = userEvent.setup(); let current: any;
  function Harness() {
    const [settings, setSettings] = useState<RecordValue>({ mode: "program", program: "echo", args: "'two words'", script: "", terminal: false, output: "clipboard", timeout_seconds: 45, Future: 8 });
    current = settings;
    return <SettingsFields uuid="com.deckforge.system.command" controller="Keypad" settings={settings} onChange={setSettings} onError={vi.fn()} />;
  }
  render(<Harness />);
  await user.selectOptions(screen.getByLabelText("Command mode"), "shell");
  await user.type(screen.getByLabelText("Shell script"), "printf hello | cat");
  await user.click(screen.getByLabelText("Run in terminal window"));
  expect(screen.getByLabelText(/^Terminal arguments/)).toBeTruthy();
  expect(screen.queryByLabelText("Command output")).toBeNull();
  expect(current).toMatchObject({ mode: "shell", script: "printf hello | cat", terminal: true, program: "echo", args: "'two words'", output: "clipboard", timeout_seconds: 45, Future: 8 });
  await user.click(screen.getByLabelText("Run in terminal window"));
  expect((screen.getByLabelText("Command output") as HTMLSelectElement).value).toBe("clipboard");
  expect((screen.getByLabelText("Timeout (seconds)") as HTMLInputElement).value).toBe("45");
});
it("edits both switch shortcuts and retains unknown settings", async () => {
  const user = userEvent.setup(),
    onChange = vi.fn();
  render(
    <SettingsFields
      uuid="com.elgato.streamdeck.system.hotkeyswitch"
      controller="Keypad"
      settings={{ Hotkeys: [{}, {}], Future: 9 }}
      onChange={onChange}
      onError={vi.fn()}
    />,
  );
  expect(screen.getByText("First shortcut")).toBeTruthy();
  expect(screen.getByText("Second shortcut")).toBeTruthy();
  const selects = screen.getAllByLabelText("Key");
  await user.selectOptions(selects[1], "0");
  expect(onChange.mock.calls.at(-1)![0].Hotkeys[1].QTKeyCode).toBe(65);
  expect(onChange.mock.calls.at(-1)![0].Future).toBe(9);
});
it("retains Unicode Text and makes imported typing mode explicit", async () => {
  const user = userEvent.setup(),
    onChange = vi.fn();
  render(
    <SettingsFields
      uuid="com.elgato.streamdeck.system.text"
      controller="Keypad"
      settings={{ pastedText: "Hei 🌍", isTypingMode: true, Future: 7 }}
      onChange={onChange}
      onError={vi.fn()}
    />,
  );
  expect(
    (screen.getByRole("textbox", { name: "Text" }) as HTMLTextAreaElement)
      .value,
  ).toBe("Hei 🌍");
  await user.selectOptions(screen.getByLabelText("Text mode"), "paste");
  expect(onChange).toHaveBeenLastCalledWith({
    pastedText: "Hei 🌍",
    isTypingMode: false,
    Future: 7,
  });
});
it.each([null, "true", 0])("preserves unsupported saved Text mode %s until explicitly changed", async (mode) => {
  const onChange = vi.fn();
  render(<SettingsFields uuid="com.elgato.streamdeck.system.text" controller="Keypad" settings={{ pastedText: "keep", isTypingMode: mode, Future: 7 }} onChange={onChange} onError={vi.fn()} />);
  expect((screen.getByLabelText("Text mode") as HTMLSelectElement).value).toBe("invalid");
  expect(onChange).not.toHaveBeenCalled();
  await userEvent.selectOptions(screen.getByLabelText("Text mode"), "paste");
  expect(onChange).toHaveBeenCalledWith({ pastedText: "keep", isTypingMode: false, Future: 7 });
});
it("Close preserves legacy automatic behavior and the selected executable", async () => {
  const user = userEvent.setup(),
    onChange = vi.fn();
  render(
    <SettingsFields
      uuid="com.elgato.streamdeck.system.close"
      controller="Keypad"
      settings={{ path: "/tmp/owned app", forceQuit: false, Future: 2 }}
      onChange={onChange}
      onError={vi.fn()}
    />,
  );
  expect((screen.getByLabelText("Close behavior") as HTMLSelectElement).value).toBe("automatic");
  await user.click(screen.getByRole("checkbox", { name: "Force close" }));
  expect(onChange).toHaveBeenLastCalledWith({
    path: "/tmp/owned app",
    forceQuit: true,
    Future: 2,
  });
});
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
it("edits dial Coalesce without rewriting chords or unknown fields", async () => {
  const user = userEvent.setup(),
    onChange = vi.fn();
  const settings = {
    Hotkeys: [{ QTKeyCode: 65 }, { QTKeyCode: 66 }, { QTKeyCode: 67 }],
    Future: 9,
  };
  render(
    <SettingsFields
      uuid="com.elgato.streamdeck.system.hotkey"
      controller="Encoder"
      settings={settings}
      onChange={onChange}
      onError={vi.fn()}
    />,
  );
  const checkbox = screen.getByRole("checkbox", {
    name: "Limit queued dial shortcuts",
  }) as HTMLInputElement;
  expect(checkbox.checked).toBe(true);
  expect(onChange).not.toHaveBeenCalled();
  await user.click(checkbox);
  expect(onChange).toHaveBeenLastCalledWith({ ...settings, Coalesce: false });
});
it("Open Application keeps imported fields and clears the desktop binding when editing the executable", async () => {
  const user=userEvent.setup(), onChange=vi.fn();
  render(<SettingsFields uuid="com.elgato.streamdeck.system.openapp" controller="Keypad"
    settings={{exec:"/old/app",source:"/old/app.desktop",bundle_id:"old",bundle_path:"old",is_bundle:true,Future:7}}
    onChange={onChange} onError={vi.fn()} />);
  await user.type(screen.getByRole("textbox",{name:"Application executable"}),"x");
  expect(onChange.mock.calls.at(-1)![0]).toMatchObject({source:"",bundle_id:"",bundle_path:"",is_bundle:false,Future:7});
  await user.selectOptions(screen.getByRole("combobox",{name:"When held"}),"force_kill");
  expect(onChange.mock.calls.at(-1)![0].long_press).toBe("force_kill");
});
