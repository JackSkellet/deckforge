// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { useState } from "react";
import { AudioMixer, type MixerConfig, type MixerSnapshot } from "./Mixer";
import { MixerPatchbay, connectPorts, routingProblems } from "./MixerPatchbay";
import { exportFormDrafts, importFormDrafts } from "./transient";
const configuration = (): MixerConfig => ({
  enabled: false,
  channels: [
    { id: "voice", name: "Voice", sources: [], apps: [] },
    { id: "other", name: "Other", sources: [], apps: [] },
  ],
  mixes: [
    { id: "monitor", name: "Personal", outputs: [], volume: 100, muted: false },
  ],
  routes: [],
  monitoring: { output: "headphones", mix: null },
});
const snapshot = (): MixerSnapshot => ({
  config: configuration(),
  connected: true,
  active: false,
  sources: [
    { name: "mic", description: "USB Mic", owned: false, monitor: false },
  ],
  sinks: [
    {
      name: "headphones",
      description: "Headphones",
      owned: false,
      monitor: false,
    },
    { name: "speakers", description: "Speakers", owned: false, monitor: false },
  ],
  applications: [{ id: "org.browser", name: "Browser", detected: false }],
  apps: [],
  warnings: [],
});
afterEach(() => {
  cleanup();
  importFormDrafts({});
});
function Fixture() {
  const [config, setConfig] = useState(configuration);
  return (
    <>
      <MixerPatchbay
        config={config}
        snapshot={snapshot()}
        onChange={setConfig}
        onSetup={() => {}}
      />
      <output data-testid="config">{JSON.stringify(config)}</output>
    </>
  );
}
const current = () =>
  JSON.parse(screen.getByTestId("config").textContent!) as MixerConfig;
const connect = (from: string, to: string) => {
  fireEvent.click(screen.getByRole("button", { name: `Connect from ${from}` }));
  fireEvent.click(screen.getByRole("button", { name: `Connect to ${to}` }));
};

it("connects a microphone through a channel/mix to the listening device and removes only the selected cable", () => {
  render(<Fixture />);
  connect("USB Mic", "Voice");
  connect("Voice", "Personal");
  connect("Personal", "Headphones");
  expect(current().channels[0].sources).toEqual(["mic"]);
  expect(current().routes).toEqual([
    { channel: "voice", mix: "monitor", volume: 100, muted: false },
  ]);
  expect(current().monitoring).toEqual({
    output: "headphones",
    mix: null,
  });
  expect(current().mixes[0].outputs).toEqual(["headphones"]);
  fireEvent.click(
    screen.getByRole("button", { name: "Voice to Personal: 100%" }),
  );
  fireEvent.click(screen.getByRole("button", { name: "Disconnect" }));
  expect(current().routes).toEqual([]);
  expect(current().channels[0].sources).toEqual(["mic"]);
  expect(
    screen.getByText("Voice has no outgoing audio connection."),
  ).toBeTruthy();
});

it("fans an installed silent app and microphone out to multiple channels", () => {
  render(<Fixture />);
  fireEvent.change(screen.getByRole("searchbox"), {
    target: { value: "Browser" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Browser" }));
  fireEvent.click(screen.getByRole("button", { name: "Connect to Voice" }));
  expect(current().channels[0].apps).toEqual([
    { field: "desktop_application", value: "org.browser" },
  ]);
  connect("Browser", "Other");
  expect(current().channels[0].apps).toHaveLength(1);
  expect(current().channels[1].apps).toHaveLength(1);
  connect("USB Mic", "Voice");
  connect("USB Mic", "Other");
  expect(current().channels.map((c) => c.sources)).toEqual([["mic"], ["mic"]]);
  connect("All other audio", "Voice");
  connect("All other audio", "Other");
  expect(current().channels.map((c) => c.capture_unassigned)).toEqual([
    true,
    true,
  ]);
});

it("supports dragging ports and cancelling an unfinished connection with Escape", () => {
  render(<Fixture />);
  const data = new Map<string, string>();
  const transfer = {
    effectAllowed: "",
    dropEffect: "",
    setData: (k: string, v: string) => data.set(k, v),
    getData: (k: string) => data.get(k) || "",
  };
  fireEvent.dragStart(
    screen.getByRole("button", { name: "Connect from USB Mic" }),
    { dataTransfer: transfer },
  );
  const target = screen.getByRole("button", { name: "Connect to Voice" });
  fireEvent.dragOver(target, { dataTransfer: transfer });
  expect(transfer.dropEffect).toBe("link");
  fireEvent.drop(target, { dataTransfer: transfer });
  expect(current().channels[0].sources).toEqual(["mic"]);
  fireEvent.click(screen.getByRole("button", { name: "Connect from Voice" }));
  fireEvent.keyDown(screen.getByLabelText("Audio connection canvas"), {
    key: "Escape",
  });
  expect(
    (
      screen.getByRole("button", {
        name: "Connect to Personal",
      }) as HTMLButtonElement
    ).disabled,
  ).toBe(true);
});

it("diagnoses missing listening mix, mute/zero gains and stranded playback without pretending recording is audible", () => {
  const config = configuration();
  config.enabled = true;
  config.routes = [
    { channel: "voice", mix: "monitor", volume: 100, muted: false },
  ];
  expect(routingProblems(config, snapshot()).join(" ")).toContain(
    "no mix is connected",
  );
  config.monitoring!.mix = "monitor";
  expect(
    routingProblems(config, snapshot()).some((s) => s.startsWith("Voice")),
  ).toBe(false);
  config.mixes[0].muted = true;
  expect(routingProblems(config, snapshot()).join(" ")).toContain(
    "Voice has no audible path",
  );
  const live = snapshot();
  live.sinks.push({
    name: "deckforge.scope.channel.removed",
    description: "Removed",
    owned: true,
    monitor: false,
  });
  live.apps = [
    { index: 1, name: "Voice app", sink: "deckforge.scope.channel.removed" },
  ];
  expect(routingProblems(config, live).join(" ")).toContain(
    "Voice app is attached to a removed channel",
  );
  expect(
    connectPorts(
      config,
      { kind: "output", id: "headphones" },
      { kind: "source", id: "mic" },
      live,
    ),
  ).toBe(config);
});

it("keeps graph edits across mixer views and sends them only when Apply routes is pressed", async () => {
  const run = vi.fn(async (_payload: Record<string, unknown>) => true);
  const live = snapshot();
  render(<AudioMixer snapshot={live} run={run} error="" />);
  fireEvent.click(screen.getByRole("button", { name: "Advanced routing" }));
  connect("USB Mic", "Voice");
  connect("Voice", "Personal");
  connect("Personal", "Speakers");
  expect(
    run.mock.calls.filter(
      ([p]) => (p as { command: string }).command === "mixer_config",
    ),
  ).toHaveLength(0);
  fireEvent.click(screen.getByRole("button", { name: "Mixer" }));
  fireEvent.click(screen.getByRole("button", { name: "Advanced routing" }));
  expect(
    screen.getByRole("button", {
      name: "Personal to Speakers: Fixed output · 100%",
    }),
  ).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Apply routes" }));
  await waitFor(() =>
    expect(
      run.mock.calls.some(
        ([p]) => (p as { command: string }).command === "mixer_config",
      ),
    ).toBe(true),
  );
  expect(exportFormDrafts()["audio-mixer"].config.routes).toHaveLength(1);
});

it("connects individual channels and two mixes to one device without replacing another feed", () => {
  render(<Fixture />);
  fireEvent.click(screen.getByRole("button", { name: "Add mix" }));
  connect("Personal", "Headphones");
  connect("Mix 2", "Headphones");
  expect(current().mixes.map((m) => m.outputs)).toEqual([
    ["headphones"],
    ["headphones"],
  ]);
  fireEvent.click(
    screen.getByRole("button", {
      name: "Show individual channels for Personal",
    }),
  );
  fireEvent.click(
    screen.getByRole("button", {
      name: "Show individual channels for Speakers",
    }),
  );
  connect("Personal Left", "Speakers Right");
  expect(current().connections?.[0]).toMatchObject({
    from: { kind: "mix", id: "monitor" },
    to: { kind: "output", name: "speakers" },
    source_channel: "front-left",
    target_channel: "front-right",
  });
});

it("removes one side of a stereo cable without removing the other channel", () => {
  render(<Fixture />);
  connect("Voice", "Personal");
  fireEvent.click(
    screen.getByRole("button", { name: "Show individual channels for Voice" }),
  );
  fireEvent.click(
    screen.getByRole("button", {
      name: "Show individual channels for Personal",
    }),
  );
  fireEvent.click(
    screen.getByRole("button", {
      name: "Voice to Personal: 100% · Left → Left",
    }),
  );
  fireEvent.click(screen.getByRole("button", { name: "Disconnect" }));
  expect(current().routes).toEqual([]);
  expect(current().connections).toHaveLength(1);
  expect(current().connections?.[0]).toMatchObject({
    source_channel: "front-right",
    target_channel: "front-right",
  });
});

it("moves nodes freely with pointer/keyboard gestures, retains the layout, and leaves audio config untouched", () => {
  const { unmount } = render(<Fixture />);
  const before = current();
  const header = screen.getByRole("button", { name: "Move Voice" });
  const node = header.closest("article")!;
  const left = parseFloat(node.style.left),
    top = parseFloat(node.style.top);
  fireEvent(
    header,
    new MouseEvent("pointerdown", {
      bubbles: true,
      button: 0,
      clientX: 100,
      clientY: 100,
    }),
  );
  fireEvent(
    header,
    new MouseEvent("pointermove", {
      bubbles: true,
      clientX: 180,
      clientY: 160,
    }),
  );
  fireEvent(
    header,
    new MouseEvent("pointerup", { bubbles: true, clientX: 180, clientY: 160 }),
  );
  expect(parseFloat(node.style.left)).toBe(left + 80);
  expect(parseFloat(node.style.top)).toBe(top + 60);
  fireEvent.keyDown(header, { key: "ArrowLeft", shiftKey: true });
  expect(parseFloat(node.style.left)).toBe(left + 30);
  expect(current()).toEqual(before);
  unmount();
  render(<Fixture />);
  expect(
    parseFloat(
      screen.getByRole("button", { name: "Move Voice" }).closest("article")!
        .style.left,
    ),
  ).toBe(left + 30);
  fireEvent.click(screen.getByRole("button", { name: "Expand canvas" }));
  expect(screen.getByRole("button", { name: "Return to mixer" })).toBeTruthy();
});

it("keeps a stereo leg unique and edits one connection gain without changing another route", () => {
  render(<Fixture />);
  connect("Voice", "Personal");
  fireEvent.click(
    screen.getByRole("button", { name: "Voice to Personal: 100%" }),
  );
  fireEvent.change(screen.getByRole("slider", { name: "Connection level" }), {
    target: { value: "37" },
  });
  expect(current().routes[0].volume).toBe(37);
  fireEvent.click(
    screen.getByRole("button", { name: "Show individual channels for Voice" }),
  );
  fireEvent.click(
    screen.getByRole("button", {
      name: "Show individual channels for Personal",
    }),
  );
  connect("Voice Left", "Personal Left");
  expect(current().connections ?? []).toEqual([]);
  fireEvent.click(
    screen.getByRole("button", {
      name: "Voice to Personal: 37% · Left → Left",
    }),
  );
  fireEvent.click(screen.getByRole("button", { name: "Disconnect" }));
  expect(current().connections?.[0]).toMatchObject({
    source_channel: "front-right",
    target_channel: "front-right",
    volume: 37,
  });
});
