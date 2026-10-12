import { mergeSettingsEdit } from "./settings";
import { newId } from "./ids";
// Explicit development-only bridge. Never imported in a production bundle.
import type { Snapshot, Action, Slot, RecordValue } from "./model";
import { sameMixerValue } from "./Mixer";
import { fixtureDevice } from "./fixtureDevices";
import { validControl } from "./device";
// Share the production edit merge with the development bridge.
export const applyFixtureSettingsPatch = mergeSettingsEdit;
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
      plugin:
        category === "Discord"
          ? "discord"
          : uuid.startsWith("com.elgato.counter.")
            ? "sample"
            : "builtin",
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
    state_count: 1,
    appearance: {
      title_parameters: {
        fontFamily: "",
        fontSize: 12,
        fontStyle: "Regular",
        fontUnderline: false,
        showTitle: true,
        titleAlignment: "bottom",
        titleColor: "#ffffff",
      },
      title_custom: false,
      image_custom: false,
    },
    raw: { ActionID: `fixture-${index}`, UUID: id, Settings: {}, Actions: [] },
    has_inspector: !id.includes("system"),
    multi_running: false,
    stack_children: [],
    trigger_children: [],
    multi_children: [],
  };
}
export function installFixture(modelId?: string) {
  const model = fixtureDevice(
    modelId ?? new URLSearchParams(location.search).get("model") ?? "plus",
  );
  let listener: ((s: Snapshot) => void) | undefined;
  let actionClipboard: { slot: Slot; mode: "copy" | "cut" } | null = null;
  let uiState: unknown = null;
  const snapshot: Snapshot = {
    running: true,
    device_model: model.id,
    device_name: model.name,
    device_geometry: model.geometry,
    device_capabilities: model.capabilities,
    device: `simulated-streamdeck-${model.id}`,
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
        removal_mode: "detach",
      },
      {
        id: "discord",
        name: "Discord for Deckforge",
        version: "0.1.0",
        connected: true,
        status: "connected",
        path: "/fixture/plugins/discord",
        removal_mode: "detach",
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
      supported: model.id === "plus",
      active: false,
      preferences: { after_minutes: 0, disable_on_lock: true, image: null },
    },
    mixer: {
      config: {
        enabled: false,
        channels: [],
        mixes: [
          {
            id: "monitor",
            name: "Monitor",
            outputs: [],
            volume: 100,
            muted: false,
          },
          {
            id: "stream",
            name: "Stream",
            outputs: [],
            volume: 100,
            muted: false,
          },
        ],
        routes: [],
      },
      connected: true,
      active: false,
      sinks: [
        {
          name: "fixture.speakers",
          description: "Desktop speakers",
          owned: false,
          monitor: false,
        },
        {
          name: "fixture.headphones",
          description: "Headphones",
          owned: false,
          monitor: false,
        },
      ],
      sources: [
        {
          name: "fixture.microphone",
          description: "USB microphone",
          owned: false,
          monitor: false,
        },
      ],
      apps: [
        {
          index: 1,
          name: "Music player",
          application_id: "org.example.Music",
          process_binary: "music-player",
          sink: "fixture.speakers",
        },
        {
          index: 2,
          name: "Voice chat",
          application_id: "org.example.Chat",
          process_binary: "voice-chat",
          sink: "fixture.headphones",
        },
      ],
      warnings: [],
      error: null,
    },
    audio_outputs: [["speakers", "Desktop speakers"]],
    audio_active: {},
    multi_runs: [],
    logs: [
      "Development fixture. No real hardware, files or plugins are accessed.",
    ],
  };
  if (new URLSearchParams(location.search).get("overflow") !== "1")
    snapshot.slots = snapshot.slots.filter((slot) =>
      validControl(snapshot, slot.controller, slot.index),
    );
  if (model.geometry.keys > 8)
    snapshot.slots.push(
      assigned(
        "com.elgato.counter.action",
        model.geometry.keys - 1,
        `Key ${model.geometry.keys}`,
        String(model.geometry.keys),
      ),
    );
  if (model.geometry.dials > 4)
    snapshot.slots.push(
      assigned(
        "com.elgato.streamdeck.system.multimedia",
        model.geometry.dials - 1,
        `Dial ${model.geometry.dials}`,
        "65%",
      ),
    );
  if (model.id === "neo") {
    snapshot.actions = [
      ...snapshot.actions,
      {
        uuid: "fixture.neo.infobar",
        name: "Profile infobar",
        plugin: "sample",
        plugin_name: "Fixture",
        category: "Infobar",
        controllers: ["Neo"],
        visible: true,
        multi_only: false,
        supported_in_multi: false,
        defaults: {},
        states: [],
      },
    ];
    snapshot.slots.push({
      ...assigned("com.elgato.counter.action", 1, "Infobar", "MAIN"),
      controller: "Neo",
      context: "fixture-infobar",
      uuid: "fixture.neo.infobar",
      name: "Profile infobar",
      has_inspector: true,
    });
  }
  for (const profile of snapshot.library.profiles) {
    profile.device_model = model.id;
    profile.device_name = model.name;
    profile.compatible = true;
  }
  function refreshNavigation() {
    const profile = snapshot.library.profiles.find(
      (p) => p.id === snapshot.library.active_profile,
    );
    const index =
      profile?.page_order.indexOf(snapshot.library.active_page) ?? -1;
    snapshot.page_navigation = {
      previous: index > 0,
      next: index >= 0 && index < (profile?.page_order.length ?? 0) - 1,
    };
  }
  refreshNavigation();
  function transfer(
    source: Slot,
    target: { controller: string; index: number },
    copy: boolean,
    expected: string | null,
  ) {
    if (
      !validControl(
        snapshot,
        target.controller as Slot["controller"],
        target.index,
      )
    )
      throw new Error("Control outside device geometry");
    const destination = snapshot.slots.find(
      (s) => s.controller === target.controller && s.index === target.index,
    );
    if ((destination?.context ?? null) !== expected)
      throw new Error(
        "The destination changed. Select it again before pasting.",
      );
    if (source.context === destination?.context) return;
    const metadata = actions.find((a) => a.uuid === source.uuid);
    if (metadata && !metadata.controllers.includes(target.controller as any))
      throw new Error("Action does not support that control");
    const reverse = actions.find((a) => a.uuid === destination?.uuid);
    if (
      !copy &&
      destination &&
      reverse &&
      !reverse.controllers.includes(source.controller)
    )
      throw new Error("These actions cannot swap");
    const result = structuredClone(source);
    result.controller = target.controller as Slot["controller"];
    result.index = target.index;
    if (copy) {
      result.context = newId();
      result.raw.ActionID = result.context;
    }
    snapshot.slots = snapshot.slots.filter(
      (s) =>
        s.context !== destination?.context &&
        (copy || s.context !== source.context),
    );
    if (!copy && destination) {
      destination.controller = source.controller;
      destination.index = source.index;
      snapshot.slots.push(destination);
    }
    snapshot.slots.push(result);
  }
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
      if (method === "fonts") return [] as T;
      if (method === "appearance_preview")
        throw new Error(
          "Use the native desktop app for Rust-rendered appearance previews. The synthetic fixture does not render fonts or icons.",
        );
      if (method === "startup_status" || method === "startup_set")
        return { enabled: payload?.enabled ?? false, this_session: true } as T;
      if (method === "pick_file") return null as T;
      if (method === "command") {
        if (payload.command === "slot_copy" || payload.command === "slot_cut") {
          const source = snapshot.slots.find(
            (s) => s.context === payload.context,
          );
          if (!source) throw new Error("Unknown assignable action");
          const mode = payload.command === "slot_copy" ? "copy" : "cut";
          actionClipboard = { slot: structuredClone(source), mode };
          snapshot.clipboard = {
            context: source.context,
            name: source.name,
            uuid: source.uuid,
            controller: source.controller,
            mode,
          };
        }
        if (payload.command === "slot_clipboard_clear") {
          actionClipboard = null;
          snapshot.clipboard = null;
        }
        if (payload.command === "slot_paste") {
          if (!actionClipboard) throw new Error("Action clipboard is empty");
          const source =
            actionClipboard.mode === "cut"
              ? snapshot.slots.find(
                  (s) => s.context === actionClipboard!.slot.context,
                )
              : actionClipboard.slot;
          if (!source) throw new Error("Cut source is unavailable");
          transfer(
            source,
            payload,
            actionClipboard.mode === "copy",
            payload.expected_target,
          );
          if (actionClipboard.mode === "cut") {
            actionClipboard = null;
            snapshot.clipboard = null;
          }
        }
        if (payload.command === "slot_transfer") {
          const source = snapshot.slots.find(
            (s) => s.context === payload.context,
          );
          if (!source) throw new Error("Unknown source action");
          transfer(source, payload, payload.copy, payload.expected_target);
        }
        if (payload.command === "uninstall_plugin") {
          const plugin = snapshot.plugins.find(
            (plugin) => plugin.id === payload.id,
          );
          if (
            !plugin ||
            payload.confirm !== true ||
            payload.expected_path !== plugin.path ||
            payload.expected_enabled !== (plugin.enabled !== false)
          )
            throw new Error("Plugin state changed; review removal again");
          const removed = new Set(
            snapshot.actions
              .filter((action) => action.plugin === plugin.id)
              .map((action) => action.uuid),
          );
          snapshot.plugins = snapshot.plugins.filter(
            (candidate) => candidate.id !== plugin.id,
          );
          snapshot.actions = snapshot.actions.filter(
            (action) => !removed.has(action.uuid),
          );
          snapshot.slots = snapshot.slots.map((slot) =>
            removed.has(slot.uuid)
              ? {
                  ...slot,
                  available: false,
                  plugin_disabled: false,
                  has_inspector: false,
                }
              : slot,
          );
        }
        if (payload.command === "plugin_enabled") {
          const plugin = snapshot.plugins.find(
            (plugin) => plugin.id === payload.id,
          );
          if (!plugin) throw new Error("Plugin not installed");
          if (payload.expected_enabled !== (plugin.enabled !== false))
            throw new Error("Plugin state changed; refresh and retry");
          plugin.enabled = payload.enabled;
          plugin.connected = payload.enabled;
          plugin.status = payload.enabled ? "connected" : "disabled";
          snapshot.actions = snapshot.actions.map((action) =>
            action.plugin === plugin.id
              ? { ...action, enabled: payload.enabled }
              : action,
          );
          snapshot.slots = snapshot.slots.map((slot) =>
            snapshot.actions.find((action) => action.uuid === slot.uuid)
              ?.plugin === plugin.id
              ? {
                  ...slot,
                  plugin_disabled: !payload.enabled,
                  available: payload.enabled,
                }
              : slot,
          );
        }
        if (payload.command === "mixer_config") {
          if (!sameMixerValue(snapshot.mixer.config, payload.expected))
            throw new Error(
              "Mixer settings changed outside this editor. Reload current settings before applying.",
            );
          snapshot.mixer.config = structuredClone(payload.settings);
          snapshot.mixer.applied_config = structuredClone(payload.settings);
          snapshot.mixer.active = payload.settings.enabled;
          snapshot.mixer.pending = false;
        }
        // Inventory refresh is read-only in this explicit synthetic fixture.
        if (payload.command === "mixer_refresh") {
          listener?.(structuredClone(snapshot));
          return null as T;
        }
        const slot = snapshot.slots.find((s) => s.context === payload.context);
        if (payload.command === "settings" && slot)
          slot.settings = payload.settings;
        if (payload.command === "settings_patch" && slot)
          slot.settings = applyFixtureSettingsPatch(
            payload.base,
            payload.settings,
            slot.settings,
          );
        if (payload.command === "appearance" && slot) {
          if (
            payload.expected_state !== undefined &&
            payload.expected_state !== slot.state
          )
            throw new Error("Action state changed");
          if (payload.state !== undefined && payload.state !== slot.state)
            throw new Error(
              "The synthetic fixture only models its current appearance state",
            );
          if (payload.title !== undefined) {
            slot.title = payload.title;
            slot.appearance!.title_custom = !!payload.title;
          }
          if (payload.reset_title) {
            slot.title = slot.name;
            slot.appearance!.title_custom = false;
          }
          if (payload.title_parameters)
            slot.appearance!.title_parameters = {
              ...slot.appearance!.title_parameters,
              ...payload.title_parameters,
            };
          if (payload.image_path) slot.appearance!.image_custom = true;
          if (payload.reset_image) slot.appearance!.image_custom = false;
        }
        if (payload.command === "assign") {
          if (!validControl(snapshot, payload.controller, payload.index))
            throw new Error("Control outside device geometry");
          if (
            snapshot.actions.find((action) => action.uuid === payload.uuid)
              ?.enabled === false
          )
            throw new Error("Plugin is disabled");
          snapshot.slots = snapshot.slots.filter(
            (s) =>
              !(
                s.controller === payload.controller && s.index === payload.index
              ),
          );
          const a = snapshot.actions.find((a) => a.uuid === payload.uuid)!;
          if (a.controllers.includes("Neo")) {
            snapshot.slots.push({
              ...assigned(
                "com.elgato.counter.action",
                payload.index,
                a.name,
                a.name.slice(0, 1),
              ),
              context: newId(),
              uuid: a.uuid,
              name: a.name,
              controller: payload.controller,
            });
          } else
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
        if (payload.command === "page_step") {
          const profile = snapshot.library.profiles.find(
            (p) => p.id === snapshot.library.active_profile,
          )!;
          const old = profile.page_order.indexOf(snapshot.library.active_page);
          if (old >= 0) {
            const next = Math.max(
              0,
              Math.min(
                profile.page_order.length - 1,
                old + Math.sign(payload.direction),
              ),
            );
            snapshot.library.active_page = profile.page_order[next];
          }
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
        refreshNavigation();
        listener?.(structuredClone(snapshot));
        return null as T;
      }
      throw new Error(`Development fixture does not implement ${method}`);
    },
  };
}
