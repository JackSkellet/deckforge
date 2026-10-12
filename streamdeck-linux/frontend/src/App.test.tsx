// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  cleanup,
  render,
  screen,
  waitFor,
  fireEvent,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import App from "./App";
import { importTextDrafts, importFormDrafts } from "./transient";
import type { Snapshot, Slot, Action } from "./model";
const counter = {
  uuid: "counter",
  name: "Counter",
  plugin: "sample",
  plugin_name: "Sample",
  category: "Utilities",
  controllers: ["Keypad"],
  visible: true,
  multi_only: false,
  supported_in_multi: true,
  defaults: { count: 0, incrementBy: 1 },
  states: [],
} as Action;
const volume = {
  ...counter,
  uuid: "com.elgato.streamdeck.system.multimedia",
  name: "System volume",
  controllers: ["Encoder"],
  defaults: { actionIdx: 18, linuxVolumeStep: 5 },
} as Action;
const slot = {
  context: "counter-1",
  uuid: "counter",
  name: "Counter",
  controller: "Keypad",
  index: 0,
  settings: { count: 7, incrementBy: 1, Future: { keep: true } },
  title: "7",
  pinned: false,
  available: true,
  raw: {},
  has_inspector: true,
  state: 0,
  multi_running: false,
  stack_children: [],
  trigger_children: [],
  multi_children: [],
} as Slot;
let snapshot: Snapshot;
let request: ReturnType<typeof vi.fn>;
let emit: (s: Snapshot) => void;
beforeEach(() => {
  importTextDrafts({});
  importFormDrafts({});
  snapshot = {
    running: true,
    device: "test",
    firmware: "test",
    connected: false,
    device_online: false,
    simulate: true,
    slots: [structuredClone(slot)],
    actions: [counter, volume],
    plugins: [],
    library: {
      active_profile: "p",
      active_page: "page",
      folder_depth: 0,
      profiles: [
        {
          id: "p",
          name: "Daily workspace",
          pin_page: "pins",
          readonly: false,
          page_order: ["page"],
          pages: [{ id: "page", name: "Main", folder: false }],
        },
      ],
    },
    device_settings: { desired: {} },
    screensaver: { active: false, preferences: {} },
    audio_outputs: [],
    audio_active: {},
    multi_runs: [],
    logs: [],
  } as Snapshot;
  request = vi.fn(async (method, payload) => {
    if (method === "snapshot") return structuredClone(snapshot);
    if (method === "ui_state_get") return null;
    if (method === "command" && payload.command === "settings")
      snapshot.slots[0].settings = payload.settings;
    if (method === "command" && payload.command === "appearance")
      snapshot.slots[0].title = payload.title;
    return null;
  });
  window.deckforge = {
    request,
    onSnapshot: (callback) => {
      emit = callback;
      return () => {};
    },
  };
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
describe("desktop interactions", () => {
  it("flushes the newest editor state before native Hide", async () => {
    const user = userEvent.setup();
    render(<App />);
    const title = await screen.findByRole("textbox", { name: "Title" });
    await user.clear(title);
    await user.type(title, "Keep this draft");
    request.mockClear();
    await user.click(screen.getByRole("button", { name: "Hide Deckforge" }));
    const calls = request.mock.calls;
    const hide = calls.findIndex(([method]) => method === "window");
    expect(hide).toBeGreaterThan(0);
    expect(calls[hide - 1][0]).toBe("ui_state_set");
    expect(calls[hide - 1][1].state.drafts["counter-1"].title).toBe(
      "Keep this draft",
    );
    expect(calls[hide][1]).toEqual({ action: "hide" });
  });
  it("keeps unapplied JSON and invalid numbers when changing selected controls", async () => {
    const user = userEvent.setup();
    render(<App />);
    await screen.findByRole("textbox", { name: "Title" });
    await user.click(screen.getByText("Advanced settings"));
    const json = screen.getByRole("textbox", { name: "Advanced settings" });
    await user.clear(json);
    await user.type(json, "{{bad");
    await user.click(screen.getByRole("button", { name: "Key 2: Unassigned" }));
    await user.click(screen.getByRole("button", { name: "Key 1: Counter" }));
    await user.click(screen.getByText(/Advanced settings/));
    expect(
      (
        screen.getByRole("textbox", {
          name: "Advanced settings",
        }) as HTMLTextAreaElement
      ).value,
    ).toBe("{bad");
    expect(
      (
        screen.getByRole("button", {
          name: "Save changes",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(true);
    await user.click(screen.getByRole("button", { name: "Discard JSON text" }));
    const increment = screen.getByRole("spinbutton", { name: "incrementBy" });
    await user.clear(increment);
    await user.click(screen.getByRole("button", { name: "Key 2: Unassigned" }));
    await user.click(screen.getByRole("button", { name: "Key 1: Counter" }));
    expect(
      (
        screen.getByRole("spinbutton", {
          name: "incrementBy",
        }) as HTMLInputElement
      ).value,
    ).toBe("");
  });
  it("restores retained drafts before applying the initial snapshot", async () => {
    const original = request.getMockImplementation()!;
    request.mockImplementation(async (method, payload) =>
      method === "ui_state_get"
        ? {
            drafts: {
              "counter-1": {
                settings: { count: 7, incrementBy: 12 },
                title: "Retained",
                raw: {},
                dirty: true,
                revision: 1,
                source: "old",
              },
            },
            selected: { controller: "Keypad", index: 0 },
            filter: "All",
            search: "",
          }
        : original(method, payload),
    );
    render(<App />);
    expect(
      (
        (await screen.findByRole("textbox", {
          name: "Title",
        })) as HTMLInputElement
      ).value,
    ).toBe("Retained");
    expect(
      (
        screen.getByRole("spinbutton", {
          name: "incrementBy",
        }) as HTMLInputElement
      ).value,
    ).toBe("12");
  });

  it("edits a numeric plugin setting and preserves unknown settings on save", async () => {
    const user = userEvent.setup();
    render(<App />);
    const increment = await screen.findByRole("spinbutton", {
      name: "incrementBy",
    });
    await user.clear(increment);
    await user.type(increment, "5");
    await user.click(screen.getByRole("button", { name: "Save changes" }));
    await waitFor(() =>
      expect(request).toHaveBeenCalledWith("command", {
        command: "settings",
        context: "counter-1",
        settings: { count: 7, incrementBy: 5, Future: { keep: true } },
      }),
    );
  });
  it("keeps unsaved edits through snapshot pushes and selection switches", async () => {
    const user = userEvent.setup();
    render(<App />);
    const title = await screen.findByRole("textbox", { name: "Title" });
    await user.clear(title);
    await user.type(title, "My counter");
    snapshot.slots[0].title = "Remote title";
    emit(structuredClone(snapshot));
    await user.click(screen.getByRole("button", { name: "Key 2: Unassigned" }));
    await user.click(screen.getByRole("button", { name: "Key 1: Counter" }));
    expect(
      (screen.getByRole("textbox", { name: "Title" }) as HTMLInputElement)
        .value,
    ).toBe("My counter");
  });
  it("supports keyboard action assignment and rejects incompatible drop", async () => {
    const user = userEvent.setup();
    render(<App />);
    await screen.findByRole("heading", { name: "Daily workspace" });
    expect(
      (
        screen.getByRole("button", {
          name: "Assign System volume",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(true);
    await user.click(
      screen.getByRole("button", { name: "Dial 1: Unassigned" }),
    );
    await user.click(
      screen.getByRole("button", { name: "Assign System volume" }),
    );
    await waitFor(() =>
      expect(request).toHaveBeenCalledWith("command", {
        command: "assign",
        controller: "Encoder",
        index: 0,
        uuid: volume.uuid,
      }),
    );
    fireEvent.drop(screen.getByRole("button", { name: "Key 2: Unassigned" }), {
      dataTransfer: { getData: () => volume.uuid },
    });
    expect(await screen.findByRole("alert")).toHaveProperty(
      "textContent",
      expect.stringContaining("does not support"),
    );
  });
  it("opens the native property inspector without embedding plugin content", async () => {
    const user = userEvent.setup();
    render(<App />);
    await user.click(
      await screen.findByRole("button", { name: "Plugin settings ↗" }),
    );
    expect(request).toHaveBeenCalledWith("command", {
      command: "open_inspector",
      context: "counter-1",
    });
    expect(document.querySelector("iframe")).toBeNull();
  });
  it("opens profile rename with current value and sends typed command", async () => {
    const user = userEvent.setup();
    render(<App />);
    await screen.findByRole("heading", { name: "Daily workspace" });
    await user.click(screen.getByText("Profile options"));
    await user.click(screen.getByRole("button", { name: "Rename profile" }));
    const name = screen.getByRole("textbox", { name: "Name" });
    await user.clear(name);
    await user.type(name, "Broadcast");
    await user.click(screen.getByRole("button", { name: /^Save$/ }));
    await waitFor(() =>
      expect(request).toHaveBeenCalledWith("command", {
        command: "profile_rename",
        id: "p",
        name: "Broadcast",
      }),
    );
  });
});
