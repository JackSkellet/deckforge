// @vitest-environment jsdom
import { afterEach, beforeEach, it, expect, vi } from "vitest";
import {
  cleanup,
  act,
  render,
  screen,
  fireEvent,
  waitFor,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import App from "./App";
import { DeviceCanvas } from "./DeviceCanvas";
import { Sequence } from "./Sequence";
import { installFixture } from "./fixture";
import { fixtureDevice } from "./fixtureDevices";
import {
  supportedControllers,
  validControl,
  acceptsInput,
  outsideControls,
  associatedDevices,
  hasEncoderDisplay,
  hasTouchscreen,
  deviceOptionLabel,
} from "./device";
import { importFormDrafts, importTextDrafts } from "./transient";
import type { Snapshot } from "./model";
beforeEach(() => {
  importFormDrafts({});
  importTextDrafts({});
  HTMLDialogElement.prototype.showModal = function () {
    this.setAttribute("open", "");
  };
  HTMLDialogElement.prototype.close = function () {
    this.removeAttribute("open");
  };
});
afterEach(() => {
  cleanup();
  delete window.deckforge;
});
function setup(id: string) {
  installFixture(id);
  const original = window.deckforge!.request;
  const request = vi.fn(original);
  window.deckforge!.request = request;
  return request;
}
it("uses exact supported geometry for every descriptor, including display-only Neo input semantics", () => {
  for (const [id, keys, dials] of [
    ["mini", 6, 0],
    ["classic", 15, 0],
    ["classic-2017", 15, 0],
    ["xl", 32, 0],
    ["plus", 8, 4],
    ["plus-xl", 36, 6],
    ["studio", 32, 2],
    ["galleon", 12, 4],
    ["neo", 8, 0],
    ["pedal", 3, 0],
  ] as const) {
    const model = fixtureDevice(id);
    const snapshot = {
      device_model: id,
      device_geometry: model.geometry,
    } as Snapshot;
    expect(model.geometry.keys).toBe(keys);
    expect(model.geometry.dials).toBe(dials);
    expect(validControl(snapshot, "Keypad", keys - 1)).toBe(true);
    expect(validControl(snapshot, "Keypad", keys)).toBe(false);
    expect(validControl(snapshot, "Encoder", dials)).toBe(false);
    expect(supportedControllers(snapshot).includes("Encoder")).toBe(dials > 0);
    expect(validControl(snapshot, "Neo", 1)).toBe(id === "neo");
    expect(acceptsInput(snapshot, "Neo", 1)).toBe(false);
  }
});
it("assigns and copies key31 on XL without a fixed eight-key limit", async () => {
  const request = setup("xl");
  const user = userEvent.setup();
  render(<App />);
  const key = await screen.findByRole("button", { name: "Key 32: Counter" });
  expect(screen.getAllByRole("button", { name: /^Key \d+:/ })).toHaveLength(32);
  await user.click(key);
  await user.click(screen.getByRole("button", { name: "Assign Website" }));
  await waitFor(() =>
    expect(request).toHaveBeenCalledWith("command", {
      command: "assign",
      device: "simulated-streamdeck-xl",
      controller: "Keypad",
      index: 31,
      uuid: "com.elgato.streamdeck.system.website",
    }),
  );
  fireEvent.keyDown(screen.getByRole("button", { name: "Key 32: Website" }), {
    key: "c",
    ctrlKey: true,
  });
  await waitFor(() =>
    expect(request).toHaveBeenCalledWith("command", {
      command: "slot_copy",
      device: "simulated-streamdeck-xl",
      context: "fixture-31-Keypad",
    }),
  );
});
it("renders key35 and assigns/copies dial5 on Plus XL", async () => {
  const request = setup("plus-xl");
  const user = userEvent.setup();
  render(<App />);
  await screen.findByRole("button", { name: "Key 36: Counter" });
  expect(screen.getAllByRole("button", { name: /^Key \d+:/ })).toHaveLength(36);
  expect(screen.getAllByRole("button", { name: /^Dial \d+:/ })).toHaveLength(6);
  const dial = screen.getByRole("button", { name: "Dial 6: System volume" });
  await user.click(dial);
  await user.click(
    screen.getByRole("button", { name: "Assign System volume" }),
  );
  await waitFor(() =>
    expect(request).toHaveBeenCalledWith("command", {
      command: "assign",
      device: "simulated-streamdeck-plus-xl",
      controller: "Encoder",
      index: 5,
      uuid: "com.elgato.streamdeck.system.multimedia",
    }),
  );
  fireEvent.keyDown(dial, { key: "c", ctrlKey: true });
  await waitFor(() =>
    expect(request).toHaveBeenCalledWith("command", {
      command: "slot_copy",
      device: "simulated-streamdeck-plus-xl",
      context: "fixture-5-Encoder",
    }),
  );
  await user.click(screen.getByRole("button", { name: "Dial 5: Unassigned" }));
  fireEvent.keyDown(
    screen.getByRole("button", { name: "Dial 5: Unassigned" }),
    { key: "v", ctrlKey: true },
  );
  await waitFor(() =>
    expect(request).toHaveBeenCalledWith("command", {
      command: "slot_paste",
      device: "simulated-streamdeck-plus-xl",
      controller: "Encoder",
      index: 4,
      expected_target: null,
    }),
  );
});
it("does not offer dials or encoder-only actions on Mini", async () => {
  setup("mini");
  render(<App />);
  await screen.findByRole("button", { name: "Key 6: Open application" });
  expect(screen.getAllByRole("button", { name: /^Key \d+:/ })).toHaveLength(6);
  expect(screen.queryByRole("button", { name: /^Dial \d+:/ })).toBeNull();
  expect(screen.queryByRole("button", { name: "Dials" })).toBeNull();
  expect(
    screen.queryByRole("button", { name: "Assign System volume" }),
  ).toBeNull();
  expect(screen.getByRole("option", { name: /Stream Deck Mini/ })).toBeTruthy();
});
it("makes the Neo infobar configurable without a Test input and exposes page sensors", async () => {
  const request = setup("neo");
  const user = userEvent.setup();
  render(<App />);
  const infobar = await screen.findByRole("button", {
    name: "Infobar: Profile infobar",
  });
  await user.click(infobar);
  expect(
    screen.queryByRole("button", { name: /^Test (press|push)$/ }),
  ).toBeNull();
  await user.click(
    screen.getByRole("button", { name: "Assign Profile infobar" }),
  );
  await waitFor(() =>
    expect(request).toHaveBeenCalledWith("command", {
      command: "assign",
      device: "simulated-streamdeck-neo",
      controller: "Neo",
      index: 1,
      uuid: "fixture.neo.infobar",
    }),
  );
  await user.click(screen.getByRole("button", { name: "Next page" }));
  expect(request).toHaveBeenCalledWith("command", {
    command: "page_step",
    device: "simulated-streamdeck-neo",
    direction: 1,
  });
});
it("shows Pedal software assignments without fake key LCDs or brightness", async () => {
  setup("pedal");
  const user = userEvent.setup();
  render(<App />);
  const pedals = await screen.findAllByRole("button", { name: /^Pedal \d+:/ });
  expect(pedals).toHaveLength(3);
  expect(pedals.every((button) => !button.querySelector("img"))).toBe(true);
  expect(
    screen.getByText("Software assignments · This device has no displays"),
  ).toBeTruthy();
  await user.click(screen.getByRole("button", { name: "Preferences" }));
  expect(screen.queryByRole("slider", { name: /Brightness/ })).toBeNull();
  expect(
    screen.getByText("This device has no display brightness control."),
  ).toBeTruthy();
});
it("reports retained out-of-layout records and disables foreign profile activation", async () => {
  setup("mini");
  const snapshot = await window.deckforge!.request<Snapshot>("snapshot");
  snapshot.slots.push({
    ...structuredClone(snapshot.slots[0]),
    context: "outside",
    index: 31,
  });
  snapshot.overflow_slots = [
    {
      controller: "Encoder",
      position: "5,0",
      name: "Retained dial",
      reason: "Outside source layout",
      raw: { Future: { keep: true } },
    },
  ];
  snapshot.library.profiles.push({
    id: "foreign",
    name: "Large profile",
    device_name: "Stream Deck XL",
    device_model: "xl",
    compatible: false,
    readonly: false,
    pin_page: "foreign-pins",
    page_order: [],
    pages: [],
  });
  const original = window.deckforge!.request;
  window.deckforge!.request = vi.fn(async (method, payload) =>
    method === "snapshot"
      ? structuredClone(snapshot)
      : original(method, payload),
  );
  render(<App />);
  expect(outsideControls(snapshot)).toHaveLength(1);
  const summary = await screen.findByText(
    "2 saved controls outside this device layout",
  );
  fireEvent.click(summary);
  fireEvent.click(screen.getByRole("button", { name: "Inspect / copy" }));
  expect(
    (screen.getByRole("button", { name: "Test press" }) as HTMLButtonElement)
      .disabled,
  ).toBe(true);
  expect(
    (screen.getByRole("button", { name: /Large profile/ }) as HTMLButtonElement)
      .disabled,
  ).toBe(true);
  expect(screen.getByText(/These assignments are preserved/)).toBeTruthy();
});

it("models Classic 2017 as fifteen display keys with no dials or unsupported sleep controls", async () => {
  setup("classic-2017");
  render(<App />);
  await screen.findByRole("option", { name: /Stream Deck \(2017\)/ });
  expect(screen.getAllByRole("button", { name: /^Key \d+:/ })).toHaveLength(15);
  expect(screen.queryByRole("button", { name: /^Dial \d+:/ })).toBeNull();
  const model = fixtureDevice("classic-2017");
  expect(model.geometry).toMatchObject({
    columns: 5,
    rows: 3,
    keys: 15,
    dials: 0,
    key_width: 72,
    key_height: 72,
  });
  expect(model.capabilities).toMatchObject({
    key_images: true,
    brightness: true,
    sleep: false,
    full_lcd: false,
  });
  expect(fixtureDevice("unknown").id).toBe("plus");
});
it("disables Neo sensors at the first and last page without wrapping", async () => {
  const request = setup("neo");
  const user = userEvent.setup();
  render(<App />);
  const previous = await screen.findByRole("button", { name: "Previous page" });
  const next = screen.getByRole("button", { name: "Next page" });
  expect((previous as HTMLButtonElement).disabled).toBe(true);
  expect(previous.getAttribute("aria-disabled")).toBe("true");
  expect((next as HTMLButtonElement).disabled).toBe(false);
  await user.click(previous);
  expect(
    request.mock.calls.some(
      ([, payload]) =>
        (payload as { command?: string } | undefined)?.command === "page_step",
    ),
  ).toBe(false);
  await user.click(next);
  await waitFor(() => expect((next as HTMLButtonElement).disabled).toBe(true));
  expect(next.getAttribute("aria-disabled")).toBe("true");
  expect((previous as HTMLButtonElement).disabled).toBe(false);
  const count = request.mock.calls.filter(
    ([, payload]) =>
      (payload as { command?: string } | undefined)?.command === "page_step",
  ).length;
  await user.click(next);
  expect(
    request.mock.calls.filter(
      ([, payload]) =>
        (payload as { command?: string } | undefined)?.command === "page_step",
    ),
  ).toHaveLength(count);
  await user.click(previous);
  await waitFor(() =>
    expect((previous as HTMLButtonElement).disabled).toBe(true),
  );
});
it("uses authoritative folder navigation flags and retains older snapshot fallback", async () => {
  setup("neo");
  const snapshot = await window.deckforge!.request<Snapshot>("snapshot");
  snapshot.library.active_page = "folder";
  snapshot.library.folder_depth = 1;
  snapshot.page_navigation = { previous: false, next: true };
  const onPage = vi.fn();
  const view = render(
    <DeviceCanvas
      snapshot={snapshot}
      renderSlot={() => null}
      onPage={onPage}
    />,
  );
  expect(
    (screen.getByRole("button", { name: "Previous page" }) as HTMLButtonElement)
      .disabled,
  ).toBe(true);
  expect(
    (screen.getByRole("button", { name: "Next page" }) as HTMLButtonElement)
      .disabled,
  ).toBe(false);
  delete snapshot.page_navigation;
  view.rerender(
    <DeviceCanvas
      snapshot={snapshot}
      renderSlot={() => null}
      onPage={onPage}
    />,
  );
  expect(
    (screen.getByRole("button", { name: "Previous page" }) as HTMLButtonElement)
      .disabled,
  ).toBe(false);
  expect(
    (screen.getByRole("button", { name: "Next page" }) as HTMLButtonElement)
      .disabled,
  ).toBe(false);
});

it("keeps the device picker order stable when the backend emits the selected device first", () => {
  const a = {
    id: "A",
    name: "Stream Deck +",
    model: "plus",
    connected: true,
    firmware: "1",
    selected: false,
  };
  const b = { ...a, id: "B", selected: true };
  const snapshot = { devices: [b, a] } as Snapshot;
  expect(associatedDevices(snapshot).map((value) => value.id)).toEqual([
    "A",
    "B",
  ]);
  expect(snapshot.devices?.[0].id).toBe("B");
});

it("disambiguates same-name devices even when their shortened ID suffixes collide", () => {
  const a = {
    id: "source-A-12345678",
    name: "Stream Deck Mini",
    model: "mini",
    connected: true,
    firmware: "1",
    selected: true,
  };
  const b = { ...a, id: "target-B-12345678", selected: false };
  const peers = [a, b];
  expect(deviceOptionLabel(a, peers)).toContain(a.id);
  expect(deviceOptionLabel(b, peers)).toContain(b.id);
  expect(deviceOptionLabel(a, peers)).not.toBe(deviceOptionLabel(b, peers));
});

it("renders Studio rectangular keys and screenless dials with keyboard assignment, copy and input", async () => {
  const request = setup("studio");
  const user = userEvent.setup();
  const model = fixtureDevice("studio");
  expect(model.geometry).toEqual({
    columns: 16,
    rows: 2,
    keys: 32,
    dials: 2,
    physical_dials: 2,
    encoder_columns: 2,
    encoder_rows: 1,
    key_width: 144,
    key_height: 112,
    strip_width: 0,
    strip_height: 0,
  });
  const snapshot = await window.deckforge!.request<Snapshot>("snapshot");
  expect(hasEncoderDisplay(snapshot)).toBe(false);
  expect(hasTouchscreen(snapshot)).toBe(false);
  render(<App />);
  const lastKey = await screen.findByRole("button", {
    name: "Key 32: Counter",
  });
  expect(screen.getAllByRole("button", { name: /^Key \d+:/ })).toHaveLength(32);
  const dials = screen.getAllByRole("button", { name: /^Dial \d+:/ });
  expect(dials).toHaveLength(2);
  expect(dials.every((dial) => !dial.querySelector(":scope > img"))).toBe(true);
  expect(lastKey.title).toBe("Key 32: Counter\nKey 32");
  expect(dials[1].title).toBe("Dial 2: Unassigned");
  lastKey.focus();
  await user.keyboard("{Enter}");
  expect(lastKey.getAttribute("aria-pressed")).toBe("true");
  dials[1].focus();
  await user.keyboard("{Enter}");
  await user.click(
    screen.getByRole("button", { name: "Assign System volume" }),
  );
  await waitFor(() =>
    expect(request).toHaveBeenCalledWith("command", {
      command: "assign",
      device: "simulated-streamdeck-studio",
      controller: "Encoder",
      index: 1,
      uuid: "com.elgato.streamdeck.system.multimedia",
    }),
  );
  const dial = screen.getByRole("button", { name: "Dial 2: System volume" });
  expect(dial.querySelector("img")).toBeNull();
  expect(dial.textContent).toContain("System volume");
  expect(
    screen.getByText("Studio LED and ring feedback is not supported."),
  ).toBeTruthy();
  expect(screen.queryByText("Appearance")).toBeNull();
  expect(screen.queryByRole("button", { name: "Tap screen" })).toBeNull();
  expect(screen.queryByRole("button", { name: "Hold screen" })).toBeNull();
  expect(screen.queryByAltText("Selected control preview")).toBeNull();
  fireEvent.keyDown(dial, { key: "c", ctrlKey: true });
  await waitFor(() =>
    expect(request).toHaveBeenCalledWith("command", {
      command: "slot_copy",
      device: "simulated-streamdeck-studio",
      context: "fixture-1-Encoder",
    }),
  );
  await user.click(screen.getByRole("button", { name: "Test push" }));
  expect(request).toHaveBeenCalledWith("command", {
    command: "input",
    device: "simulated-streamdeck-studio",
    event: "dialDown",
    index: 1,
  });
  await user.click(screen.getByRole("button", { name: "Turn right ↷" }));
  expect(request).toHaveBeenCalledWith("command", {
    command: "input",
    device: "simulated-streamdeck-studio",
    event: "dialRotate",
    index: 1,
    ticks: 1,
  });
  expect(
    request.mock.calls.some(([method]) => method === "appearance_preview"),
  ).toBe(false);
});
it("requires positive encoder image geometry and respects absent touchscreen capability", () => {
  const model = fixtureDevice("plus");
  const snapshot = {
    device_geometry: model.geometry,
    device_capabilities: model.capabilities,
  } as Snapshot;
  expect(hasEncoderDisplay(snapshot)).toBe(true);
  expect(hasTouchscreen(snapshot)).toBe(true);
  snapshot.device_capabilities = { ...model.capabilities, touchscreen: false };
  expect(hasEncoderDisplay(snapshot)).toBe(true);
  expect(hasTouchscreen(snapshot)).toBe(false);
  snapshot.device_geometry = { ...model.geometry, strip_height: 0 };
  expect(hasEncoderDisplay(snapshot)).toBe(false);
  snapshot.device_geometry = {
    ...model.geometry,
    strip_width: -1,
    strip_height: -1,
  };
  expect(hasEncoderDisplay(snapshot)).toBe(false);
});

it("hides screenless stack appearance without dropping its stored value when behavior changes", async () => {
  const onChange = vi.fn();
  const raw = {
    Settings: {
      ShowStackIcon: false,
      FastNavigation: false,
      FutureSetting: "keep",
    },
    Actions: [],
  };
  const props = {
    uuid: "com.elgato.streamdeck.dial.stack",
    raw,
    actions: [],
    outputs: [] as [string, string][],
    onChange,
    onError: vi.fn(),
  };
  const user = userEvent.setup();
  const view = render(<Sequence {...props} encoderDisplay={false} />);
  expect(
    screen.queryByRole("checkbox", { name: "Show stack icon" }),
  ).toBeNull();
  await user.click(
    screen.getByRole("checkbox", { name: "Hold and turn to choose" }),
  );
  expect(onChange).toHaveBeenCalledWith({
    ...raw,
    Settings: { ...raw.Settings, FastNavigation: true },
  });
  view.rerender(<Sequence {...props} encoderDisplay={true} />);
  expect(
    (
      screen.getByRole("checkbox", {
        name: "Show stack icon",
      }) as HTMLInputElement
    ).checked,
  ).toBe(false);
});

it.each(["plus", "studio"])(
  "renders %s dial software icons independently of touchscreen feedback",
  async (model) => {
    setup(model);
    const bridge = window.deckforge!,
      snapshot = await bridge.request<Snapshot>("snapshot");
    const dial = snapshot.slots.find((slot) => slot.controller === "Encoder")!;
    const first =
      "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aZAAAAABJRU5ErkJggg==";
    const second =
      "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
    dial.software_icon = first;
    let emit!: (snapshot: Snapshot) => void;
    window.deckforge = {
      onSnapshot: (callback) => {
        emit = callback;
        return () => {};
      },
      request: async <T,>(method: string, payload?: unknown) =>
        method === "snapshot"
          ? (structuredClone(snapshot) as T)
          : bridge.request<T>(method, payload),
    };
    render(<App />);
    const button = await screen.findByRole("button", {
      name: `Dial 1: ${dial.name}`,
    });
    const software = () =>
      button.querySelector<HTMLImageElement>(".dial-knob .dial-software-icon");
    expect(software()?.getAttribute("src")).toBe(
      `data:image/png;base64,${first}`,
    );
    expect(software()?.draggable).toBe(false);
    const lcd = button.querySelector<HTMLImageElement>(":scope > img");
    expect(lcd?.getAttribute("src") ?? null).toBe(
      model === "plus" ? dial.preview : null,
    );
    const lcdBefore = lcd?.getAttribute("src");
    await act(async () => {
      dial.software_icon = second;
      emit(structuredClone(snapshot));
    });
    expect(software()?.getAttribute("src")).toBe(
      `data:image/png;base64,${second}`,
    );
    expect(
      button
        .querySelector<HTMLImageElement>(":scope > img")
        ?.getAttribute("src"),
    ).toBe(lcdBefore);
    await act(async () => {
      dial.software_icon = "";
      emit(structuredClone(snapshot));
    });
    expect(software()).toBeNull();
    expect(button.querySelector(".dial-knob i")).toBeTruthy();
    expect(
      button
        .querySelector<HTMLImageElement>(":scope > img")
        ?.getAttribute("src"),
    ).toBe(lcdBefore);
  },
);

it("Galleon separates the four 2D widgets from two physical dials", async () => {
  setup("galleon");
  render(<App />);
  await screen.findByRole("button", { name: /^Key 12:/ });
  expect(
    screen.getAllByRole("button", { name: /^Dial action [1-4]:/ }),
  ).toHaveLength(4);
  const grid = document.querySelector<HTMLElement>(".dial-grid")!;
  expect(grid.style.gridTemplateColumns).toBe("repeat(2,minmax(0,1fr))");
  expect(document.querySelectorAll(".galleon-dials > span")).toHaveLength(2);
  expect(document.querySelectorAll(".dial-knob")).toHaveLength(0);
  const descriptor = fixtureDevice("galleon");
  expect(descriptor.geometry.physical_dials).toBe(2);
  expect(descriptor.capabilities.transport_supported).toBe(true);
  expect(descriptor.capabilities.touchscreen).toBe(false);
});
it("Galleon simulator buttons issue complete operations without a held UI input", async () => {
  const descriptor = fixtureDevice("galleon"),
    input = vi.fn(),
    user = userEvent.setup();
  const snapshot = {
    device: "galleon-test",
    device_model: "galleon",
    device_name: descriptor.name,
    device_geometry: descriptor.geometry,
    device_capabilities: descriptor.capabilities,
    encoder_routes: [0, 3],
  } as unknown as Snapshot;
  render(
    <DeviceCanvas
      snapshot={snapshot}
      renderSlot={() => null}
      onPage={vi.fn()}
      onDialInput={input}
    />,
  );
  await user.click(screen.getByRole("button", { name: "Press left dial" }));
  expect(input).toHaveBeenLastCalledWith(0, "click", undefined);
  await user.click(
    screen.getByRole("button", { name: "Switch left dial row" }),
  );
  expect(input).toHaveBeenLastCalledWith(0, "next", undefined);
  await user.click(
    screen.getByRole("button", { name: "Turn right dial right" }),
  );
  expect(input).toHaveBeenLastCalledWith(1, "dialRotate", 1);
  expect(input).toHaveBeenCalledTimes(3);
});
