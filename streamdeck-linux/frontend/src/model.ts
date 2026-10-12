// Vendor action records deliberately remain open: unfamiliar fields must survive editing.
export type RecordValue = Record<string, any>;
export type Controller = "Keypad" | "Encoder";
export interface Action {
  user_title_enabled?: boolean;
  uuid: string;
  name: string;
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
export interface Slot {
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
  id: string;
  name: string;
  readonly: boolean;
  pin_page: string;
  page_order: string[];
  pages: { id: string; name: string; folder: boolean }[];
}
export interface Clipboard {
  name: string;
  uuid: string;
  controller: Controller;
  mode: "copy" | "cut";
  context: string;
}
export interface Snapshot {
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
export interface Draft {
  settings: RecordValue;
  title: string;
  raw: RecordValue;
  dirty: boolean;
  revision: number;
  source: string;
}
export const clone = <T>(value: T): T => structuredClone(value);
export const sourceOf = (slot: Slot) =>
  JSON.stringify([slot.settings, slot.title, slot.raw]);
export function reconcileDraft(existing: Draft | undefined, slot: Slot): Draft {
  if (existing && (existing.dirty || existing.source === sourceOf(slot)))
    return existing;
  return {
    settings: clone(slot.settings),
    title: slot.title,
    raw: clone(slot.raw),
    dirty: false,
    revision: existing?.revision ?? 0,
    source: sourceOf(slot),
  };
}
export function editDraft(draft: Draft, patch: Partial<Draft>): Draft {
  return { ...draft, ...patch, dirty: true, revision: draft.revision + 1 };
}
export function finishSave(current: Draft, saved: Draft): Draft {
  return current.revision === saved.revision
    ? {
        ...current,
        dirty: false,
        source: JSON.stringify([saved.settings, saved.title, saved.raw]),
      }
    : current;
}
export const MULTI = "com.elgato.streamdeck.multiactions.";
export const STACK = "com.elgato.streamdeck.dial.stack";
export const TRIGGER = "com.elgato.streamdeck.keys.adaptor";
export const isMulti = (id: string) =>
  [MULTI + "routine", MULTI + "routine2", MULTI + "random"].includes(id);
export const isContainer = (id: string) =>
  isMulti(id) || id === STACK || id === TRIGGER;
export function allowed(action: Action, controller: Controller) {
  return (
    action.visible !== false &&
    !action.multi_only &&
    action.controllers.includes(controller)
  );
}
export function childAllowed(action: Action, parent: string) {
  if (parent === STACK)
    return action.controllers.includes("Encoder") && action.uuid !== STACK;
  if (parent === TRIGGER)
    return (
      action.controllers.includes("Keypad") &&
      ![STACK, TRIGGER].includes(action.uuid)
    );
  return (
    action.controllers.includes("Keypad") &&
    action.supported_in_multi &&
    !isContainer(action.uuid)
  );
}
export function newChild(action: Action): RecordValue {
  const raw: RecordValue = {
    ActionID: crypto.randomUUID(),
    UUID: action.uuid,
    Name: action.name,
    Settings: clone(action.defaults ?? {}),
    State: 0,
    States: clone(action.states ?? []),
  };
  if (isMulti(action.uuid)) raw.Actions = [{ Actions: [] }, { Actions: [] }];
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
  const appearance = userTitleEnabled
    ? [{ command: "appearance", context: slot.context, title: draft.title }]
    : [];
  if (isContainer(slot.uuid))
    return [
      {
        command:
          slot.uuid === STACK
            ? "stack_save"
            : slot.uuid === TRIGGER
              ? "trigger_save"
              : "multi_save",
        context: slot.context,
        raw: draft.raw,
      },
      ...appearance,
    ];
  return [
    { command: "settings", context: slot.context, settings: draft.settings },
    ...appearance,
  ];
}
export function filteredActions(
  actions: Action[],
  query: string,
  controller: Controller | "All",
) {
  const words = query.toLocaleLowerCase().trim().split(/\s+/).filter(Boolean);
  return actions.filter(
    (a) =>
      a.visible !== false &&
      !a.multi_only &&
      (controller === "All" || a.controllers.includes(controller)) &&
      words.every((word) =>
        `${a.name} ${a.category} ${a.plugin_name} ${a.uuid}`
          .toLocaleLowerCase()
          .includes(word),
      ),
  );
}
