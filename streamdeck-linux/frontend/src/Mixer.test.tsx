// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  render,
  screen,
  cleanup,
  waitFor,
  fireEvent,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  AudioMixer,
  PeakMeter,
  sameMixerValue,
  editRoute,
  removeChannel,
  removeMix,
  appChoices,
  type MixerSnapshot,
  type MixerConfig,
} from "./Mixer";
import { patchMixerControl } from "./MixerLive";
import { importFormDrafts } from "./transient";
const config: MixerConfig = {
  enabled: false,
  channels: [
    {
      id: "music",
      name: "Music",
      apps: [{ field: "application_id", value: "org.example.Player" }],
      sources: ["missing.mic"],
    },
  ],
  mixes: [
    {
      id: "monitor",
      name: "Monitor",
      outputs: ["missing.speakers"],
      volume: 100,
      muted: false,
    },
    { id: "stream", name: "Stream", outputs: [], volume: 90, muted: false },
  ],
  routes: [{ channel: "music", mix: "monitor", volume: 45, muted: false }],
};
const inventory = (): MixerSnapshot => ({
  config: structuredClone(config),
  connected: true,
  active: false,
  sinks: [
    {
      name: "physical.output",
      description: "Desktop speakers",
      owned: false,
      monitor: false,
    },
    {
      name: "deckforge.owned",
      description: "Owned mixer output",
      owned: true,
      monitor: false,
    },
  ],
  sources: [
    {
      name: "physical.mic",
      description: "USB microphone",
      owned: false,
      monitor: false,
    },
    {
      name: "deckforge.source",
      description: "Owned mixer source",
      owned: true,
      monitor: false,
    },
    {
      name: "system.monitor",
      description: "System monitor",
      owned: false,
      monitor: true,
    },
  ],
  apps: [
    {
      index: 1,
      name: "Browser",
      application_id: "org.example.Browser",
      process_binary: "browser",
      sink: "physical.output",
    },
  ],
  warnings: [],
  error: null,
});
beforeEach(() => importFormDrafts({}));
afterEach(() => {
  cleanup();
  delete window.deckforge;
});
describe("mixer graph drafts", () => {
  it("does not mistake Rust object-key reordering for an external edit", () => {
    const reordered = {
      routes: config.routes.map((r) => ({
        muted: r.muted,
        volume: r.volume,
        mix: r.mix,
        channel: r.channel,
      })),
      mixes: config.mixes,
      channels: config.channels,
      enabled: config.enabled,
    };
    expect(sameMixerValue(config, reordered)).toBe(true);
    expect(sameMixerValue(config, { ...reordered, enabled: true })).toBe(false);
  });
  it("changes one route without overwriting another mix or dropping input identities", () => {
    const next = editRoute(config, "music", "stream", {
      volume: 23,
      muted: true,
    });
    expect(next.routes).toEqual([
      ...config.routes,
      { channel: "music", mix: "stream", volume: 23, muted: true },
    ]);
    expect(next.channels).toEqual(config.channels);
    expect(config.routes).toHaveLength(1);
    expect(editRoute(next, "music", "monitor", null).routes).toEqual([
      next.routes[1],
    ]);
  });
  it("removes only edges owned by a removed input or mix", () => {
    expect(removeChannel(config, "music").routes).toEqual([]);
    expect(removeChannel(config, "music").mixes).toEqual(config.mixes);
    expect(removeMix(config, "monitor").routes).toEqual([]);
    expect(removeMix(config, "monitor").channels).toEqual(config.channels);
  });
  it("deduplicates stable application identities rather than storing transient stream indexes", () => {
    const app = {
      index: 1,
      name: "Browser",
      application_id: "browser.app",
      process_binary: "browser",
      sink: "output",
    };
    expect(
      appChoices([app, { ...app, index: 2 }]).map((v) => v.selector),
    ).toEqual([
      { field: "application_id", value: "browser.app" },
      { field: "process_binary", value: "browser" },
    ]);
  });
});
describe("mixer panel", () => {
  it("excludes other-session mixer endpoints but retains saved unavailable identities", async () => {
    const user = userEvent.setup();
    const snapshot = inventory();
    snapshot.sinks.push({
      name: "deckforge.parked",
      description: "Other session output",
      owned: false,
      monitor: false,
    });
    snapshot.sources.push({
      name: "deckforge.parked.source",
      description: "Other session source",
      owned: false,
      monitor: false,
    });
    snapshot.config.mixes[0].outputs.push("deckforge.parked");
    snapshot.config.channels[0].sources.push("deckforge.parked.source");
    render(
      <AudioMixer
        snapshot={snapshot}
        run={vi.fn(async (_payload: Record<string, any>) => true)}
      />,
    );
    await user.click(
      screen.getByText("Monitor outputs", { selector: "summary" }),
    );
    await user.click(screen.getByText("Edit inputs", { selector: "summary" }));
    expect(
      screen.queryByRole("checkbox", { name: "Other session output" }),
    ).toBeNull();
    expect(
      screen.queryByRole("checkbox", { name: "Other session source" }),
    ).toBeNull();
    expect(
      screen.getByRole("button", {
        name: "Remove unavailable deckforge.parked",
      }),
    ).toBeTruthy();
    expect(
      screen.getByRole("button", {
        name: "Remove unavailable deckforge.parked.source",
      }),
    ).toBeTruthy();
  });
  it("opens read-only inventory and applies routing only explicitly, preserving missing identities", async () => {
    const user = userEvent.setup();
    const run = vi.fn(async (_payload: Record<string, any>) => true);
    render(<AudioMixer snapshot={inventory()} run={run} />);
    await waitFor(() =>
      expect(run).toHaveBeenCalledWith({ command: "mixer_refresh" }),
    );
    expect(run).toHaveBeenCalledTimes(1);
    await user.click(
      screen.getByText("Monitor outputs", { selector: "summary" }),
    );
    expect(screen.getByText("missing.speakers")).toBeTruthy();
    expect(
      screen.queryByRole("checkbox", { name: "Owned mixer output" }),
    ).toBeNull();
    await user.click(
      screen.getByRole("checkbox", { name: "Enable audio routing" }),
    );
    await user.click(
      screen.getByRole("checkbox", { name: "Route Music to Stream" }),
    );
    fireEvent.change(
      screen.getByRole("slider", { name: "Music → Stream volume" }),
      { target: { value: "27" } },
    );
    expect(run).toHaveBeenCalledTimes(1);
    await user.click(screen.getByRole("button", { name: "Apply routes" }));
    await waitFor(() =>
      expect(run).toHaveBeenCalledWith({
        command: "mixer_config",
        expected: config,
        settings: {
          ...config,
          enabled: true,
          routes: [
            ...config.routes,
            { channel: "music", mix: "stream", volume: 27, muted: false },
          ],
        },
      }),
    );
  });
  it("keeps a dirty draft on external updates and requires reloading before apply", async () => {
    const user = userEvent.setup();
    const run = vi.fn(async (_payload: Record<string, any>) => true);
    const first = inventory();
    const { rerender } = render(<AudioMixer snapshot={first} run={run} />);
    await user.click(
      screen.getByRole("checkbox", { name: "Enable audio routing" }),
    );
    const newer = inventory();
    newer.config.mixes[0].volume = 61;
    newer.config.channels[0].sources = ["another.mic"];
    rerender(<AudioMixer snapshot={newer} run={run} />);
    expect(screen.getByText(/Routing settings changed outside/)).toBeTruthy();
    expect(
      (
        screen.getByRole("button", {
          name: "Apply routes",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(true);
    expect(
      (
        screen.getByRole("checkbox", {
          name: "Enable audio routing",
        }) as HTMLInputElement
      ).checked,
    ).toBe(true);
    await user.click(
      screen.getByRole("button", { name: "Reload current settings" }),
    );
    expect(
      (
        screen.getByRole("checkbox", {
          name: "Enable audio routing",
        }) as HTMLInputElement
      ).checked,
    ).toBe(false);
    expect(
      (
        screen.getByRole("slider", {
          name: "Monitor master volume",
        }) as HTMLInputElement
      ).value,
    ).toBe("61");
  });
  it("restores unapplied graph edits when the panel is recreated", async () => {
    const user = userEvent.setup();
    const run = vi.fn(async (_payload: Record<string, any>) => true);
    const view = render(<AudioMixer snapshot={inventory()} run={run} />);
    await user.click(
      screen.getByRole("checkbox", { name: "Enable audio routing" }),
    );
    fireEvent.change(
      screen.getByRole("slider", { name: "Monitor master volume" }),
      { target: { value: "36" } },
    );
    view.unmount();
    render(<AudioMixer snapshot={inventory()} run={run} />);
    expect(
      (
        screen.getByRole("slider", {
          name: "Monitor master volume",
        }) as HTMLInputElement
      ).value,
    ).toBe("36");
    expect(
      (
        screen.getByRole("checkbox", {
          name: "Enable audio routing",
        }) as HTMLInputElement
      ).checked,
    ).toBe(true);
    expect(
      run.mock.calls.filter(([payload]) => payload.command === "mixer_config"),
    ).toHaveLength(0);
  });
  it("adds exact app selectors and physical sources without exposing owned audio devices", async () => {
    const user = userEvent.setup();
    const run = vi.fn(async (_payload: Record<string, any>) => true);
    render(<AudioMixer snapshot={inventory()} run={run} />);
    await user.click(screen.getByText("Edit inputs", { selector: "summary" }));
    expect(screen.getByText("missing.mic")).toBeTruthy();
    expect(
      screen.queryByRole("checkbox", { name: "Owned mixer source" }),
    ).toBeNull();
    expect(
      screen.queryByRole("checkbox", { name: "System monitor" }),
    ).toBeNull();
    const selector = { field: "application_id", value: "org.example.Browser" };
    await user.selectOptions(
      screen.getByRole("combobox", { name: "Application for Music" }),
      JSON.stringify(selector),
    );
    await user.click(screen.getByRole("button", { name: "Add application" }));
    await user.click(screen.getByRole("checkbox", { name: "USB microphone" }));
    await user.click(screen.getByRole("button", { name: "Apply routes" }));
    await waitFor(() => {
      const call = run.mock.calls.find(
        ([payload]) => payload.command === "mixer_config",
      )?.[0];
      expect(call?.settings.channels[0].apps).toEqual([
        ...config.channels[0].apps,
        selector,
      ]);
      expect(call?.settings.channels[0].sources).toEqual([
        "missing.mic",
        "physical.mic",
      ]);
    });
  });
});

it("renders real peak readings and an explicit unavailable state instead of fake silence", () => {
  const view = render(
    <PeakMeter label="Voice" level={{ available: true, peak: 0.5 }} />,
  );
  const meter = screen.getByRole("meter", { name: "Voice peak level" });
  expect(meter.getAttribute("aria-valuetext")).toBe("-6.0 dBFS");
  view.rerender(
    <PeakMeter label="Voice" level={{ available: true, peak: 0 }} />,
  );
  expect(screen.getByRole("meter").getAttribute("aria-valuetext")).toBe(
    "−∞ dBFS",
  );
  view.rerender(
    <PeakMeter label="Voice" level={{ available: true, peak: 1 }} />,
  );
  expect(screen.getByRole("meter").className).toContain("clipping");
  view.rerender(
    <PeakMeter label="Voice" level={{ available: false, peak: 0 }} />,
  );
  expect(screen.queryByRole("meter")).toBeNull();
  expect(
    screen.getByRole("img", { name: "Voice: meter unavailable" }),
  ).toBeTruthy();
});

it("sends saved inactive controls immediately, serializes revisions and preserves pending route drafts", async () => {
  const user = userEvent.setup(),
    run = vi.fn(async (_payload: Record<string, any>) => true);
  let live = { ...inventory(), revision: 7 };
  const calls: any[] = [];
  window.deckforge = {
    onSnapshot: () => () => {},
    request: vi.fn(async (method, payload: any) => {
      expect(method).toBe("mixer_control");
      expect(payload.revision).toBe(live.revision);
      calls.push(payload);
      live = {
        ...live,
        config: patchMixerControl(live.config, payload.target, payload),
        revision:
          live.revision +
          (["channel", "preferences"].includes(payload.target.kind) ? 1 : 0),
      };
      return structuredClone(live) as any;
    }),
  };
  render(<AudioMixer snapshot={live} run={run} />);
  await user.click(
    screen.getByRole("checkbox", { name: "Route Music to Stream" }),
  );
  fireEvent.change(
    screen.getByRole("slider", { name: "Music → Stream volume" }),
    { target: { value: "22" } },
  );
  expect(calls).toHaveLength(0);
  await user.click(
    screen.getByRole("checkbox", { name: "Link Music mix levels" }),
  );
  fireEvent.change(
    screen.getByRole("slider", { name: "Music → Monitor volume" }),
    { target: { value: "61" } },
  );
  fireEvent.change(
    screen.getByRole("spinbutton", { name: "Default fade (ms)" }),
    { target: { value: "250" } },
  );
  await waitFor(() => expect(calls).toHaveLength(3));
  expect(calls.map((c) => c.revision)).toEqual([7, 8, 8]);
  expect(
    (
      screen.getByRole("slider", {
        name: "Music → Stream volume",
      }) as HTMLInputElement
    ).value,
  ).toBe("22");
  await user.click(screen.getByRole("button", { name: "Apply routes" }));
  const apply = run.mock.calls.find(
    ([arg]: any) => arg.command === "mixer_config",
  )?.[0] as any;
  expect(apply.expected).toEqual(live.config);
  expect(apply.settings.routes).toContainEqual({
    channel: "music",
    mix: "stream",
    volume: 22,
    muted: false,
  });
  expect(apply.settings.enabled).toBe(false);
});
it("keeps only latest consecutive fader intent and blocks Apply until accepted", async () => {
  const user = userEvent.setup(),
    run = vi.fn(async (_payload: Record<string, any>) => true);
  let live = { ...inventory(), revision: 1 };
  const finish: (() => void)[] = [],
    calls: any[] = [];
  window.deckforge = {
    onSnapshot: () => () => {},
    request: vi.fn(async (_method, payload: any) => {
      calls.push(payload);
      await new Promise<void>((resolve) => finish.push(resolve));
      live = {
        ...live,
        config: patchMixerControl(live.config, payload.target, payload),
      };
      return structuredClone(live) as any;
    }),
  };
  render(<AudioMixer snapshot={live} run={run} />);
  await user.click(
    screen.getByRole("checkbox", { name: "Route Music to Stream" }),
  );
  const slider = screen.getByRole("slider", { name: "Music → Monitor volume" });
  for (const value of ["50", "60", "70"])
    fireEvent.change(slider, { target: { value } });
  expect(calls).toHaveLength(1);
  expect((slider as HTMLInputElement).value).toBe("70");
  expect(
    (screen.getByRole("button", { name: "Apply routes" }) as HTMLButtonElement)
      .disabled,
  ).toBe(true);
  finish[0]();
  await waitFor(() => expect(calls).toHaveLength(2));
  expect(calls[1].volume).toBe(70);
  finish[1]();
  await waitFor(() =>
    expect(
      (
        screen.getByRole("button", {
          name: "Apply routes",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(false),
  );
  expect((slider as HTMLInputElement).value).toBe("70");
});
it("edits output gain separately and surfaces rejection without losing routing edits", async () => {
  const user = userEvent.setup(),
    run = vi.fn(async (_payload: Record<string, any>) => true);
  let live = { ...inventory(), revision: 2 };
  const rpc = vi.fn(async (_method, payload: any) => {
    if (payload.muted) throw new Error("Graph changed; retry");
    live = {
      ...live,
      config: patchMixerControl(live.config, payload.target, payload),
    };
    return structuredClone(live) as any;
  });
  window.deckforge = { request: rpc, onSnapshot: () => () => {} };
  render(<AudioMixer snapshot={live} run={run} />);
  await user.click(
    screen.getByRole("checkbox", { name: "Route Music to Stream" }),
  );
  await user.click(
    screen.getByText("Monitor outputs", { selector: "summary" }),
  );
  fireEvent.change(
    screen.getByRole("slider", {
      name: "Monitor output · missing.speakers volume",
    }),
    { target: { value: "33" } },
  );
  await waitFor(() =>
    expect(
      live.config.mixes[0].output_controls?.["missing.speakers"].volume,
    ).toBe(33),
  );
  expect(live.config.mixes[0].volume).toBe(100);
  await user.click(
    screen.getByRole("button", {
      name: "Mute Monitor output · missing.speakers",
    }),
  );
  await waitFor(() =>
    expect(screen.getByRole("alert").textContent).toContain("Graph changed"),
  );
  expect(
    (
      screen.getByRole("checkbox", {
        name: "Route Music to Stream",
      }) as HTMLInputElement
    ).checked,
  ).toBe(true);
});

it("does not bake optimistic controls into a structural edit made while an RPC is pending", async () => {
  const user = userEvent.setup(),
    run = vi.fn(async (_payload: Record<string, any>) => true);
  let live = { ...inventory(), revision: 1 };
  let finish!: () => void;
  window.deckforge = {
    onSnapshot: () => () => {},
    request: vi.fn(async (_method, payload: any) => {
      await new Promise<void>((resolve) => (finish = resolve));
      live = {
        ...live,
        config: patchMixerControl(live.config, payload.target, payload),
      };
      return structuredClone(live) as any;
    }),
  };
  render(<AudioMixer snapshot={live} run={run} />);
  fireEvent.change(
    screen.getByRole("slider", { name: "Music → Monitor volume" }),
    { target: { value: "60" } },
  );
  fireEvent.change(screen.getByRole("textbox", { name: "Monitor mix name" }), {
    target: { value: "Pending name" },
  });
  finish();
  await waitFor(() =>
    expect(
      (
        screen.getByRole("button", {
          name: "Apply routes",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(false),
  );
  expect(
    (
      screen.getByRole("slider", {
        name: "Music → Pending name volume",
      }) as HTMLInputElement
    ).value,
  ).toBe("60");
  await user.click(screen.getByRole("button", { name: "Apply routes" }));
  const apply = run.mock.calls.find(([p]) => p.command === "mixer_config")?.[0];
  expect(apply?.expected).toEqual(live.config);
  expect(apply?.settings.routes[0].volume).toBe(60);
  expect(apply?.settings.mixes[0].name).toBe("Pending name");
});
