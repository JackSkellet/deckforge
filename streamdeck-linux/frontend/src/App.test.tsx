// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  cleanup,
  render,
  screen,
  waitFor,
  fireEvent,
  act,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import App from "./App";
import { applyFixtureSettingsPatch } from "./fixture";
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
    if (
      method === "command" &&
      ["slot_copy", "slot_cut"].includes(payload.command)
    ) {
      const source = snapshot.slots.find((s) => s.context === payload.context)!;
      if (source.controller === "Neo") throw new Error("Not assignable");
      snapshot.clipboard = {
        context: source.context,
        name: source.name,
        uuid: source.uuid,
        controller: source.controller,
        mode: payload.command === "slot_cut" ? "cut" : "copy",
      };
    }
    if (method === "command" && payload.command === "slot_clipboard_clear")
      snapshot.clipboard = null;
    if (method === "command" && payload.command === "settings")
      snapshot.slots[0].settings = payload.settings;
    if (method === "command" && payload.command === "settings_patch")
      snapshot.slots[0].settings = applyFixtureSettingsPatch(
        payload.base,
        payload.settings,
        snapshot.slots[0].settings,
      );
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
  it("merges a container checkbox against a snapshot arriving before its click handler runs", async () => {
    const wheel = "com.elgato.streamdeck.keys.stack";
    snapshot.slots = [
      {
        ...slot,
        uuid: wheel,
        name: "Action Wheel",
        controller: "Encoder",
        settings: { CurrentIdx: 1, InvertRotation: false },
        raw: {
          Settings: { CurrentIdx: 1, InvertRotation: false },
          Actions: [],
        },
      } as Slot,
    ];
    const user = userEvent.setup();
    render(<App />);
    await user.click(
      await screen.findByRole("button", { name: "Dial 1: Action Wheel" }),
    );
    await user.click(screen.getByText("Wheel options"));
    const toggle = screen.getByRole("checkbox", { name: "Reverse rotation" });
    act(() => {
      snapshot.slots[0] = {
        ...snapshot.slots[0],
        settings: { CurrentIdx: 0, InvertRotation: false },
        raw: {
          Settings: { CurrentIdx: 0, InvertRotation: false },
          Actions: [],
        },
      };
      emit(structuredClone(snapshot));
      fireEvent.click(toggle);
    });
    await user.click(screen.getByRole("button", { name: "Save changes" }));
    await waitFor(() =>
      expect(request).toHaveBeenCalledWith(
        "command",
        expect.objectContaining({
          command: "wheel_save",
          raw: {
            Settings: { CurrentIdx: 0, InvertRotation: true },
            Actions: [],
          },
        }),
      ),
    );
  });
  it("offers the global Multi Action feedback preference with stale-write protection", async () => {
    snapshot.multi_action_feedback = true;
    const user = userEvent.setup();
    render(<App />);
    await user.click(
      await screen.findByRole("button", { name: "Preferences" }),
    );
    await user.click(screen.getByRole("button", { name: "Startup & about" }));
    const toggle = screen.getByRole("checkbox", {
      name: "Show Multi Action progress and completion check marks",
    }) as HTMLInputElement;
    expect(toggle.checked).toBe(true);
    await user.click(toggle);
    expect(request).toHaveBeenCalledWith(
      "command",
      expect.objectContaining({
        command: "multi_action_feedback",
        enabled: false,
        baseline: true,
      }),
    );
  });
  it("preserves multiline titles, inserts Enter without submitting, and saves explicit title intent", async () => {
    snapshot.slots[0].title = "Imported\nTitle";
    const user = userEvent.setup();
    render(<App />);
    const title = (await screen.findByRole("textbox", {
      name: "Title",
    })) as HTMLTextAreaElement;
    expect(title.tagName).toBe("TEXTAREA");
    expect(title.value).toBe("Imported\nTitle");
    expect(title.rows).toBe(2);
    await user.type(title, "{Enter}Third{Enter}Fourth");
    expect(title.value).toBe("Imported\nTitle\nThird\nFourth");
    expect(title.rows).toBe(3);
    expect(request.mock.calls.some(([method]) => method === "command")).toBe(
      false,
    );
    await user.click(screen.getByRole("button", { name: "Save changes" }));
    await waitFor(() =>
      expect(request).toHaveBeenCalledWith("command", {
        device: "test",
        command: "appearance",
        context: "counter-1",
        expected_state: 0,
        state: 0,
        title: "Imported\nTitle\nThird\nFourth",
      }),
    );
    expect(
      request.mock.calls.some(([, payload]) => payload?.command === "settings"),
    ).toBe(false);
  });

  it("opens the clicked control's edit menu with right-click and keyboard context keys", async () => {
    snapshot.slots.push({
      ...structuredClone(slot),
      context: "second",
      index: 1,
      name: "Second counter",
    });
    render(<App />);
    const target = await screen.findByRole("button", {
      name: "Key 2: Second counter",
    });
    expect(fireEvent.contextMenu(target, { clientX: 250, clientY: 200 })).toBe(
      false,
    );
    expect(
      screen.getByRole("heading", { name: "Second counter" }),
    ).toBeTruthy();
    const summary = screen.getByText("Edit ▾");
    expect((summary.closest("details") as HTMLDetailsElement).open).toBe(true);
    const copy = screen.getByRole("button", { name: "Copy Ctrl C" });
    expect(document.activeElement).toBe(copy);
    fireEvent.click(copy);
    await waitFor(() =>
      expect(request).toHaveBeenCalledWith("command", {
        device: "test",
        command: "slot_copy",
        context: "second",
      }),
    );
    fireEvent.keyDown(target, { key: "F10", shiftKey: true });
    expect((summary.closest("details") as HTMLDetailsElement).open).toBe(true);
    fireEvent.keyDown(copy, { key: "Escape" });
    expect((summary.closest("details") as HTMLDetailsElement).open).toBe(false);
    fireEvent.keyDown(target, { key: "ContextMenu" });
    expect((summary.closest("details") as HTMLDetailsElement).open).toBe(true);
  });
  it("preserves text context menus and does not open the control menu behind a dialog", async () => {
    const user = userEvent.setup();
    render(<App />);
    const title = await screen.findByRole("textbox", { name: "Title" });
    expect(fireEvent.contextMenu(title)).toBe(true);
    const summary = screen.getByText("Edit ▾");
    expect((summary.closest("details") as HTMLDetailsElement).open).toBe(false);
    const target = screen.getByRole("button", { name: "Key 1: Counter" });
    await user.click(screen.getByRole("button", { name: /^Preferences$/ }));
    fireEvent.contextMenu(target);
    fireEvent.keyDown(target, { key: "F10", shiftKey: true });
    expect((summary.closest("details") as HTMLDetailsElement).open).toBe(false);
  });

  it("keeps a dirty cut source on another page instead of pasting it", async () => {
    snapshot.clipboard = {
      name: "Counter",
      uuid: "counter",
      controller: "Keypad",
      mode: "cut",
      context: "hidden-source",
    };
    const original = request.getMockImplementation()!;
    request.mockImplementation(async (method, payload) =>
      method === "ui_state_get"
        ? {
            drafts: {
              "hidden-source": {
                dirty: true,
                settings: { count: 55 },
                title: "Unsaved",
                raw: {},
                revision: 1,
                source: "previous",
              },
            },
          }
        : original(method, payload),
    );
    render(<App />);
    const target = await screen.findByRole("button", {
      name: "Key 2: Unassigned",
    });
    fireEvent.click(target);
    fireEvent.keyDown(target, { key: "v", ctrlKey: true });
    expect(await screen.findByRole("alert")).toHaveProperty(
      "textContent",
      expect.stringContaining("Save or revert changes"),
    );
    expect(
      request.mock.calls.some(
        ([, payload]) => payload?.command === "slot_paste",
      ),
    ).toBe(false);
  });
  it("allows copying read-only actions but blocks cut and removal", async () => {
    snapshot.library.profiles[0].readonly = true;
    render(<App />);
    const source = await screen.findByRole("button", {
      name: "Key 1: Counter",
    });
    fireEvent.keyDown(source, { key: "x", ctrlKey: true });
    fireEvent.keyDown(source, { key: "Delete" });
    expect(
      request.mock.calls.some(([, payload]) =>
        ["slot_cut", "remove"].includes(payload?.command),
      ),
    ).toBe(false);
    fireEvent.keyDown(source, { key: "c", ctrlKey: true });
    await waitFor(() =>
      expect(request).toHaveBeenCalledWith("command", {
        device: "test",
        command: "slot_copy",
        context: "counter-1",
      }),
    );
  });

  it("copies saved actions and pastes to an exact empty destination with keyboard shortcuts", async () => {
    render(<App />);
    const source = await screen.findByRole("button", {
      name: "Key 1: Counter",
    });
    fireEvent.keyDown(source, { key: "c", ctrlKey: true });
    await waitFor(() =>
      expect(request).toHaveBeenCalledWith("command", {
        device: "test",
        command: "slot_copy",
        context: "counter-1",
      }),
    );
    await waitFor(() =>
      expect(screen.getByText("Copied: Counter · Key")).toBeTruthy(),
    );
    const target = screen.getByRole("button", { name: "Key 2: Unassigned" });
    fireEvent.click(target);
    fireEvent.keyDown(target, { key: "v", ctrlKey: true });
    await waitFor(() =>
      expect(request).toHaveBeenCalledWith("command", {
        device: "test",
        command: "slot_paste",
        controller: "Keypad",
        index: 1,
        expected_target: null,
      }),
    );
  });
  it("keeps text-editing copy cut paste and Delete shortcuts native", async () => {
    render(<App />);
    const title = await screen.findByRole("textbox", { name: "Title" });
    request.mockClear();
    for (const key of ["c", "x", "v", "Delete"]) {
      const event = new KeyboardEvent("keydown", {
        key,
        ctrlKey: key !== "Delete",
        bubbles: true,
        cancelable: true,
      });
      title.dispatchEvent(event);
      expect(event.defaultPrevented).toBe(false);
    }
    expect(request.mock.calls.some(([method]) => method === "command")).toBe(
      false,
    );
  });
  it("blocks copy and removal of dirty action drafts", async () => {
    const user = userEvent.setup();
    render(<App />);
    const title = await screen.findByRole("textbox", { name: "Title" });
    await user.type(title, " changed");
    const source = screen.getByRole("button", { name: "Key 1: Counter" });
    request.mockClear();
    fireEvent.keyDown(source, { key: "c", ctrlKey: true });
    expect(await screen.findByRole("alert")).toHaveProperty(
      "textContent",
      expect.stringContaining("Save or revert changes"),
    );
    fireEvent.keyDown(source, { key: "Delete" });
    expect(request.mock.calls.some(([method]) => method === "command")).toBe(
      false,
    );
  });
  it("marks the exact cut action and lets Escape cancel without removing it", async () => {
    render(<App />);
    const source = await screen.findByRole("button", {
      name: "Key 1: Counter",
    });
    fireEvent.keyDown(source, { key: "x", ctrlKey: true });
    await waitFor(() =>
      expect(source.classList.contains("cut-control")).toBe(true),
    );
    fireEvent.keyDown(source, { key: "Escape" });
    await waitFor(() =>
      expect(request).toHaveBeenCalledWith("command", {
        device: "test",
        command: "slot_clipboard_clear",
      }),
    );
    await waitFor(() =>
      expect(source.classList.contains("cut-control")).toBe(false),
    );
    expect(
      request.mock.calls.some(([, payload]) => payload?.command === "remove"),
    ).toBe(false);
  });
  it("sends occupied drag targets explicitly and uses Ctrl-drop for copies", async () => {
    snapshot.slots.push({
      ...structuredClone(slot),
      context: "second",
      index: 1,
    });
    render(<App />);
    const target = await screen.findByRole("button", {
      name: "Key 2: Counter",
    });
    const transfer = {
      getData: (type: string) =>
        type === "application/x-deckforge-slot" ? "counter-1" : "",
    };
    fireEvent.drop(target, { dataTransfer: transfer, ctrlKey: false });
    await waitFor(() =>
      expect(request).toHaveBeenCalledWith("command", {
        device: "test",
        command: "slot_transfer",
        context: "counter-1",
        controller: "Keypad",
        index: 1,
        copy: false,
        expected_target: "second",
      }),
    );
    await waitFor(() =>
      expect(
        (screen.getByRole("textbox", { name: "Title" }) as HTMLInputElement)
          .disabled,
      ).toBe(false),
    );
    const dropEvent = new Event("drop", { bubbles: true, cancelable: true });
    Object.assign(dropEvent, {
      dataTransfer: transfer,
      ctrlKey: true,
      metaKey: false,
    });
    fireEvent(target, dropEvent);
    await waitFor(() =>
      expect(request).toHaveBeenCalledWith("command", {
        device: "test",
        command: "slot_transfer",
        context: "counter-1",
        controller: "Keypad",
        index: 1,
        copy: true,
        expected_target: "second",
      }),
    );
  });
  it("advertises a standard Linux drag format without accepting plain text as an action", async () => {
    render(<App />);
    const target = await screen.findByRole("button", {
      name: "Key 2: Unassigned",
    });
    const library = document.querySelector(".library-action")!;
    const data = new Map<string, string>();
    const transfer = {
      setData: (type: string, value: string) => data.set(type, value),
      getData: (type: string) => data.get(type) ?? "",
      get types() {
        return [...data.keys()];
      },
      effectAllowed: "none",
      dropEffect: "none",
    };
    fireEvent.dragStart(library, { dataTransfer: transfer });
    expect(transfer.effectAllowed).toBe("copy");
    expect(transfer.types).toContain("text/plain");
    expect(transfer.getData("text/plain")).not.toBe("");
    expect(transfer.getData("application/x-deckforge-action")).toBe("counter");
    expect(fireEvent.dragOver(target, { dataTransfer: transfer })).toBe(false);
    expect(transfer.dropEffect).toBe("copy");
    fireEvent.drop(target, { dataTransfer: transfer });
    await waitFor(() =>
      expect(request).toHaveBeenCalledWith("command", {
        device: "test",
        command: "assign",
        controller: "Keypad",
        index: 1,
        uuid: "counter",
      }),
    );
    request.mockClear();
    data.delete("application/x-deckforge-action");
    fireEvent.drop(target, { dataTransfer: transfer });
    expect(request.mock.calls.some(([, p]) => p?.command === "assign")).toBe(
      false,
    );
    data.clear();
    const source = screen.getByRole("button", { name: "Key 1: Counter" });
    fireEvent.dragStart(source, { dataTransfer: transfer });
    expect(transfer.effectAllowed).toBe("copyMove");
    expect(transfer.getData("text/plain")).not.toBe("");
    expect(transfer.getData("application/x-deckforge-slot")).toBe("counter-1");
  });

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
                source: JSON.stringify([slot.settings, slot.title, slot.raw]),
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
        device: "test",
        command: "settings_patch",
        context: "counter-1",
        base: { count: 7, incrementBy: 1, Future: { keep: true } },
        settings: { count: 7, incrementBy: 5, Future: { keep: true } },
      }),
    );
  });
  it("keeps plugin updates arriving between a rendered control and its edit callback", async () => {
    render(<App />);
    const increment = await screen.findByRole("spinbutton", {
      name: "incrementBy",
    });
    act(() => {
      snapshot.slots[0].settings = {
        count: 8,
        incrementBy: 1,
        Future: { keep: false, added: "runtime" },
        titleKey: "clickme",
      };
      emit(structuredClone(snapshot));
      fireEvent.change(increment, { target: { value: "3" } });
    });
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));
    await waitFor(() =>
      expect(request).toHaveBeenCalledWith("command", {
        device: "test",
        command: "settings_patch",
        context: "counter-1",
        base: {
          count: 8,
          incrementBy: 1,
          Future: { keep: false, added: "runtime" },
          titleKey: "clickme",
        },
        settings: {
          count: 8,
          incrementBy: 3,
          Future: { keep: false, added: "runtime" },
          titleKey: "clickme",
        },
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
        device: "test",
        command: "assign",
        controller: "Encoder",
        index: 0,
        uuid: volume.uuid,
      }),
    );
    fireEvent.drop(screen.getByRole("button", { name: "Key 2: Unassigned" }), {
      dataTransfer: {
        getData: (type: string) =>
          type === "application/x-deckforge-action" ? volume.uuid : "",
      },
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
      device: "test",
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
        device: "test",
        command: "profile_rename",
        id: "p",
        name: "Broadcast",
      }),
    );
  });
});
