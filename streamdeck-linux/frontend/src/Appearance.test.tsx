// @vitest-environment jsdom
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
} from "@testing-library/react";
import { afterEach, beforeEach, it, expect, vi } from "vitest";
import { AppearanceEditor } from "./Appearance";
import { editAppearance, reconcileDraft, type Slot, type Draft } from "./model";
const appearance = {
  title_parameters: {
    fontFamily: "",
    fontSize: 11,
    fontStyle: "Regular" as const,
    fontUnderline: false,
    showTitle: true,
    titleAlignment: "bottom" as const,
    titleColor: "#ffffff",
  },
  title_custom: false,
  image_custom: false,
  title: "Live counter",
};
const slot = {
  context: "one",
  uuid: "counter",
  controller: "Keypad",
  index: 0,
  name: "Counter",
  pinned: false,
  available: true,
  has_inspector: false,
  multi_running: false,
  stack_children: [],
  trigger_children: [],
  multi_children: [],
  state: 0,
  state_count: 2,
  title: "Live counter",
  settings: { count: 3 },
  raw: { Future: true },
  appearance,
  preview: "data:image/png;base64,saved",
} as Slot;
const response = (preview: string, state = 0) => ({
  ...appearance,
  preview,
  state,
  title: "Preview counter",
});
let request: ReturnType<typeof vi.fn>;
beforeEach(() => {
  vi.useFakeTimers();
  request = vi.fn();
  window.deckforge = { request, onSnapshot: () => () => {} };
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  delete window.deckforge;
});
const props = (current: Slot, draft: Draft) => ({
  slot: current,
  draft,
  onEdit: vi.fn(),
  onSelectState: vi.fn(),
  onRefreshState: vi.fn(),
  onError: vi.fn(),
});
it("debounces native draft previews and ignores older replies, without issuing save commands", async () => {
  const replies: ((v: any) => void)[] = [];
  request.mockImplementation(
    () => new Promise((resolve) => replies.push(resolve)),
  );
  const initial = editAppearance(
    reconcileDraft(undefined, slot),
    slot,
    0,
    { title: "First" },
    appearance,
  );
  const view = render(<AppearanceEditor {...props(slot, initial)} />);
  await act(() => vi.advanceTimersByTimeAsync(179));
  expect(request).not.toHaveBeenCalled();
  await act(() => vi.advanceTimersByTimeAsync(1));
  expect(request).toHaveBeenCalledWith("appearance_preview", {
    context: "one",
    expected_state: 0,
    state: 0,
    title: "First",
  });
  const next = editAppearance(
    initial,
    slot,
    0,
    { title: "Second" },
    appearance,
  );
  view.rerender(<AppearanceEditor {...props(slot, next)} />);
  await act(() => vi.advanceTimersByTimeAsync(180));
  await act(async () => replies[1](response("data:image/png;base64,new")));
  expect(
    (
      screen.getByRole("img", {
        name: "Draft appearance preview",
      }) as HTMLImageElement
    ).src,
  ).toBe("data:image/png;base64,new");
  await act(async () => replies[0](response("data:image/png;base64,old")));
  expect(
    (
      screen.getByRole("img", {
        name: "Draft appearance preview",
      }) as HTMLImageElement
    ).src,
  ).toBe("data:image/png;base64,new");
  expect(
    request.mock.calls.every(([method]) => method === "appearance_preview"),
  ).toBe(true);
});
it("ignores replies for a previous context and retains the new slot preview", async () => {
  let resolve!: (value: any) => void;
  request.mockImplementation(
    () =>
      new Promise((r) => {
        resolve = r;
      }),
  );
  const first = editAppearance(
    reconcileDraft(undefined, slot),
    slot,
    0,
    { title: "Draft" },
    appearance,
  );
  const view = render(<AppearanceEditor {...props(slot, first)} />);
  await act(() => vi.advanceTimersByTimeAsync(180));
  const next = {
    ...slot,
    context: "two",
    preview: "data:image/png;base64,two",
  };
  view.rerender(
    <AppearanceEditor {...props(next, reconcileDraft(undefined, next))} />,
  );
  await act(async () => resolve(response("data:image/png;base64,wrong")));
  expect(
    (
      screen.getByRole("img", {
        name: "Selected control preview",
      }) as HTMLImageElement
    ).src,
  ).toBe("data:image/png;base64,two");
});
it("retains appearance edits and stops preview requests when live state changes", async () => {
  const draft = editAppearance(
    reconcileDraft(undefined, slot),
    slot,
    0,
    { title: "Retained" },
    appearance,
  );
  const controls = props({ ...slot, state: 1 }, draft);
  render(<AppearanceEditor {...controls} />);
  await act(() => vi.advanceTimersByTimeAsync(500));
  expect(request).not.toHaveBeenCalled();
  expect(
    (screen.getByRole("textbox", { name: "Title" }) as HTMLInputElement).value,
  ).toBe("Retained");
  fireEvent.click(
    screen.getByRole("button", { name: "Refresh state and preview" }),
  );
  expect(controls.onRefreshState).toHaveBeenCalledOnce();
});
it("queries inactive-state appearance without flipping the live state", async () => {
  request.mockResolvedValue(response("data:image/png;base64,state-two", 1));
  const draft = { ...reconcileDraft(undefined, slot), appearanceState: 1 };
  render(<AppearanceEditor {...props(slot, draft)} />);
  await act(() => vi.advanceTimersByTimeAsync(180));
  expect(request).toHaveBeenCalledWith("appearance_preview", {
    context: "one",
    expected_state: 0,
    state: 1,
  });
  expect(slot.state).toBe(0);
  expect(request.mock.calls.some(([method]) => method === "command")).toBe(
    false,
  );
});
it("keeps the default and imported font options, and lets live-owned titles be styled", async () => {
  request.mockResolvedValue(["DejaVu Sans"]);
  const current = {
    ...slot,
    appearance: {
      ...appearance,
      title_parameters: {
        ...appearance.title_parameters,
        fontFamily: "Imported Face",
      },
    },
  };
  const controls = props(current, reconcileDraft(undefined, current));
  render(<AppearanceEditor {...controls} userTitleEnabled={false} />);
  const summary = screen.getByText("Appearance");
  const details = summary.closest("details")!;
  details.open = true;
  fireEvent(details, new Event("toggle", { bubbles: true }));
  await act(async () => {});
  const family = screen.getByRole("combobox", { name: "Font family" });
  expect((family as HTMLSelectElement).value).toBe("Imported Face");
  expect(
    screen.getAllByRole("option", { name: "Default" }).length,
  ).toBeGreaterThan(0);
  expect(
    (screen.getByDisplayValue("Live counter") as HTMLInputElement).disabled,
  ).toBe(true);
  fireEvent.click(screen.getByRole("checkbox", { name: "Underline" }));
  expect(controls.onEdit).toHaveBeenCalledWith(
    0,
    { title_parameters: { fontUnderline: true } },
    expect.anything(),
  );
  expect(controls.onEdit.mock.calls[0][1]).not.toHaveProperty("title");
});
it("shows native fallback reasons while keeping the requested imported family", async () => {
  request.mockImplementation(async (method) =>
    method === "fonts"
      ? ["DejaVu Sans"]
      : {
          ...response("data:image/png;base64,font"),
          title_parameters: {
            ...appearance.title_parameters,
            fontFamily: "Missing Font",
          },
          font_info: {
            fallback: true,
            reason: "Requested Missing Font; using DejaVu Sans",
            resolved_family: "DejaVu Sans",
          },
        },
  );
  const current = {
    ...slot,
    appearance: {
      ...appearance,
      title_parameters: {
        ...appearance.title_parameters,
        fontFamily: "Missing Font",
      },
    },
  };
  render(
    <AppearanceEditor
      {...props(current, reconcileDraft(undefined, current))}
    />,
  );
  const details = screen.getByText("Appearance").closest("details")!;
  details.open = true;
  fireEvent(details, new Event("toggle", { bubbles: true }));
  await act(() => vi.advanceTimersByTimeAsync(180));
  expect(
    screen.getByText("Requested Missing Font; using DejaVu Sans"),
  ).toBeTruthy();
  expect(
    (screen.getByRole("combobox", { name: "Font family" }) as HTMLSelectElement)
      .value,
  ).toBe("Missing Font");
});
it("captures the preview device alongside its context instead of consulting later selection", async () => {
  request.mockResolvedValue(response("data:image/png;base64,owned"));
  const draft = editAppearance(
    reconcileDraft(undefined, slot),
    slot,
    0,
    { title: "Owned" },
    appearance,
  );
  render(<AppearanceEditor {...props(slot, draft)} device="deck-A" />);
  await act(() => vi.advanceTimersByTimeAsync(180));
  expect(request).toHaveBeenCalledWith("appearance_preview", {
    device: "deck-A",
    context: "one",
    expected_state: 0,
    state: 0,
    title: "Owned",
  });
});

it("uses localized state names with fallback and live markers without changing user titles or state indexes", () => {
  const current = {
    ...slot,
    state_count: 3,
    state_names: ["Ein", "", "Aus"],
    title: "My personal title",
  };
  const callbacks = props(current, reconcileDraft(undefined, current));
  render(<AppearanceEditor {...callbacks} />);
  expect(
    screen
      .getByRole("option", { name: "Ein · live", hidden: true })
      .getAttribute("value"),
  ).toBe("0");
  expect(
    screen
      .getByRole("option", { name: "State 2", hidden: true })
      .getAttribute("value"),
  ).toBe("1");
  expect(
    screen
      .getByRole("option", { name: "Aus", hidden: true })
      .getAttribute("value"),
  ).toBe("2");
  fireEvent.change(screen.getByLabelText("Appearance state"), {
    target: { value: "2" },
  });
  expect(callbacks.onSelectState).toHaveBeenCalledWith(2);
  expect(callbacks.onEdit).not.toHaveBeenCalled();
  expect((screen.getByLabelText("Title") as HTMLTextAreaElement).value).toBe(
    "My personal title",
  );
});
