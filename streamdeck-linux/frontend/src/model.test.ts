import { describe, it, expect } from "vitest";
import {
  newChild,
  reconcileDraft,
  editDraft,
  finishSave,
  replaceChildren,
  saveRequests,
  editAppearance,
  filteredActions,
  STACK,
  WHEEL,
  MULTI,
  TRIGGER,
  type Slot,
  type Action,
} from "./model";
const slot = {
  context: "counter-1",
  uuid: "counter",
  settings: { count: 1, Future: { keep: true } },
  title: "Counter",
  raw: { Future: 7 },
  controller: "Keypad",
  index: 0,
  name: "Counter",
  pinned: false,
  available: true,
  state: 0,
  has_inspector: false,
  multi_running: false,
  stack_children: [],
  trigger_children: [],
  multi_children: [],
} as Slot;
describe("action drafts", () => {
  it("sends a wheel-specific save with the captured baseline", () => {
    const wheel = {
      ...slot,
      uuid: WHEEL,
      controller: "Encoder" as const,
      raw: { Settings: { CurrentIdx: 1, InvertRotation: false }, Actions: [] },
    };
    const draft = editDraft(reconcileDraft(undefined, wheel), {
      raw: {
        ...wheel.raw,
        Settings: { ...wheel.raw.Settings, InvertRotation: true },
      },
    });
    expect(saveRequests(wheel, draft)).toEqual([
      {
        command: "wheel_save",
        context: wheel.context,
        raw: draft.raw,
        base: wheel.raw,
      },
    ]);
  });
  it("falls back to legacy replacement only without a usable captured settings baseline", () => {
    const edited = editDraft(reconcileDraft(undefined, slot), {
      settings: { count: 99 },
    });
    expect(
      saveRequests(slot, { ...edited, source: "invalid old source" }),
    ).toEqual([
      { command: "settings", context: slot.context, settings: { count: 99 } },
    ]);
    expect(saveRequests(slot, edited)).toEqual([
      {
        command: "settings_patch",
        context: slot.context,
        base: slot.settings,
        settings: { count: 99 },
      },
    ]);
  });
  it("does not freeze a live plugin title when only settings change", () => {
    const initial = reconcileDraft(undefined, slot);
    const edited = editDraft(initial, {
      settings: { count: 8, Future: slot.settings.Future },
    });
    const live = { ...slot, title: "Plugin changed", state: 1 };
    expect(reconcileDraft(edited, live)).toBe(edited);
    expect(saveRequests(live, edited)).toEqual([
      {
        command: "settings_patch",
        context: slot.context,
        base: slot.settings,
        settings: edited.settings,
      },
    ]);
  });
  it("keeps per-state appearance intent and never writes untouched settings or other state records", () => {
    const base = {
      title_parameters: { fontFamily: "", fontSize: 11, FutureFont: true },
      title_custom: false,
      image_custom: false,
      title: "Counter",
    };
    let draft = reconcileDraft(undefined, slot);
    draft = editAppearance(
      draft,
      slot,
      1,
      { title_parameters: { fontSize: 18 } },
      base,
    );
    draft = editAppearance(draft, slot, 0, { title: "Custom" }, base);
    draft = editAppearance(draft, slot, 0, { reset_title: true }, base);
    expect(saveRequests({ ...slot, state: 1 }, draft)).toEqual([
      {
        command: "appearance",
        context: slot.context,
        expected_state: 0,
        state: 0,
        reset_title: true,
      },
      {
        command: "appearance",
        context: slot.context,
        expected_state: 0,
        state: 1,
        title_parameters: { fontSize: 18 },
      },
    ]);
    expect(draft.raw).toEqual(slot.raw);
    expect(draft.settings).toEqual(slot.settings);
    expect(draft.appearanceBases?.[1].title_parameters.FutureFont).toBe(true);
  });
  it("allows title styling on actions with live-owned text, without sending a title override", () => {
    const base = {
      title_parameters: {},
      title_custom: false,
      image_custom: false,
    };
    let draft = editAppearance(
      reconcileDraft(undefined, slot),
      slot,
      0,
      { title: "Never override", title_parameters: { fontStyle: "Bold" } },
      base,
    );
    expect(saveRequests(slot, draft, false)).toEqual([
      {
        command: "appearance",
        context: slot.context,
        expected_state: 0,
        state: 0,
        title_parameters: { fontStyle: "Bold" },
      },
    ]);
  });

  it("does not write an appearance override for action-owned live titles", () => {
    const draft = reconcileDraft(undefined, slot);
    expect(
      saveRequests(
        slot,
        editDraft(draft, { settings: { ...slot.settings, count: 2 } }),
        false,
      ),
    ).toEqual([
      {
        command: "settings_patch",
        base: slot.settings,
        context: slot.context,
        settings: { ...slot.settings, count: 2 },
      },
    ]);
  });
  it("does not let live plugin updates erase edited settings or title", () => {
    const first = reconcileDraft(undefined, slot);
    const dirty = editDraft(first, { title: "My counter" });
    const remote = { ...slot, settings: { count: 2 } };
    expect(reconcileDraft(dirty, remote)).toBe(dirty);
    expect(reconcileDraft(first, remote).settings.count).toBe(2);
  });
  it("does not mark changes made during save as saved", () => {
    const draft = editDraft(reconcileDraft(undefined, slot), {
      title: "First",
    });
    const later = editDraft(draft, { title: "Second" });
    expect(finishSave(later, draft)).toBe(later);
    expect(finishSave(draft, draft).dirty).toBe(false);
  });
  it("preserves vendor settings in exact host save commands", () => {
    const draft = editDraft(reconcileDraft(undefined, slot), {
      title: "Changed",
      settings: { ...slot.settings, count: 2 },
    });
    expect(saveRequests(slot, draft)).toEqual([
      {
        command: "appearance",
        context: "counter-1",
        expected_state: 0,
        state: 0,
        title: "Changed",
      },
      {
        command: "settings_patch",
        base: slot.settings,
        context: "counter-1",
        settings: { ...slot.settings, count: 2 },
      },
    ]);
  });
});
describe("structured containers", () => {
  it("reorders a sequence without dropping unknown branch/parent/child fields", () => {
    const first = { UUID: "x", Future: 9 };
    const second = { UUID: "y", Future: 10 };
    const raw = {
      FutureParent: 1,
      Actions: [
        { FutureBranch: 2, Actions: [first, second] },
        { Actions: [{ UUID: "z" }] },
      ],
    };
    const next = replaceChildren(raw, MULTI + "routine2", [second, first]);
    expect(next).toEqual({
      ...raw,
      Actions: [{ FutureBranch: 2, Actions: [second, first] }, raw.Actions[1]],
    });
    expect(raw.Actions[0].Actions?.[0]).toBe(first);
  });
  it("retains extra trigger gestures and stack properties", () => {
    const raw = {
      Settings: { Future: true },
      Actions: [{}, {}, {}, { FutureExtra: true }],
    };
    const next = replaceChildren(raw, TRIGGER, [
      { UUID: "new" },
      ...raw.Actions.slice(1),
    ]);
    expect(next.Actions[3]).toEqual({ FutureExtra: true });
    expect(next.Settings).toEqual(raw.Settings);
    expect(
      saveRequests(
        { ...slot, uuid: STACK },
        editDraft(reconcileDraft(undefined, { ...slot, raw }), { raw }),
      )[0].command,
    ).toBe("stack_save");
  });
  it("refuses destructive normalization of unfamiliar structures", () => {
    expect(() =>
      replaceChildren({ Actions: { Future: 1 } }, STACK, []),
    ).toThrow("cannot be edited safely");
  });
});
it("searches all terms across category/plugin/name and applies controller filters", () => {
  const a = {
    uuid: "counter",
    name: "Counter",
    category: "Utilities",
    plugin_name: "Elgato",
    visible: true,
    multi_only: false,
    controllers: ["Keypad"],
  } as Action;
  expect(filteredActions([a], "elgato count", "All")).toEqual([a]);
  expect(filteredActions([a], "count missing", "All")).toEqual([]);
  expect(filteredActions([a], "counter", "Encoder")).toEqual([]);
});

it("creates nested records from original manifest names/states, never translated presentation", () => {
  const originalStates = [
    {
      Name: "Off",
      Image: "icons/off",
      Title: "Original title",
      Future: { keep: 1 },
    },
  ];
  const action = {
    uuid: "plugin.localized",
    name: "Auslöser",
    source_name: "Trigger source",
    source_states: originalStates,
    states: [{ Name: "Aus", Image: "icons/off", Title: "Original title" }],
    defaults: { value: 7 },
  } as unknown as Action;
  for (const parent of [MULTI + "routine", STACK, TRIGGER]) {
    const child = newChild(action);
    expect(child.Name).toBe("Trigger source");
    expect(child.States).toEqual(originalStates);
    const container = replaceChildren({}, parent, [child]);
    expect(JSON.stringify(container)).not.toContain("Aus");
    child.States[0].Future.keep = 99;
    expect(originalStates[0].Future.keep).toBe(1);
  }
  expect(newChild({ ...action, source_states: null }).States).toEqual([]);
  const legacy = { ...action };
  delete legacy.source_name;
  delete legacy.source_states;
  expect(newChild(legacy)).toMatchObject({
    Name: "Auslöser",
    States: action.states,
  });
});
