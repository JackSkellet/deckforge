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
function view() {
  const current = structuredClone(snapshots[selected]);
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
    if (method === "fonts") return [];
    if (method === "pick_file") return pickFile ? pickFile() : null;
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
