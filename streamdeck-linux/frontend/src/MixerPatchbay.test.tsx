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
    mix: "monitor",
  });
  expect(current().mixes[0].outputs).toEqual([]);
  fireEvent.click(
    screen.getByRole("button", { name: "Voice to Personal: 100%" }),
  );
  fireEvent.click(screen.getByRole("button", { name: "Disconnect" }));
  expect(current().routes).toEqual([]);
  expect(current().channels[0].sources).toEqual(["mic"]);
  expect(screen.getByText("Voice has no connection to a mix.")).toBeTruthy();
});

it("finds an installed silent app, then transfers it to exactly one channel", () => {
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
  expect(current().channels[0].apps).toEqual([]);
  expect(current().channels[1].apps).toHaveLength(1);
  connect("USB Mic", "Voice");
  connect("USB Mic", "Other");
  expect(current().channels.map((c) => c.sources)).toEqual([[], ["mic"]]);
  connect("All other audio", "Voice");
  connect("All other audio", "Other");
  expect(current().channels.map((c) => c.capture_unassigned)).toEqual([
    false,
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
      { kind: "channel", id: "voice" },
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
