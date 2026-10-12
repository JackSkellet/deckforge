// Explicit development-only bridge. Never imported in a production bundle.
import type { Snapshot, Action, Slot, RecordValue } from "./model";
const icon = (label: string, mark: string, color = "#96dfc4") =>
  "data:image/svg+xml;base64," +
  btoa(
    unescape(
      encodeURIComponent(
        `<svg xmlns="http://www.w3.org/2000/svg" width="120" height="120" viewBox="0 0 120 120"><rect width="120" height="120" rx="8" fill="#142027"/><text x="60" y="62" text-anchor="middle" fill="${color}" font-size="36" font-family="sans-serif">${mark}</text><text x="60" y="100" text-anchor="middle" fill="#d4e6ef" font-size="12" font-family="sans-serif">${label}</text></svg>`,
      ),
    ),
  );
const definitions: [string, string, string, "Keypad" | "Encoder"][] = [
  ["com.elgato.counter.action", "Counter", "Utilities", "Keypad"],
  ["com.elgato.streamdeck.system.website", "Website", "System", "Keypad"],
  ["com.elgato.streamdeck.system.open", "Open application", "System", "Keypad"],
  ["com.elgato.streamdeck.system.hotkey", "Hotkey", "System", "Keypad"],
  [
    "com.elgato.streamdeck.system.multimedia",
    "System volume",
    "System",
    "Encoder",
  ],
  [
    "com.elgato.streamdeck.soundboard.playaudio",
    "Play audio",
    "Soundboard",
    "Keypad",
  ],
  [
    "com.elgato.streamdeck.soundboard.stopaudioplay",
    "Stop audio",
    "Soundboard",
    "Keypad",
  ],
  [
    "com.elgato.streamdeck.multiactions.routine",
    "Multi Action",
    "Navigation & sequences",
    "Keypad",
  ],
  [
    "com.elgato.streamdeck.multiactions.routine2",
    "Multi Action Switch",
    "Navigation & sequences",
    "Keypad",
  ],
  [
    "com.elgato.streamdeck.multiactions.random",
    "Random Action",
    "Navigation & sequences",
    "Keypad",
  ],
  [
    "com.elgato.streamdeck.multiactions.delay",
    "Delay",
    "Navigation & sequences",
    "Keypad",
  ],
  [
    "com.elgato.streamdeck.dial.stack",
    "Dial Stack",
    "Navigation & sequences",
    "Encoder",
  ],
  [
    "com.elgato.streamdeck.keys.adaptor",
    "Action Trigger",
    "Navigation & sequences",
    "Encoder",
  ],
  ["discord.mute", "Mute microphone", "Discord", "Keypad"],
  ["discord.deafen", "Deafen", "Discord", "Keypad"],
  [
    "com.elgato.streamdeck.page.next",
    "Next page",
    "Navigation & sequences",
    "Keypad",
  ],
];
const actions = definitions.map(
  ([uuid, name, category, controller]) =>
    ({
      uuid,
      name,
      category,
      controllers: [controller],
      plugin: category === "Discord" ? "discord" : "sample",
      plugin_name:
        category === "Discord"
          ? "Discord for Deckforge"
          : category === "Utilities"
            ? "Counter"
            : "Deckforge",
      visible: true,
      multi_only: uuid.endsWith(".delay"),
      supported_in_multi: !uuid.includes("routine"),
      defaults: uuid.includes("counter")
        ? { count: 0, incrementBy: 1 }
        : uuid.includes("website")
          ? { path: "https://example.com", openInBrowser: true }
          : uuid.includes("multimedia")
            ? { actionIdx: 18, linuxVolumeStep: 5 }
            : {},
      states: [],
    }) as Action,
);
function assigned(
  id: string,
  index: number,
  title: string,
  mark: string,
): Slot {
  const action = actions.find((a) => a.uuid === id)!;
  return {
    context: `fixture-${index}-${action.controllers[0]}`,
    uuid: id,
    name: action.name,
    controller: action.controllers[0],
    index,
    settings: {
      ...action.defaults,
      ...(id.includes("counter") ? { count: 42, incrementBy: 1 } : {}),
    },
    title,
    pinned: false,
    available: true,
    preview: icon(title, mark),
    state: 0,
    raw: { ActionID: `fixture-${index}`, UUID: id, Settings: {}, Actions: [] },
    has_inspector: !id.includes("system"),
    multi_running: false,
    stack_children: [],
    trigger_children: [],
    multi_children: [],
  };
}
export function installFixture() {
  let listener: ((s: Snapshot) => void) | undefined;
  let uiState: unknown = null;
  const snapshot: Snapshot = {
    running: true,
    device: "simulated-streamdeck-plus",
    firmware: "Simulation",
    simulate: true,
    device_online: true,
    connected: false,
    slots: [
      assigned("com.elgato.counter.action", 0, "Counter", "42"),
      assigned("com.elgato.streamdeck.system.website", 1, "Website", "www"),
      assigned("com.elgato.streamdeck.system.hotkey", 2, "Hotkey", "⌘"),
      assigned("discord.mute", 3, "Mic mute", "M"),
      assigned(
        "com.elgato.streamdeck.soundboard.playaudio",
        4,
        "Play audio",
        "▶",
      ),
      assigned("com.elgato.streamdeck.system.open", 5, "Applications", "↗"),
      assigned("com.elgato.streamdeck.multiactions.routine", 6, "Go live", "≡"),
      assigned("discord.deafen", 7, "Deafen", "D"),
      assigned("com.elgato.streamdeck.system.multimedia", 0, "Volume", "65%"),
    ],
    actions,
    plugins: [
      {
        id: "sample",
        name: "Counter",
        version: "1.0.0",
        connected: true,
        status: "connected",
        path: "/fixture/plugins/counter",
      },
      {
        id: "discord",
        name: "Discord for Deckforge",
        version: "0.1.0",
        connected: true,
        status: "connected",
        path: "/fixture/plugins/discord",
      },
    ],
    library: {
      active_profile: "daily",
      active_page: "main",
      folder_depth: 0,
      profiles: [
        {
          id: "daily",
          name: "Daily workspace",
          readonly: false,
          pin_page: "pins",
          page_order: ["main", "creation"],
          pages: [
            { id: "main", name: "Main", folder: false },
            { id: "creation", name: "Creation", folder: false },
          ],
        },
        {
          id: "stream",
          name: "Streaming",
          readonly: false,
          pin_page: "stream-pins",
          page_order: ["stream-main"],
          pages: [{ id: "stream-main", name: "Live controls", folder: false }],
        },
      ],
    },
    device_settings: { desired: { brightness: 80 } },
    screensaver: {
      active: false,
      preferences: { after_minutes: 0, disable_on_lock: true, image: null },
    },
    audio_outputs: [["speakers", "Desktop speakers"]],
    audio_active: {},
    multi_runs: [],
    logs: [
      "Development fixture. No real hardware, files or plugins are accessed.",
    ],
  };
  window.deckforge = {
    onSnapshot(fn) {
      listener = fn;
      return () => {
        listener = undefined;
      };
    },
    async request<T>(method: string, payload: any): Promise<T> {
      if (method === "snapshot") return structuredClone(snapshot) as T;
      if (method === "ui_state_get") return uiState as T;
      if (method === "ui_state_set") {
        uiState = payload.state;
        return null as T;
      }
      if (method === "action_icon") return null as T;
      if (method === "startup_status" || method === "startup_set")
        return { enabled: payload?.enabled ?? false, this_session: true } as T;
      if (method === "pick_file") return null as T;
      if (method === "command") {
        const slot = snapshot.slots.find((s) => s.context === payload.context);
        if (payload.command === "settings" && slot)
          slot.settings = payload.settings;
        if (payload.command === "appearance" && slot)
          slot.title = payload.title;
        if (payload.command === "assign") {
          snapshot.slots = snapshot.slots.filter(
            (s) =>
              !(
                s.controller === payload.controller && s.index === payload.index
              ),
          );
          const a = actions.find((a) => a.uuid === payload.uuid)!;
          snapshot.slots.push(
            assigned(a.uuid, payload.index, a.name, a.name.slice(0, 1)),
          );
        }
        if (payload.command === "profile_select") {
          snapshot.library.active_profile = payload.id;
          snapshot.library.active_page = snapshot.library.profiles.find(
            (p) => p.id === payload.id,
          )!.page_order[0];
        }
        if (payload.command === "page_select")
          snapshot.library.active_page = payload.id;
        if (payload.command === "slot_pin" && slot)
          slot.pinned = payload.pinned;
        if (payload.command === "remove")
          snapshot.slots = snapshot.slots.filter(
            (s) => s.context !== payload.context,
          );
        if (payload.command === "profile_rename") {
          const p = snapshot.library.profiles.find((p) => p.id === payload.id);
          if (p) p.name = payload.name;
        }
        if (
          ["multi_save", "stack_save", "trigger_save"].includes(
            payload.command,
          ) &&
          slot
        )
          slot.raw = payload.raw;
        listener?.(structuredClone(snapshot));
        return null as T;
      }
      throw new Error(`Development fixture does not implement ${method}`);
    },
  };
}
