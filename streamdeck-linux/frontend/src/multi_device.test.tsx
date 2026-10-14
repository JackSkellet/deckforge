// @vitest-environment jsdom
import { beforeEach, afterEach, it, expect, vi } from "vitest";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import App from "./App";
import { installFixture, applyFixtureSettingsPatch } from "./fixture";
import { fixtureDevice } from "./fixtureDevices";
import { importFormDrafts, importTextDrafts } from "./transient";
import type { Snapshot, RecordValue } from "./model";
let snapshots: Record<string, Snapshot>;
let selected: string;
let listener: ((value: Snapshot) => void) | undefined;
let state: RecordValue | null;
let request: ReturnType<typeof vi.fn>;
let beforeCommand: ((payload: RecordValue) => Promise<void>) | undefined;
let pickFile: (() => Promise<string | null>) | undefined;
let copyTargets: Set<string>;
let pendingActionFile: Snapshot["pending_action_file"];
const originalElementFromPoint = document.elementFromPoint;
it("keeps the global typing Stop available while inspecting another action", async () => {
  snapshots["deck-A"].typing_status = { phase: "typing", message: "Sending text" };
  render(<App />);
  await userEvent.click(await screen.findByRole("button", { name: "Stop desktop typing" }));
  expect(request).toHaveBeenCalledWith("command", { command: "text_typing_disable" });
});
async function nativeFileDrop(
  element: Element | null,
  kind = "action",
  phase = "drop",
) {
  Object.defineProperty(document, "elementFromPoint", {
    configurable: true,
    value: () => element,
  });
  await act(async () => {
    window.dispatchEvent(
      new CustomEvent("deckforge-file-drag", {
        detail: {
          phase,
          path:
            kind === "action" ? "/tmp/shared.streamDeckAction"
              : kind === "plugin" ? "/tmp/package.streamDeckPlugin"
              : kind === "icon_pack" ? "/tmp/icons.streamDeckIconPack"
              : "/tmp/profile.streamDeckProfile",
          kind,
          x: 10,
          y: 20,
        },
      }),
    );
  });
}
afterEach(() =>
  Object.defineProperty(document, "elementFromPoint", {
    configurable: true,
    value: originalElementFromPoint,
  }),
);
function view() {
  const current = structuredClone(snapshots[selected]);
  current.pending_action_file = pendingActionFile;
  current.devices = Object.entries(snapshots).map(([id, snapshot]) => ({
    id,
    name: snapshot.device_name!,
    model: snapshot.device_model!,
    connected: snapshot.connected,
    firmware: snapshot.firmware,
    selected: id === selected,
    online: snapshot.device_online,
    profile_compatible: id === selected || copyTargets.has(id),
  }));
  return current;
}
async function switchDevice(id: string) {
  selected = id;
  await act(async () => listener?.(view()));
}
beforeEach(async () => {
  importFormDrafts({});
  importTextDrafts({});
  installFixture("plus");
  const source = await window.deckforge!.request<Snapshot>("snapshot");
  snapshots = {};
  selected = "deck-A";
  listener = undefined;
  state = null;
  beforeCommand = undefined;
  pickFile = undefined;
  copyTargets = new Set();
  pendingActionFile = null;
  for (const [id, modelId] of [
    ["deck-A", "plus"],
    ["deck-B", "mini"],
  ]) {
    const model = fixtureDevice(modelId);
    const snapshot = structuredClone(source);
    snapshot.device = id;
    snapshot.device_model = model.id;
    snapshot.device_name = model.name;
    snapshot.device_geometry = model.geometry;
    snapshot.device_capabilities = model.capabilities;
    snapshot.connected = id === "deck-A";
    snapshot.device_online = id === "deck-A";
    snapshot.simulate = false;
    snapshot.slots = snapshot.slots
      .filter((slot) =>
        slot.controller === "Keypad"
          ? slot.index < model.geometry.keys
          : slot.controller === "Encoder" && slot.index < model.geometry.dials,
      )
      .map((slot) => ({ ...slot, context: `${id}:${slot.context}` }));
    snapshot.library.profiles = snapshot.library.profiles.map((profile) => ({
      ...profile,
      device_name: model.name,
      device_model: model.id,
    }));
    snapshots[id] = snapshot;
  }
  request = vi.fn(async (method: string, payload?: any) => {
    if (method === "snapshot") return view();
    if (method === "ui_state_get") return state;
    if (method === "ui_state_set") {
      state = structuredClone(payload.state);
      return null;
    }
    if (method === "action_icon") return null;
    if (method === "icon_packs") return { packs: [], errors: [], archives: [] };
    if (method === "fonts" || method === "application_catalog") return [];
    if (method === "pick_file") return pickFile ? pickFile() : null;
    if (method === "open_file") return "File imported";
    if (method === "action_file_choose" || method === "action_file_cancel") {
      pendingActionFile = null; listener?.(view()); return "Action imported";
    }
    if (method === "action_file_prepare") {
      await beforeCommand?.({ command: "slot_import", device: payload.device, path: payload.path, ...payload.destination });
      return "Action imported";
    }
    if (method === "startup_status")
      return { enabled: false, this_session: true };
    if (method === "appearance_preview") {
      const target = snapshots[payload.device];
      const slot = target?.slots.find(
        (slot) => slot.context === payload.context,
      );
      if (!slot) throw new Error("Preview owner mismatch");
      return {
        preview: slot.preview,
        state: payload.state,
        title: payload.title ?? slot.title,
        ...slot.appearance,
      };
    }
    if (method === "command") {
      if (payload.command === "smart_preview") return null;
      if (payload.command === "device_select") {
        selected = payload.device;
        listener?.(view());
        return null;
      }
      await beforeCommand?.(payload);
      const target = snapshots[payload.device];
      if (!target) throw new Error("Captured device is required");
      const slot = target.slots.find(
        (slot) => slot.context === payload.context,
      );
      if (payload.context && !slot) throw new Error("Action owner mismatch");
      if (payload.command === "settings")
        slot!.settings = structuredClone(payload.settings);
      if (payload.command === "settings_patch")
        slot!.settings = applyFixtureSettingsPatch(
          payload.base,
          payload.settings,
          slot!.settings,
        );
      if (payload.command === "appearance" && payload.title !== undefined)
        slot!.title = payload.title;
      if (payload.command === "profile_copy_to_device") {
        const original = target.library.profiles.find(
          (profile) => profile.id === payload.profile,
        )!;
        const destination = snapshots[payload.target_device];
        const copied = {
          ...structuredClone(original),
          id: `copy:${payload.device}:${payload.target_device}`,
          name: payload.name ?? `${original.name} copy`,
        };
        destination.library.profiles.push(copied);
        destination.library.active_profile = copied.id;
      }
      if (payload.command === "profile_create")
        target.library.profiles.push({
          id: `created:${payload.device}`,
          name: payload.name,
          readonly: false,
          pin_page: "new-pins",
          page_order: [],
          pages: [],
        });
      listener?.(view());
      return null;
    }
    return null;
  });
  window.deckforge = {
    request,
    onSnapshot(callback) {
      listener = callback;
      return () => {
        listener = undefined;
      };
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
async function select(user: ReturnType<typeof userEvent.setup>, id: string) {
  await user.selectOptions(
    screen.getByRole("combobox", { name: "Selected device" }),
    id,
  );
  await waitFor(() => {
    const picker = screen.getByRole("combobox", {
      name: "Selected device",
    }) as HTMLSelectElement;
    expect(picker.value).toBe(id);
    expect(picker.disabled).toBe(false);
  });
}
it("switches to an offline associated device while retaining flat drafts and each device selection", async () => {
  const user = userEvent.setup();
  render(<App />);
  const title = await screen.findByRole("textbox", { name: "Title" });
  await user.clear(title);
  await user.type(title, "Device A draft");
  await user.click(
    screen.getByRole("button", { name: "Dial 1: System volume" }),
  );
  await select(user, "deck-B");
  expect(
    screen.getByRole("option", { name: /Stream Deck Mini.*Offline/ }),
  ).toBeTruthy();
  await user.click(screen.getByRole("button", { name: "Key 2: Website" }));
  await select(user, "deck-A");
  expect(
    screen
      .getByRole("button", { name: "Dial 1: System volume" })
      .getAttribute("aria-pressed"),
  ).toBe("true");
  await user.click(screen.getByRole("button", { name: "Key 1: Counter" }));
  expect(
    (screen.getByRole("textbox", { name: "Title" }) as HTMLTextAreaElement)
      .value,
  ).toBe("Device A draft");
  await select(user, "deck-B");
  expect(
    screen
      .getByRole("button", { name: "Key 2: Website" })
      .getAttribute("aria-pressed"),
  ).toBe("true");
  expect(state!.drafts["deck-A:fixture-0-Keypad"].title).toBe("Device A draft");
  expect(state!.selections["deck-B"]).toEqual({
    controller: "Keypad",
    index: 1,
  });
  expect(
    request.mock.calls
      .filter(([method]) => method === "command")
      .every(([, payload]) => payload.command === "device_select"),
  ).toBe(true);
});
it("keeps both parts of a delayed action save on device A after selecting B", async () => {
  const user = userEvent.setup();
  let release!: () => void;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  beforeCommand = async (payload) => {
    if (payload.command === "appearance") await gate;
  };
  render(<App />);
  await user.type(
    await screen.findByRole("textbox", { name: "Title" }),
    " local",
  );
  const increment = screen.getByRole("spinbutton", { name: "Increment by" });
  await user.clear(increment);
  await user.type(increment, "5");
  await user.click(screen.getByRole("button", { name: "Save changes" }));
  await waitFor(() =>
    expect(request).toHaveBeenCalledWith(
      "command",
      expect.objectContaining({
        command: "appearance",
        device: "deck-A",
        context: "deck-A:fixture-0-Keypad",
      }),
    ),
  );
  await select(user, "deck-B");
  snapshots["deck-A"].slots[0].settings.count = 44;
  await act(async () => release());
  await waitFor(() =>
    expect(request).toHaveBeenCalledWith("command", {
      command: "settings_patch",
      history_group: expect.any(String),
      device: "deck-A",
      context: "deck-A:fixture-0-Keypad",
      base: { count: 42, incrementBy: 1 },
      settings: { count: 42, incrementBy: 5 },
    }),
  );
  expect(
    (
      screen.getByRole("combobox", {
        name: "Selected device",
      }) as HTMLSelectElement
    ).value,
  ).toBe("deck-B");
  expect(snapshots["deck-B"].slots[0].settings.incrementBy).toBe(1);
  expect(snapshots["deck-A"].slots[0].settings.incrementBy).toBe(5);
  expect(snapshots["deck-A"].slots[0].settings.count).toBe(44);
});
it("keeps Smart Profile settings attached to the device that opened the dialog", async () => {
  snapshots["deck-A"].smart_profiles = {
    config: { enabled: false, fallback: null, bindings: {} },
    backend: "KDE KWin",
    observing: false,
    paused: true,
    focus: null,
    recent: [],
    error: null,
  };
  const user = userEvent.setup();
  render(<App />);
  await screen.findByRole("textbox", { name: "Title" });
  await user.click(screen.getByText("Profile options"));
  await user.click(screen.getByRole("button", { name: "Smart Profiles…" }));
  await user.click(
    screen.getByRole("checkbox", { name: "Switch profiles automatically" }),
  );
  await switchDevice("deck-B");
  await user.click(screen.getByRole("button", { name: "Save Smart Profiles" }));
  await waitFor(() =>
    expect(request).toHaveBeenCalledWith("command", {
      command: "smart_config",
      device: "deck-A",
      expected: { enabled: false, fallback: null, bindings: {} },
      settings: { enabled: true, fallback: null, bindings: {} },
    }),
  );
});
it("drops an action on the hit-tested control and captures its device/page before waiting", async () => {
  const user = userEvent.setup();
  let release!: () => void;
  beforeCommand = async (payload) => {
    if (payload.command === "slot_import")
      await new Promise<void>((resolve) => {
        release = resolve;
      });
  };
  render(<App />);
  await screen.findByRole("textbox", { name: "Title" });
  const target = document.querySelectorAll(".deck-key")[1],
    original = snapshots["deck-A"];
  await nativeFileDrop(
    target.querySelector("img") ?? target,
    "action",
    "enter",
  );
  expect(target.classList.contains("file-drop-target")).toBe(true);
  await nativeFileDrop(target);
  await waitFor(() => expect(release).toBeTypeOf("function"));
  expect(request).toHaveBeenCalledWith("action_file_prepare", {
    device: "deck-A", path: "/tmp/shared.streamDeckAction",
    destination: {
      controller: "Keypad", index: 1,
      expected_target: original.slots.find((s) => s.controller === "Keypad" && s.index === 1)?.context ?? null,
      expected_profile: original.library.active_profile, expected_page: original.library.active_page,
    },
  });
  await select(user, "deck-B");
  await act(async () => release());
  await screen.findByText("Action imported");
  expect(
    (
      screen.getByRole("combobox", {
        name: "Selected device",
      }) as HTMLSelectElement
    ).value,
  ).toBe("deck-B");
});
it("blocks a drop over unsaved action edits but opens profile files without overwriting them", async () => {
  const user = userEvent.setup();
  render(<App />);
  await user.type(
    await screen.findByRole("textbox", { name: "Title" }),
    " edited",
  );
  const target = document.querySelectorAll(".deck-key")[0];
  await nativeFileDrop(target);
  expect(
    await screen.findByText(
      "Save or revert changes before replacing this action.",
    ),
  ).toBeTruthy();
  expect(
    request.mock.calls.filter(
      ([method, payload]) =>
        (method === "command" && payload.command === "slot_import") || method === "action_file_prepare",
    ),
  ).toHaveLength(0);
  await nativeFileDrop(target, "profile");
  expect(request).toHaveBeenCalledWith("open_file", {
    path: "/tmp/profile.streamDeckProfile",
    device: "deck-A",
  });
  expect(
    (screen.getByRole("textbox", { name: "Title" }) as HTMLInputElement).value,
  ).toContain(" edited");
});
it("rejects readonly control drops while allowing background action-file clipboard loading", async () => {
  const profile = snapshots["deck-A"].library;
  profile.profiles.find((p) => p.id === profile.active_profile)!.readonly =
    true;
  render(<App />);
  await screen.findByRole("textbox", { name: "Title" });
  await nativeFileDrop(document.querySelector(".deck-key"));
  expect(
    await screen.findByText("Cannot drop an action into a read-only profile."),
  ).toBeTruthy();
  await nativeFileDrop(null);
  expect(request).toHaveBeenCalledWith("open_file", {
    path: "/tmp/shared.streamDeckAction",
    device: "deck-A",
  });
  expect(
    request.mock.calls.filter(
      ([method, payload]) =>
        (method === "command" && payload.command === "slot_import") || method === "action_file_prepare",
    ),
  ).toHaveLength(0);
});
it("shows new desktop-file results instead of retaining an older success notice", async () => {
  snapshots["deck-A"].file_open = {
    id: "one",
    error: false,
    message: "Action copied from file",
  };
  render(<App />);
  expect(await screen.findByText("Action copied from file")).toBeTruthy();
  snapshots["deck-A"].file_open = {
    id: "two",
    error: true,
    message: "Could not open file: invalid archive",
  };
  await switchDevice("deck-A");
  expect(
    await screen.findByText("Could not open file: invalid archive"),
  ).toBeTruthy();
  expect(screen.queryByText("Action copied from file")).toBeNull();
});
it("blocks pasting a nested cut while its owning container has unsaved edits", async () => {
  const user = userEvent.setup();
  const source = snapshots["deck-A"].slots[0];
  snapshots["deck-A"].clipboard = {
    context: "nested-child",
    root_context: source.context,
    uuid: source.uuid,
    name: source.name,
    controller: source.controller,
    mode: "cut",
  };
  render(<App />);
  const title = await screen.findByRole("textbox", { name: "Title" });
  await user.type(title, " edited");
  await user.click(
    document.querySelectorAll<HTMLButtonElement>(".deck-key")[1],
  );
  await user.keyboard("{Control>}v{/Control}");
  expect(
    await screen.findByText(
      "Save or revert changes before moving the cut action.",
    ),
  ).toBeTruthy();
  expect(
    request.mock.calls.filter(
      ([method, payload]) =>
        method === "command" && payload.command === "slot_paste",
    ),
  ).toHaveLength(0);
});
it("finishing a paste does not undo a newer canvas selection", async () => {
  const user = userEvent.setup();
  const source = snapshots["deck-A"].slots[0];
  snapshots["deck-A"].clipboard = {
    context: source.context,
    uuid: source.uuid,
    name: source.name,
    controller: source.controller,
    mode: "copy",
  };
  let release!: () => void;
  beforeCommand = async (payload) => {
    if (payload.command === "slot_paste")
      await new Promise<void>((resolve) => {
        release = resolve;
      });
  };
  render(<App />);
  await screen.findByRole("textbox", { name: "Title" });
  const keys = document.querySelectorAll<HTMLButtonElement>(".deck-key");
  await user.click(keys[0]);
  await user.keyboard("{Control>}v{/Control}");
  await waitFor(() => expect(release).toBeTypeOf("function"));
  await user.click(keys[1]);
  await act(async () => release());
  await screen.findByText(`Pasted ${source.name}`);
  expect(keys[1].getAttribute("aria-pressed")).toBe("true");
  expect(keys[0].getAttribute("aria-pressed")).toBe("false");
});
it("canceling action sharing leaves the host unchanged", async () => {
  const user = userEvent.setup();
  render(<App />);
  await screen.findByRole("textbox", { name: "Title" });
  for (const name of ["Export Action…", "Import Action…"]) {
    await user.click(screen.getByLabelText("Edit selected control"));
    await user.click(screen.getByRole("button", { name }));
  }
  expect(
    request.mock.calls.filter(
      ([method, payload]) =>
        (method === "command" && ["slot_import", "slot_export"].includes(payload.command)) || method === "action_file_prepare",
    ),
  ).toHaveLength(0);
});
it("captures profile-export ownership before waiting for a native file picker", async () => {
  const user = userEvent.setup();
  let resolvePath!: (value: string) => void;
  pickFile = () =>
    new Promise((resolve) => {
      resolvePath = resolve;
    });
  render(<App />);
  await screen.findByRole("textbox", { name: "Title" });
  await user.click(screen.getByRole("button", { name: "↑ Export profile" }));
  await waitFor(() =>
    expect(request).toHaveBeenCalledWith("pick_file", {
      kind: "export_profile",
    }),
  );
  await select(user, "deck-B");
  await act(async () => resolvePath("/tmp/deck-A.streamDeckProfile"));
  await waitFor(() =>
    expect(request).toHaveBeenCalledWith("command", {
      command: "export_profile",
      device: "deck-A",
      path: "/tmp/deck-A.streamDeckProfile",
    }),
  );
});
it.each(["import", "export"])(
  "keeps action %s on its captured device and page while the picker is open",
  async (mode) => {
    const user = userEvent.setup();
    let resolvePath!: (value: string) => void;
    pickFile = () =>
      new Promise((resolve) => {
        resolvePath = resolve;
      });
    render(<App />);
    await screen.findByRole("textbox", { name: "Title" });
    const original = snapshots["deck-A"];
    await user.click(screen.getByLabelText("Edit selected control"));
    await user.click(
      screen.getByRole("button", {
        name: mode === "import" ? "Import Action…" : "Export Action…",
      }),
    );
    await waitFor(() =>
      expect(request).toHaveBeenCalledWith("pick_file", {
        kind: mode === "import" ? "action" : "export_action",
      }),
    );
    await select(user, "deck-B");
    await act(async () => resolvePath("/tmp/shared.streamDeckAction"));
    await waitFor(() =>
      expect(request).toHaveBeenCalledWith(mode === "import" ? "action_file_prepare" : "command", mode === "export" ? {
        command: "slot_export", device: "deck-A", path: "/tmp/shared.streamDeckAction", context: "deck-A:fixture-0-Keypad",
      } : {
        device: "deck-A", path: "/tmp/shared.streamDeckAction",
        destination: {
          controller: "Keypad", index: 0, expected_target: "deck-A:fixture-0-Keypad",
          expected_profile: original.library.active_profile, expected_page: original.library.active_page,
        },
      }),
    );
  },
);
it("retains the profile-dialog owner when the backend selection changes externally", async () => {
  const user = userEvent.setup();
  render(<App />);
  await user.click(await screen.findByRole("button", { name: "New profile" }));
  const name = screen.getByRole("textbox", { name: "Name" });
  await user.clear(name);
  await user.type(name, "A-only profile");
  await switchDevice("deck-B");
  await user.click(screen.getByRole("button", { name: /^Save$/ }));
  await waitFor(() =>
    expect(request).toHaveBeenCalledWith("command", {
      command: "profile_create",
      device: "deck-A",
      id: undefined,
      name: "A-only profile",
    }),
  );
  expect(
    snapshots["deck-A"].library.profiles.some(
      (profile) => profile.name === "A-only profile",
    ),
  ).toBe(true);
  expect(
    snapshots["deck-B"].library.profiles.some(
      (profile) => profile.name === "A-only profile",
    ),
  ).toBe(false);
});

it("copies a captured source profile to an eligible offline device without switching the editor", async () => {
  copyTargets.add("deck-B");
  const before = structuredClone(snapshots["deck-A"].library);
  const user = userEvent.setup();
  render(<App />);
  await screen.findByRole("textbox", { name: "Title" });
  await user.click(screen.getByText("Profile options"));
  await user.click(screen.getByRole("button", { name: "Copy to device…" }));
  const target = screen.getByRole("combobox", { name: "Target device" });
  expect((target as HTMLSelectElement).value).toBe("deck-B");
  expect(
    screen.getAllByRole("option", { name: /Stream Deck Mini.*Offline/ }),
  ).toHaveLength(2);
  const name = screen.getByRole("textbox", { name: /^Copy name/ });
  await user.clear(name);
  await user.type(name, "On the second deck");
  await user.click(screen.getByRole("button", { name: "Copy profile" }));
  await waitFor(() =>
    expect(request).toHaveBeenCalledWith("command", {
      command: "profile_copy_to_device",
      device: "deck-A",
      profile: before.active_profile,
      target_device: "deck-B",
      name: "On the second deck",
    }),
  );
  expect(selected).toBe("deck-A");
  expect(snapshots["deck-A"].library).toEqual(before);
  expect(snapshots["deck-B"].library.active_profile).toBe("copy:deck-A:deck-B");
  expect(
    request.mock.calls.some(
      ([, payload]) => payload?.command === "device_select",
    ),
  ).toBe(false);
});
it("does not offer missing compatibility metadata or incompatible destinations", async () => {
  const user = userEvent.setup();
  render(<App />);
  await screen.findByRole("textbox", { name: "Title" });
  await user.click(screen.getByText("Profile options"));
  await user.click(screen.getByRole("button", { name: "Copy to device…" }));
  expect(
    screen.getByText(
      "No other compatible devices are associated with this profile.",
    ),
  ).toBeTruthy();
  expect(
    (screen.getByRole("button", { name: "Copy profile" }) as HTMLButtonElement)
      .disabled,
  ).toBe(true);
  expect(
    screen
      .getByRole("combobox", { name: "Target device" })
      .querySelectorAll("option"),
  ).toHaveLength(1);
  expect(
    request.mock.calls.some(
      ([, payload]) => payload?.command === "profile_copy_to_device",
    ),
  ).toBe(false);
});
it("retains source and eligibility captured before an external selection change", async () => {
  copyTargets.add("deck-B");
  const user = userEvent.setup();
  render(<App />);
  await screen.findByRole("textbox", { name: "Title" });
  await user.click(screen.getByText("Profile options"));
  await user.click(screen.getByRole("button", { name: "Copy to device…" }));
  const source = snapshots["deck-A"].library.active_profile;
  await switchDevice("deck-B");
  await user.clear(screen.getByRole("textbox", { name: /^Copy name/ }));
  await user.click(screen.getByRole("button", { name: "Copy profile" }));
  await waitFor(() =>
    expect(request).toHaveBeenCalledWith("command", {
      command: "profile_copy_to_device",
      device: "deck-A",
      profile: source,
      target_device: "deck-B",
    }),
  );
  expect(selected).toBe("deck-B");
});

it("keeps the background picker bound to its original device, profile and page", async () => {
  const user = userEvent.setup();
  let resolvePath!: (value: string) => void;
  pickFile = () =>
    new Promise((resolve) => {
      resolvePath = resolve;
    });
  const owner = snapshots["deck-A"].library;
  const profile = owner.active_profile,
    id = owner.active_page;
  render(<App />);
  await screen.findByRole("textbox", { name: "Title" });
  await user.click(screen.getByLabelText("Page options"));
  await user.click(
    screen.getByRole("button", { name: "Set display background…" }),
  );
  await waitFor(() =>
    expect(request).toHaveBeenCalledWith("pick_file", { kind: "image" }),
  );
  await select(user, "deck-B");
  await act(async () => resolvePath("/tmp/chosen.png"));
  await waitFor(() =>
    expect(request).toHaveBeenCalledWith("command", {
      command: "page_background",
      device: "deck-A",
      profile,
      id,
      path: "/tmp/chosen.png",
    }),
  );
  expect(
    screen.queryByRole("button", { name: "Set display background…" }),
  ).toBeNull();
});

it("routes undo/redo with captured revision and device while leaving text undo alone", async () => {
  snapshots["deck-A"].history = { revision: 9, undo: "Edit action", redo: "Remove action", note: null };
  render(<App />); const user = userEvent.setup();
  await user.click(await screen.findByRole("button", { name: "Undo" }));
  expect(request).toHaveBeenCalledWith("command", { command: "history_undo", expected_revision: 9, device: "deck-A" });
  await user.click(screen.getByRole("button", { name: "Redo" }));
  expect(request).toHaveBeenCalledWith("command", { command: "history_redo", expected_revision: 9, device: "deck-A" });
  const title = screen.getByRole("textbox", { name: "Title" });
  await user.click(title); const count = request.mock.calls.filter(([m,p]) => m === "command" && p.command.startsWith("history_")).length;
  await user.keyboard("{Control>}z{/Control}");
  expect(request.mock.calls.filter(([m,p]) => m === "command" && p.command.startsWith("history_")).length).toBe(count);
  await user.type(title, " pending");
  expect((screen.getByRole("button", { name: "Undo" }) as HTMLButtonElement).disabled).toBe(true);
  expect((screen.getByRole("button", { name: "Redo" }) as HTMLButtonElement).disabled).toBe(true);
});


it("allows undo to restore an action whose retained dirty draft is no longer in the graph", async () => {
  snapshots["deck-A"].history = { revision: 2, undo: "Remove action", redo: null, note: null,
    contexts: snapshots["deck-A"].slots.map((s) => s.context) };
  render(<App />); const user = userEvent.setup();
  const title = await screen.findByRole("textbox", { name: "Title" });
  await user.type(title, " pending");
  expect((screen.getByRole("button", { name: "Undo" }) as HTMLButtonElement).disabled).toBe(true);
  snapshots["deck-A"].slots = [];
  snapshots["deck-A"].history!.contexts = [];
  await act(async () => listener?.(view()));
  expect((screen.getByRole("button", { name: "Undo" }) as HTMLButtonElement).disabled).toBe(false);
  await user.click(screen.getByRole("button", { name: "Undo" }));
  expect(request).toHaveBeenCalledWith("command", { command: "history_undo", expected_revision: 2, device: "deck-A" });
});


it("routes package drops globally even over a control, preserving unsaved action drafts", async () => {
  render(<App />); const user = userEvent.setup();
  const title = await screen.findByRole("textbox", { name: "Title" });
  await user.clear(title); await user.type(title, "Keep pending title");
  const key = screen.getByRole("button", { name: "Key 2: Website" });
  for (const [kind, path] of [["plugin", "/tmp/package.streamDeckPlugin"], ["icon_pack", "/tmp/icons.streamDeckIconPack"]]) {
    await nativeFileDrop(key, kind);
    await waitFor(() => expect(request).toHaveBeenCalledWith("open_file", { path, device: "deck-A" }));
    expect((screen.getByRole("textbox", { name: "Title" }) as HTMLTextAreaElement).value).toBe("Keep pending title");
  }
  expect(request.mock.calls.some(([m,p]) => (m === "command" && p.command === "slot_import") || m === "action_file_prepare")).toBe(false);
});


it("opens pack management without assigning an action to an empty profile", async () => {
  snapshots["deck-A"].slots = [];
  render(<App />); const user = userEvent.setup();
  await user.click(await screen.findByRole("button", { name: "Manage icon packs…" }));
  expect(screen.getByRole("dialog", { name: "Icon library" })).toBeTruthy();
  expect(screen.getByText(/Manage packs here/)).toBeTruthy();
  expect(request.mock.calls.some(([m,p]) => m === "command" && p.command === "assign")).toBe(false);
});


it("retains a pending file choice across native view recreation and uses its captured device", async () => {
  const original = snapshots["deck-A"];
  pendingActionFile = { id: "pending-file", device: "deck-A", destination: { controller: "Keypad", index: 1, expected_target: null, expected_profile: original.library.active_profile, expected_page: original.library.active_page }, info: { path: "/multiple.streamDeckAction", source_hash: "contents", roots: [
    { index: 0, name: "First root", uuid: "unknown.key", controller: "Keypad", page: "page", column: 0, row: 0 },
    { index: 1, name: "Second root", uuid: "unknown.key", controller: "Keypad", page: "page", column: 1, row: 0 },
  ] } };
  let app = render(<App />); const user = userEvent.setup();
  await user.type(await screen.findByLabelText("Find action"), "Second");
  await waitFor(() => expect(state?.actionFileChoice).toEqual({ id: "pending-file", root: 1, query: "Second" }));
  app.unmount(); selected = "deck-B"; app = render(<App />);
  const search = await screen.findByLabelText("Find action");
  expect((search as HTMLInputElement).value).toBe("Second");
  expect((screen.getByLabelText("Action in file") as HTMLSelectElement).value).toBe("1");
  expect(screen.getByText(/Key 2 on Stream Deck \+/)).toBeTruthy();
  const importAction = await screen.findByRole("button", { name: "Import action" });
  await waitFor(() => expect((importAction as HTMLButtonElement).disabled).toBe(false));
  await user.click(importAction);
  expect(request).toHaveBeenCalledWith("action_file_choose", { id: "pending-file", root: 1 });
});
