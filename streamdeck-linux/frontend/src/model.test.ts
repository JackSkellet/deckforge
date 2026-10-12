import { describe, it, expect } from "vitest";
import {
  reconcileDraft,
  editDraft,
  finishSave,
  replaceChildren,
  saveRequests,
  filteredActions,
  STACK,
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
  it("does not write an appearance override for action-owned live titles", () => {
    const draft = reconcileDraft(undefined, slot);
    expect(saveRequests(slot, draft, false)).toEqual([
      { command: "settings", context: slot.context, settings: slot.settings },
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
    });
    expect(saveRequests(slot, draft)).toEqual([
      { command: "settings", context: "counter-1", settings: slot.settings },
      { command: "appearance", context: "counter-1", title: "Changed" },
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
        reconcileDraft(undefined, { ...slot, raw }),
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
