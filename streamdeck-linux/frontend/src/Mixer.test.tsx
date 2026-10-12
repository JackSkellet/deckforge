// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  render,
  screen,
  cleanup,
  waitFor,
  fireEvent,
  within,
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
  sameApplication,
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
function setupView() {
  fireEvent.click(screen.getByRole("button", { name: "Routing setup" }));
}
function dailyView() {
  fireEvent.click(screen.getByRole("button", { name: "Mixer" }));
}
function gain(name: string) {
  dailyView();
  if (name.includes(" master ") || name.includes(" output · ")) {
    const mixName = name.split(/ master | output · /)[0];
    const toggle = screen.getByRole("button", {
      name: `${mixName} mix settings`,
    });
    if (toggle.getAttribute("aria-expanded") !== "true")
      fireEvent.click(toggle);
  }
  return screen.getByRole("slider", { name });
}
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
    setupView();
    setupView();
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
    setupView();
    expect(screen.getByText("missing.speakers")).toBeTruthy();
    expect(
      screen.queryByRole("checkbox", { name: "Owned mixer output" }),
    ).toBeNull();
    setupView();
    await user.click(
      screen.getByRole("checkbox", { name: "Enable audio routing" }),
    );
    setupView();
    await user.click(
      screen.getByRole("checkbox", { name: "Route Music to Stream" }),
    );
    fireEvent.change(gain("Music → Stream volume"), {
      target: { value: "27" },
    });
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
    setupView();
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
    setupView();
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
    setupView();
    expect(
      (
        screen.getByRole("checkbox", {
          name: "Enable audio routing",
        }) as HTMLInputElement
      ).checked,
    ).toBe(false);
    expect((gain("Monitor master volume") as HTMLInputElement).value).toBe(
      "61",
    );
  });
  it("restores unapplied graph edits when the panel is recreated", async () => {
    const user = userEvent.setup();
    const run = vi.fn(async (_payload: Record<string, any>) => true);
    const view = render(<AudioMixer snapshot={inventory()} run={run} />);
    setupView();
    await user.click(
      screen.getByRole("checkbox", { name: "Enable audio routing" }),
    );
    fireEvent.change(gain("Monitor master volume"), {
      target: { value: "36" },
    });
    view.unmount();
    render(<AudioMixer snapshot={inventory()} run={run} />);
    expect((gain("Monitor master volume") as HTMLInputElement).value).toBe(
      "36",
    );
    setupView();
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
    setupView();
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
  setupView();
  await user.click(
    screen.getByRole("checkbox", { name: "Route Music to Stream" }),
  );
  fireEvent.change(gain("Music → Stream volume"), { target: { value: "22" } });
  expect(calls).toHaveLength(0);
  dailyView();
  await user.click(
    screen.getByRole("checkbox", { name: "Link Music mix levels" }),
  );
  fireEvent.change(gain("Music → Monitor volume"), { target: { value: "61" } });
  setupView();
  fireEvent.change(
    screen.getByRole("spinbutton", { name: "Default fade (ms)" }),
    { target: { value: "250" } },
  );
  await waitFor(() => expect(calls).toHaveLength(3));
  expect(calls.map((c) => c.revision)).toEqual([7, 8, 8]);
  expect((gain("Music → Stream volume") as HTMLInputElement).value).toBe("22");
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
  setupView();
  await user.click(
    screen.getByRole("checkbox", { name: "Route Music to Stream" }),
  );
  const slider = gain("Music → Monitor volume");
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
  setupView();
  await user.click(
    screen.getByRole("checkbox", { name: "Route Music to Stream" }),
  );
  dailyView();
  fireEvent.change(gain("Monitor output · missing.speakers volume"), {
    target: { value: "33" },
  });
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
  setupView();
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
  fireEvent.change(gain("Music → Monitor volume"), { target: { value: "60" } });
  setupView();
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
  expect((gain("Music → Pending name volume") as HTMLInputElement).value).toBe(
    "60",
  );
  await user.click(screen.getByRole("button", { name: "Apply routes" }));
  const apply = run.mock.calls.find(([p]) => p.command === "mixer_config")?.[0];
  expect(apply?.expected).toEqual(live.config);
  expect(apply?.settings.routes[0].volume).toBe(60);
  expect(apply?.settings.mixes[0].name).toBe("Pending name");
});

it("matches observed identities to installed apps only when unambiguous", () => {
  const apps = [
    {
      index: 1,
      name: "Chat",
      application_id: "chat",
      process_binary: "electron",
      desktop_application: "chat.desktop",
      sink: "speakers",
    },
    {
      index: 2,
      name: "Music",
      application_id: "music",
      process_binary: "electron",
      desktop_application: "music.desktop",
      sink: "speakers",
    },
  ];
  expect(
    sameApplication(
      { field: "desktop_application", value: "chat.desktop" },
      { field: "application_id", value: "chat" },
      apps,
    ),
  ).toBe(true);
  expect(
    sameApplication(
      { field: "desktop_application", value: "chat.desktop" },
      { field: "process_binary", value: "electron" },
      apps,
    ),
  ).toBe(false);
  expect(
    appChoices(apps).some(
      (choice) => choice.selector.field === "process_binary",
    ),
  ).toBe(false);
  expect(
    sameApplication(
      { field: "desktop_application", value: "chat.desktop" },
      { field: "application_id", value: "chat" },
      [...apps, { ...apps[0], index: 3, desktop_application: null }],
    ),
  ).toBe(false);
});
it("searches installed apps, fills stable identity, and retains pending matches without applying routing", async () => {
  const user = userEvent.setup(),
    run = vi.fn(async (_payload: Record<string, any>) => true),
    snapshot = inventory();
  snapshot.applications = [
    { id: "org.example.Chat.desktop", name: "Chat App", detected: false },
    { id: "org.example.Browser.desktop", name: "Browser", detected: true },
  ];
  snapshot.apps[0].desktop_application = "org.example.Browser.desktop";
  const view = render(<AudioMixer snapshot={snapshot} run={run} />);
  setupView();
  await user.click(screen.getByText(/Edit inputs/, { selector: "summary" }));
  const search = screen.getByRole("searchbox", {
    name: "Search installed applications for Music",
  });
  await user.type(search, "Chat");
  const chooser = screen.getByRole("listbox", {
    name: "Installed application for Music",
  });
  expect(
    screen.queryByRole("option", { name: /Browser · Detected/ }),
  ).toBeNull();
  await user.selectOptions(chooser, "org.example.Chat.desktop");
  expect(screen.getByText(/Exact identity:/).textContent).toContain(
    "org.example.Chat.desktop",
  );
  await user.click(screen.getByRole("button", { name: "Add application" }));
  expect(run).toHaveBeenCalledTimes(1);
  expect(screen.getByText("Chat App", { selector: "strong" })).toBeTruthy();
  expect(
    screen.getByText("Installed application · Pending audio"),
  ).toBeTruthy();
  expect(
    (
      screen
        .getByText("Advanced: manual identity")
        .closest("details") as HTMLDetailsElement
    ).open,
  ).toBe(false);
  view.unmount();
  render(<AudioMixer snapshot={snapshot} run={run} />);
  setupView();
  await user.click(screen.getByText(/Edit inputs/, { selector: "summary" }));
  expect(screen.getByText("Chat App", { selector: "strong" })).toBeTruthy();
  await user.click(screen.getByRole("button", { name: "Apply routes" }));
  const applied = run.mock.calls.find(
    ([payload]) => payload.command === "mixer_config",
  )?.[0];
  expect(applied?.settings.channels[0].apps).toContainEqual({
    field: "desktop_application",
    value: "org.example.Chat.desktop",
  });
  expect(applied?.settings.enabled).toBe(false);
});
it("excludes installed aliases already assigned through a playing identity in another group", async () => {
  const user = userEvent.setup(),
    snapshot = inventory();
  snapshot.applications = [
    { id: "browser.desktop", name: "Browser", detected: true },
  ];
  snapshot.apps[0].desktop_application = "browser.desktop";
  snapshot.config.channels.push({
    id: "other",
    name: "Other",
    sources: [],
    apps: [{ field: "application_id", value: "org.example.Browser" }],
  });
  render(<AudioMixer snapshot={snapshot} run={vi.fn(async () => true)} />);
  setupView();
  await user.click(
    screen.getAllByText(/Edit inputs/, { selector: "summary" })[0],
  );
  expect(
    screen.queryByRole("option", { name: /Browser · Detected/ }),
  ).toBeNull();
});

it("allows multiple groups to share the catch-all source in a routing draft", async () => {
  const user = userEvent.setup(),
    snapshot = inventory(),
    run = vi.fn(async (_payload: Record<string, any>) => true);
  snapshot.config.channels[0].capture_unassigned = true;
  snapshot.config.channels.push({
    id: "other",
    name: "Other",
    apps: [],
    sources: [],
  });
  render(<AudioMixer snapshot={snapshot} run={run} />);
  setupView();
  for (const summary of screen.getAllByText(/Edit inputs/, {
    selector: "summary",
  }))
    await user.click(summary);
  await user.click(
    screen.getByRole("checkbox", { name: "Default for other apps · Other" }),
  );
  expect(
    (
      screen.getByRole("checkbox", {
        name: "Default for other apps · Music",
      }) as HTMLInputElement
    ).checked,
  ).toBe(true);
  expect(
    (
      screen.getByRole("checkbox", {
        name: "Default for other apps · Other",
      }) as HTMLInputElement
    ).checked,
  ).toBe(true);
  expect(screen.getAllByText("Default", { selector: "small" })).toHaveLength(2);
  expect(run).toHaveBeenCalledTimes(1);
  await user.click(screen.getByRole("button", { name: "Apply routes" }));
  const applied = run.mock.calls.find(
    ([p]) => p.command === "mixer_config",
  )?.[0];
  expect(
    applied?.settings.channels.map((c: any) => !!c.capture_unassigned),
  ).toEqual([true, true]);
  expect(applied?.settings.channels[0].apps).toEqual(
    snapshot.config.channels[0].apps,
  );
  expect(applied?.settings.enabled).toBe(false);
});

it("keeps all mix columns compact, separates setup forms and restores view/settings after reopening", async () => {
  const user = userEvent.setup(),
    snapshot = inventory(),
    run = vi.fn(async (_p: Record<string, any>) => true);
  snapshot.config.routes.push({
    channel: "music",
    mix: "stream",
    volume: 70,
    muted: false,
  });
  const view = render(<AudioMixer snapshot={snapshot} run={run} />);
  expect(
    screen.getByRole("slider", { name: "Music → Monitor volume" }),
  ).toBeTruthy();
  expect(
    screen.getByRole("slider", { name: "Music → Stream volume" }),
  ).toBeTruthy();
  expect(
    screen.queryByRole("textbox", { name: "Monitor mix name" }),
  ).toBeNull();
  expect(
    screen.queryByRole("checkbox", { name: "Enable audio routing" }),
  ).toBeNull();
  await user.click(screen.getByRole("button", { name: "Stream mix settings" }));
  expect(
    screen.getByRole("slider", { name: "Stream master volume" }),
  ).toBeTruthy();
  view.unmount();
  const reopened = render(<AudioMixer snapshot={snapshot} run={run} />);
  expect(
    screen
      .getByRole("button", { name: "Stream mix settings" })
      .getAttribute("aria-expanded"),
  ).toBe("true");
  await user.click(screen.getByRole("button", { name: "Routing setup" }));
  expect(screen.queryByRole("slider")).toBeNull();
  fireEvent.change(screen.getByRole("textbox", { name: "Monitor mix name" }), {
    target: { value: "Headphones" },
  });
  reopened.unmount();
  render(<AudioMixer snapshot={snapshot} run={run} />);
  expect(
    screen
      .getByRole("button", { name: "Routing setup" })
      .getAttribute("aria-pressed"),
  ).toBe("true");
  expect(
    (
      screen.getByRole("textbox", {
        name: "Headphones mix name",
      }) as HTMLInputElement
    ).value,
  ).toBe("Headphones");
  await user.click(screen.getByRole("button", { name: "Mixer" }));
  expect(
    (screen.getByRole("button", { name: "Apply routes" }) as HTMLButtonElement)
      .disabled,
  ).toBe(false);
  expect(
    run.mock.calls.filter(([p]) => p.command === "mixer_config"),
  ).toHaveLength(0);
});
it("creates a named app-bound channel from search, adds a draft route and makes an optional catch-all", async () => {
  const user = userEvent.setup(),
    snapshot = inventory(),
    run = vi.fn(async (_p: Record<string, any>) => true);
  snapshot.applications = [
    { id: "chat.desktop", name: "Chat App", detected: false },
  ];
  render(<AudioMixer snapshot={snapshot} run={run} />);
  await user.click(screen.getByRole("button", { name: "＋ Create channel" }));
  await user.type(
    screen.getByRole("searchbox", { name: "Search apps and inputs" }),
    "Chat",
  );
  await user.click(
    screen.getByRole("button", { name: "Chat App Not playing" }),
  );
  expect(screen.getByText("Chat App", { selector: "strong" })).toBeTruthy();
  await user.click(
    screen.getByRole("button", { name: "Add route Chat App to Monitor" }),
  );
  expect(
    screen.getByRole("slider", { name: "Chat App → Monitor volume" }),
  ).toBeTruthy();
  await user.click(screen.getByRole("button", { name: "＋ Create channel" }));
  await user.click(
    screen.getByRole("button", { name: /All audio System sounds/ }),
  );
  expect(screen.getByText("Default", { selector: "small" })).toBeTruthy();
  expect(run).toHaveBeenCalledTimes(1);
  await user.click(screen.getByRole("button", { name: "Apply routes" }));
  const saved = run.mock.calls.find(([p]) => p.command === "mixer_config")?.[0]
    .settings;
  expect(saved.channels.find((c: any) => c.name === "Chat App").apps).toEqual([
    { field: "desktop_application", value: "chat.desktop" },
  ]);
  expect(saved.channels.filter((c: any) => c.capture_unassigned)).toHaveLength(
    1,
  );
  expect(saved.enabled).toBe(false);
});

it("quick-creates a physical input channel without selecting owned devices or enabling routes", async () => {
  const user = userEvent.setup(),
    run = vi.fn(async (_p: Record<string, any>) => true);
  render(<AudioMixer snapshot={inventory()} run={run} />);
  await user.click(screen.getByRole("button", { name: "＋ Create channel" }));
  expect(
    screen.queryByRole("button", { name: "Owned mixer source" }),
  ).toBeNull();
  await user.click(screen.getByRole("button", { name: "USB microphone" }));
  expect(
    screen.getByText("USB microphone", { selector: "strong" }),
  ).toBeTruthy();
  expect(
    screen.getByRole("button", { name: "Add route USB microphone to Monitor" }),
  ).toBeTruthy();
  await user.click(screen.getByRole("button", { name: "Apply routes" }));
  const config = run.mock.calls.find(([p]) => p.command === "mixer_config")?.[0]
    .settings;
  expect(
    config.channels.find((c: any) => c.name === "USB microphone").sources,
  ).toEqual(["physical.mic"]);
  expect(config.enabled).toBe(false);
  expect(config.routes).toHaveLength(1);
});

it("keeps fade preferences in setup and exposes saving/fading status in both views", async () => {
  const user = userEvent.setup(),
    snapshot = { ...inventory(), fading: true };
  render(<AudioMixer snapshot={snapshot} run={vi.fn(async () => true)} />);
  expect(
    screen.queryByRole("spinbutton", { name: "Default fade (ms)" }),
  ).toBeNull();
  expect(screen.getByText("Fading…")).toBeTruthy();
  await user.click(screen.getByRole("button", { name: "Routing setup" }));
  expect(
    screen.getByRole("spinbutton", { name: "Default fade (ms)" }),
  ).toBeTruthy();
  expect(screen.getByText("Fading…")).toBeTruthy();
});

it("saves listening preferences in order while retaining routing drafts and guarding fixed outputs", async () => {
  const user = userEvent.setup(),
    run = vi.fn(async (_p: Record<string, any>) => true);
  let live = { ...inventory(), revision: 5 };
  live.sinks.push({
    name: "headphones",
    description: "Headphones",
    owned: false,
    monitor: false,
    muted: true,
  });
  const calls: any[] = [];
  window.deckforge = {
    onSnapshot: () => () => {},
    request: vi.fn(async (method, payload: any) => {
      expect(method).toBe("mixer_control");
      expect(payload.revision).toBe(live.revision);
      calls.push(payload);
      live = {
        ...live,
        revision: live.revision + 1,
        config: patchMixerControl(live.config, payload.target, payload),
      };
      return structuredClone(live) as any;
    }),
  };
  render(<AudioMixer snapshot={live} run={run} />);
  setupView();
  fireEvent.change(screen.getByRole("textbox", { name: "Monitor mix name" }), {
    target: { value: "Pending name" },
  });
  dailyView();
  await user.selectOptions(
    screen.getByRole("combobox", { name: "Listen on" }),
    "headphones",
  );
  await user.click(screen.getByRole("button", { name: "Listen to Stream" }));
  await waitFor(() => expect(calls).toHaveLength(2));
  expect(calls.map((c) => c.revision)).toEqual([5, 6]);
  expect(live.config.monitoring).toEqual({
    output: "headphones",
    mix: "stream",
  });
  expect(live.config.enabled).toBe(false);
  expect(screen.getByText(/Listening device is muted/)).toBeTruthy();
  expect(
    screen.queryByRole("option", {
      name: "Unavailable · missing.speakers",
    }) as HTMLOptionElement | null,
  ).toBeNull();
  setupView();
  expect(
    (
      screen.getAllByRole("checkbox", {
        name: "Headphones · used by Listen on",
      })[0] as HTMLInputElement
    ).disabled,
  ).toBe(true);
  expect(
    (
      screen.getByRole("textbox", {
        name: "Pending name mix name",
      }) as HTMLInputElement
    ).value,
  ).toBe("Pending name");
  dailyView();
  await user.click(screen.getByRole("button", { name: "Listen to Stream" }));
  await waitFor(() => expect(live.config.monitoring?.mix).toBeNull());
  expect(live.config.monitoring?.output).toBe("headphones");
  await user.selectOptions(
    screen.getByRole("combobox", { name: "Listen on" }),
    "",
  );
  await waitFor(() => expect(live.config.monitoring?.output).toBeNull());
  expect(screen.getByText("No listening output")).toBeTruthy();
});
it("keeps unavailable monitoring output explicit and uses applied monitoring to show switching", () => {
  const snapshot = { ...inventory(), revision: 1, active: true };
  snapshot.config.monitoring = { output: "gone.headphones", mix: "monitor" };
  const view = render(
    <AudioMixer snapshot={snapshot} run={vi.fn(async () => true)} />,
  );
  expect(
    screen.getByText("Unavailable output · saved selection kept"),
  ).toBeTruthy();
  expect(
    (screen.getByRole("combobox", { name: "Listen on" }) as HTMLSelectElement)
      .value,
  ).toBe("gone.headphones");
  const next = structuredClone(snapshot);
  next.config.monitoring = { output: "physical.output", mix: "monitor" };
  next.applied_config = {
    ...structuredClone(next.config),
    monitoring: { output: null, mix: null },
  };
  view.rerender(<AudioMixer snapshot={next} run={vi.fn(async () => true)} />);
  expect(screen.getByText("Switching listening output…")).toBeTruthy();
  next.applied_config = structuredClone(next.config);
  view.rerender(
    <AudioMixer
      snapshot={{ ...next, pending: true, fading: true }}
      run={vi.fn(async () => true)}
    />,
  );
  expect(screen.getByText("Listening to Monitor")).toBeTruthy();
  expect(screen.queryByText("Switching listening output…")).toBeNull();
});
it("disables listening to draft mixes but allows a shared fixed output", async () => {
  const user = userEvent.setup(),
    snapshot = { ...inventory(), revision: 1 };
  snapshot.config.mixes[0].outputs.push("physical.output");
  render(<AudioMixer snapshot={snapshot} run={vi.fn(async () => true)} />);
  expect(
    (
      screen.getByRole("option", {
        name: "Desktop speakers · fixed output of Monitor",
      }) as HTMLOptionElement
    ).disabled,
  ).toBe(false);
  setupView();
  await user.click(screen.getByRole("button", { name: "＋ Add mix" }));
  dailyView();
  expect(
    (
      screen.getByRole("button", {
        name: "Listen to Mix 3",
      }) as HTMLButtonElement
    ).disabled,
  ).toBe(true);
});

it("shows only the canonical recording input and never substitutes a legacy monitor source", () => {
  const snapshot = inventory(),
    run = vi.fn(async () => true);
  const legacy = {
    name: "deckforge.fixture.source.monitor",
    description: "Legacy mix monitor",
    owned: true,
    monitor: true,
  };
  const canonical = {
    name: "deckforge.fixture.source.monitor.input",
    description: "Deckforge mix monitor (recording)",
    owned: true,
    monitor: false,
  };
  snapshot.sources = [...snapshot.sources, legacy, canonical];
  snapshot.recording_sources = { monitor: canonical.name };
  const view = render(<AudioMixer snapshot={snapshot} run={run} />);
  setupView();
  const card = screen.getByRole("region", { name: "Monitor output setup" });
  expect(within(card).getByText(canonical.description)).toBeTruthy();
  expect(
    within(card).getByText(canonical.name, { selector: "code" }),
  ).toBeTruthy();
  expect(within(card).queryByText(legacy.description)).toBeNull();
  view.rerender(
    <AudioMixer snapshot={{ ...snapshot, sources: [legacy] }} run={run} />,
  );
  expect(
    within(card).getByText("Enable routing to create this input."),
  ).toBeTruthy();
  expect(within(card).queryByText(legacy.name)).toBeNull();
});
