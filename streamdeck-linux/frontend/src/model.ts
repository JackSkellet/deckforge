import { newId } from "./ids";
import type { DeviceGeometry, DeviceCapabilities } from "./device";
// Vendor action records deliberately remain open: unfamiliar fields must survive editing.
export type RecordValue = Record<string, any>;
export type Controller = "Keypad" | "Encoder" | "Neo";
export interface Action {
  enabled?: boolean;
  user_title_enabled?: boolean;
  uuid: string;
  name: string;
  source_name?: string;
  source_states?: RecordValue[] | null;
  plugin: string;
  plugin_name: string;
  category: string;
  controllers: Controller[];
  visible: boolean;
  multi_only: boolean;
  supported_in_multi: boolean;
  defaults: RecordValue;
  states: RecordValue[];
  icon?: string;
  icon_generation?: number;
  tooltip?: string;
}
export interface TitleParameters extends RecordValue {
  fontFamily?: string;
  fontSize?: number;
  fontStyle?: "Regular" | "Bold" | "Italic" | "Bold Italic" | "";
  fontUnderline?: boolean;
  showTitle?: boolean;
  titleAlignment?: "top" | "middle" | "bottom";
  titleColor?: string;
}
export interface AppearanceInfo {
  title_parameters: TitleParameters;
  title_custom: boolean;
  image_custom: boolean;
  title?: string;
}
export interface AppearanceEdit {
  title?: string;
  reset_title?: boolean;
  title_parameters?: TitleParameters;
  image_path?: string | null;
  reset_image?: boolean;
}
export interface Slot {
  plugin_disabled?: boolean;
  appearance?: AppearanceInfo;
  state_count?: number;
  state_names?: string[];
  context: string;
  uuid: string;
  name: string;
  controller: Controller;
  index: number;
  settings: RecordValue;
  title: string;
  pinned: boolean;
  available: boolean;
  preview?: string;
  software_icon?: string;
  state: number;
  raw: RecordValue;
  has_inspector: boolean;
  multi_running: boolean;
  stack_children: Slot[];
  trigger_children: (Slot | null)[];
  multi_children: Slot[][];
  trigger_description?: RecordValue;
}
export interface Profile {
  compatible?: boolean;
  device_name?: string;
  device_model?: string;
  id: string;
  name: string;
  readonly: boolean;
  pin_page: string;
  page_order: string[];
  pages: { id: string; name: string; folder: boolean }[];
}
export interface Clipboard {
  device?: string;
  name: string;
  uuid: string;
  controller: Controller;
  mode: "copy" | "cut";
  context: string;
}
export interface DeviceRecord {
  profile_compatible?: boolean;
  id: string;
  model: string;
  name: string;
  connected: boolean;
  firmware: string;
  selected: boolean;
  online?: boolean;
}
export interface PluginLanguageState {
  preference: string;
  active: string;
  resolved: string;
  restart_required: boolean;
  options: { value: string; label: string }[];
}
export interface Snapshot {
  marketplace?: MarketplaceState;
  plugin_language?: PluginLanguageState;
  multi_action_feedback?: boolean;
  application_monitor?: { enabled: boolean; error: string | null };
  devices?: DeviceRecord[];
  page_navigation?: { previous: boolean; next: boolean };
  device_model?: string;
  device_name?: string;
  device_geometry?: DeviceGeometry;
  device_model_name?: string;
  device_custom_name?: string | null;
  device_capabilities?: DeviceCapabilities;
  overflow_slots?: RecordValue[];
  clipboard?: Clipboard | null;
  running: boolean;
  device: string;
  firmware: string;
  connected: boolean;
  device_online: boolean;
  simulate: boolean;
  connection_error?: string;
  slots: Slot[];
  actions: Action[];
  plugins: RecordValue[];
  library: {
    profiles: Profile[];
    active_profile: string;
    active_page: string;
    folder_depth: number;
  };
  device_settings: RecordValue;
  screensaver: RecordValue;
  audio_outputs: [string, string][];
  audio_active: RecordValue;
  multi_runs: RecordValue[];
  logs: string[];
  reference_app?: string;
  system_status?: string;
  [key: string]: any;
}

export interface MarketplaceState {
  busy: boolean;
  authenticated: boolean | null;
  account: string | null;
  message: string;
  error: string | null;
  pending: string | null;
  review: import("./PluginUpdate").PluginUpdateReview | null;
}
export interface Draft {
  settings: RecordValue;
  title: string;
  raw: RecordValue;
  dirty: boolean;
  revision: number;
  source: string;
  baseState?: number;
  settingsEdited?: boolean;
  rawEdited?: boolean;
  titleEdited?: boolean;
  appearanceState?: number;
  appearanceExpectedState?: number;
  appearanceEdits?: Record<string, AppearanceEdit>;
  appearanceBases?: Record<string, AppearanceInfo>;
}
export const clone = <T>(value: T): T => structuredClone(value);
export const sourceOf = (slot: Slot) =>
  JSON.stringify([
    slot.settings,
    slot.title,
    slot.raw,
    slot.state,
    slot.appearance,
  ]);
function migrateDraft(existing: Draft, slot: Slot): Draft {
  if (
    existing.settingsEdited !== undefined &&
    existing.titleEdited !== undefined &&
    existing.rawEdited !== undefined
  )
    return existing;
  // Migration compares the old captured baseline, never the plugin's current title.
  let baseline: any[] = [];
  try {
    baseline = JSON.parse(existing.source);
  } catch {
    /* Legacy malformed source is kept as a settings draft. */
  }
  const titleEdited =
    existing.titleEdited ??
    (existing.dirty &&
      typeof baseline[1] === "string" &&
      existing.title !== baseline[1]);
  const target = existing.appearanceState ?? existing.baseState ?? slot.state;
  return {
    ...existing,
    baseState: existing.baseState ?? slot.state,
    titleEdited,
    settingsEdited:
      existing.settingsEdited ??
      (existing.dirty &&
        (baseline[0] === undefined ||
          JSON.stringify(existing.settings) !== JSON.stringify(baseline[0]))),
    rawEdited:
      existing.rawEdited ??
      (existing.dirty &&
        baseline[2] !== undefined &&
        JSON.stringify(existing.raw) !== JSON.stringify(baseline[2])),
    appearanceEdits:
      existing.appearanceEdits ??
      (titleEdited ? { [target]: { title: existing.title } } : {}),
    appearanceExpectedState:
      existing.appearanceExpectedState ??
      (titleEdited ? slot.state : undefined),
  };
}
export function reconcileDraft(existing: Draft | undefined, slot: Slot): Draft {
  if (existing) {
    existing = migrateDraft(existing, slot);
    if (existing.dirty || existing.source === sourceOf(slot)) return existing;
  }
  return {
    settings: clone(slot.settings),
    title: slot.title,
    raw: clone(slot.raw),
    dirty: false,
    revision: existing?.revision ?? 0,
    source: sourceOf(slot),
    baseState: slot.state,
    settingsEdited: false,
    rawEdited: false,
    titleEdited: false,
    appearanceEdits: {},
    appearanceBases: {},
    appearanceState: existing?.appearanceState,
  };
}
export function editDraft(draft: Draft, patch: Partial<Draft>): Draft {
  let next = { ...draft, ...patch, dirty: true, revision: draft.revision + 1 };
  if (Object.hasOwn(patch, "settings")) next.settingsEdited = true;
  if (Object.hasOwn(patch, "raw")) next.rawEdited = true;
  if (Object.hasOwn(patch, "title")) {
    const state = draft.appearanceState ?? draft.baseState ?? 0;
    const edits = { ...draft.appearanceEdits?.[state], title: patch.title };
    delete edits.reset_title;
    next = {
      ...next,
      titleEdited: true,
      appearanceState: state,
      appearanceExpectedState:
        draft.appearanceExpectedState ?? draft.baseState ?? 0,
      appearanceEdits: { ...draft.appearanceEdits, [state]: edits },
    };
  }
  return next;
}
export function editAppearance(
  draft: Draft,
  slot: Slot,
  state: number,
  patch: AppearanceEdit,
  base: AppearanceInfo,
): Draft {
  const edits = { ...draft.appearanceEdits?.[state], ...patch };
  if (patch.title_parameters)
    edits.title_parameters = {
      ...draft.appearanceEdits?.[state]?.title_parameters,
      ...patch.title_parameters,
    };
  if (Object.hasOwn(patch, "title")) delete edits.reset_title;
  if (patch.reset_title) delete edits.title;
  if (Object.hasOwn(patch, "image_path")) delete edits.reset_image;
  if (patch.reset_image) delete edits.image_path;
  return {
    ...draft,
    dirty: true,
    revision: draft.revision + 1,
    appearanceState: state,
    titleEdited:
      draft.titleEdited || Object.hasOwn(patch, "title") || !!patch.reset_title,
    title: patch.title ?? draft.title,
    appearanceExpectedState: draft.appearanceExpectedState ?? slot.state,
    appearanceEdits: { ...draft.appearanceEdits, [state]: edits },
    appearanceBases: {
      ...draft.appearanceBases,
      [state]: draft.appearanceBases?.[state] ?? clone(base),
    },
  };
}
export function hasAppearanceEdits(draft: Draft) {
  return Object.keys(draft.appearanceEdits ?? {}).length > 0;
}
export function appearancePatch(
  draft: Draft,
  state: number,
  userTitleEnabled = true,
): AppearanceEdit {
  const patch = clone(draft.appearanceEdits?.[state] ?? {});
  if (!userTitleEnabled) {
    delete patch.title;
    delete patch.reset_title;
  }
  return patch;
}
export function finishSave(current: Draft, saved: Draft): Draft {
  return current.revision === saved.revision
    ? {
        ...current,
        dirty: false,
        settingsEdited: false,
        rawEdited: false,
        titleEdited: false,
        appearanceEdits: {},
        appearanceBases: {},
        appearanceExpectedState: undefined,
        source: "",
      }
    : current;
}
export const MULTI = "com.elgato.streamdeck.multiactions.";
export const STACK = "com.elgato.streamdeck.dial.stack";
export const WHEEL = "com.elgato.streamdeck.keys.stack";
export const TRIGGER = "com.elgato.streamdeck.keys.adaptor";
export const isMulti = (id: string) =>
  [MULTI + "routine", MULTI + "routine2", MULTI + "random"].includes(id);
export const isContainer = (id: string) =>
  isMulti(id) || id === STACK || id === TRIGGER || id === WHEEL;
export function allowed(action: Action, controller: Controller) {
  return (
    action.visible !== false &&
    action.enabled !== false &&
    !action.multi_only &&
    action.controllers.includes(controller)
  );
}
export function childAllowed(action: Action, parent: string) {
  if (action.enabled === false) return false;
  if (parent === STACK)
    return action.controllers.includes("Encoder") && action.uuid !== STACK;
  if (parent === WHEEL && action.multi_only) return false;
  if (parent === TRIGGER || parent === WHEEL)
    return (
      action.controllers.includes("Keypad") &&
      ![STACK, TRIGGER, WHEEL].includes(action.uuid)
    );
  return (
    action.controllers.includes("Keypad") &&
    action.supported_in_multi &&
    !isContainer(action.uuid)
  );
}
export function newChild(action: Action): RecordValue {
  const raw: RecordValue = {
    ActionID: newId(),
    UUID: action.uuid,
    Name: action.source_name ?? action.name,
    Settings: clone(action.defaults ?? {}),
    State: 0,
    States: clone(
      action.source_states !== undefined
        ? (action.source_states ?? [])
        : (action.states ?? []),
    ),
  };
  if (isMulti(action.uuid)) raw.Actions = [{ Actions: [] }, { Actions: [] }];
  if (action.uuid === WHEEL) raw.Actions = [];
  if (action.uuid === TRIGGER)
    raw.Actions = [{ Actions: [] }, { Actions: [] }, { Actions: [] }];
  return raw;
}
export function children(
  raw: RecordValue,
  uuid: string,
  branch = 0,
): RecordValue[] {
  const value = isMulti(uuid) ? raw.Actions?.[branch]?.Actions : raw.Actions;
  return Array.isArray(value) ? value : [];
}
export function replaceChildren(
  raw: RecordValue,
  uuid: string,
  values: RecordValue[],
  branch = 0,
) {
  const next = clone(raw);
  if (next.Actions !== undefined && !Array.isArray(next.Actions))
    throw new Error(
      "Imported action structure cannot be edited safely. Use the advanced record editor.",
    );
  next.Actions ??= [];
  if (isMulti(uuid)) {
    while (next.Actions.length <= branch) next.Actions.push({ Actions: [] });
    if (
      typeof next.Actions[branch] !== "object" ||
      Array.isArray(next.Actions[branch]) ||
      next.Actions[branch] === null
    )
      throw new Error("Imported sequence structure is not an object.");
    next.Actions[branch] = { ...next.Actions[branch], Actions: values };
  } else next.Actions = values;
  return next;
}
export function saveRequests(
  slot: Slot,
  draft: Draft,
  userTitleEnabled = true,
): RecordValue[] {
  const requests: RecordValue[] = [];
  for (const state of Object.keys(draft.appearanceEdits ?? {})) {
    const patch = appearancePatch(draft, Number(state), userTitleEnabled);
    if (Object.keys(patch).length)
      requests.push({
        command: "appearance",
        context: slot.context,
        expected_state: draft.appearanceExpectedState ?? slot.state,
        state: Number(state),
        ...patch,
      });
  }
  if (isContainer(slot.uuid) && draft.rawEdited) {
    let wheelBase: RecordValue | undefined;
    if (slot.uuid === WHEEL) {
      try {
        wheelBase = JSON.parse(draft.source)[2];
      } catch {
        /* Legacy draft has no merge baseline. */
      }
    }
    requests.push({
      command:
        slot.uuid === STACK
          ? "stack_save"
          : slot.uuid === WHEEL
            ? "wheel_save"
            : slot.uuid === TRIGGER
              ? "trigger_save"
              : "multi_save",
      context: slot.context,
      raw: draft.raw,
      ...(wheelBase ? { base: wheelBase } : {}),
    });
  } else if (!isContainer(slot.uuid) && draft.settingsEdited) {
    let base: RecordValue | undefined;
    try {
      const source = JSON.parse(draft.source);
      if (
        Array.isArray(source) &&
        source[0] !== null &&
        typeof source[0] === "object" &&
        !Array.isArray(source[0])
      )
        base = source[0];
    } catch {
      /* Older retained drafts may not have a parseable baseline. */
    }
    requests.push(
      base === undefined
        ? {
            command: "settings",
            context: slot.context,
            settings: draft.settings,
          }
        : {
            command: "settings_patch",
            context: slot.context,
            base,
            settings: draft.settings,
          },
    );
  }
  return requests;
}
export function filteredActions(
  actions: Action[],
  query: string,
  controller: Controller | "All",
  availableControllers: Controller[] = ["Keypad", "Encoder"],
) {
  const words = query.toLocaleLowerCase().trim().split(/\s+/).filter(Boolean);
  return actions.filter(
    (a) =>
      a.visible !== false &&
      !a.multi_only &&
      a.controllers.some((value) => availableControllers.includes(value)) &&
      (controller === "All" || a.controllers.includes(controller)) &&
      words.every((word) =>
        `${a.name} ${a.category} ${a.plugin_name} ${a.uuid}`
          .toLocaleLowerCase()
          .includes(word),
      ),
  );
}
