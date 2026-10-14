// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import {
  cleanup,
  render,
  screen,
  waitFor,
  within,
  fireEvent,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Sequence } from "./Sequence";
import { STACK, LOGIC, type Action, type Slot } from "./model";
afterEach(cleanup);
it("inspects and copies deep read-only children while blocking mutations and drops", async () => {
  const user = userEvent.setup(),
    clipboardRun = vi.fn(async () => true),
    onChange = vi.fn();
  const leaf = {
    UUID: "example",
    ActionID: "leaf",
    Name: "Second",
    Settings: { value: 12 },
  };
  const branch = {
    UUID: "com.elgato.streamdeck.multiactions.routine2",
    ActionID: "branch",
    Name: "Switch",
    Actions: [{ Actions: [] }, { Actions: [leaf] }],
  };
  const raw = { Actions: [branch, {}, {}] };
  const child = {
    uuid: leaf.UUID,
    context: "leaf-context",
    name: "Second",
    raw: leaf,
  };
  const nested = {
    uuid: branch.UUID,
    context: "branch-context",
    raw: branch,
    multi_children: [[], [child]],
  };
  const runtime = {
    uuid: LOGIC,
    context: "root",
    raw,
    trigger_children: [nested, null, null],
  } as unknown as Slot;
  render(
    <Sequence
      raw={raw}
      uuid={LOGIC}
      runtime={runtime}
      readOnly
      clipboardRun={clipboardRun}
      clipboard={{
        mode: "copy",
        context: "source",
        uuid: "example",
        name: "Example",
        controller: "Keypad",
      }}
      actions={[
        {
          uuid: "example",
          name: "Example",
          controllers: ["Keypad"],
          supported_in_multi: true,
        } as Action,
      ]}
      outputs={[]}
      onError={vi.fn()}
      onChange={onChange}
    />,
  );
  await user.click(
    screen.getByRole("button", { name: "1× Switch Single press" }),
  );
  await user.click(screen.getByRole("button", { name: "Sequence 2" }));
  const inner = screen.getByRole("region", { name: "Action sequence" });
  const row = within(inner).getByRole("button", { name: "1 Second" });
  await user.click(row);
  expect(
    within(inner).getByLabelText("Action label").matches(":disabled"),
  ).toBe(true);
  await user.click(
    within(inner).getByRole("button", { name: "Copy selected" }),
  );
  expect(clipboardRun).toHaveBeenLastCalledWith({
    command: "slot_copy",
    context: "leaf-context",
  });
  expect(
    within(inner)
      .getByRole("button", { name: "Cut selected" })
      .matches(":disabled"),
  ).toBe(true);
  expect(
    within(inner)
      .getByRole("button", { name: "Paste after selected" })
      .matches(":disabled"),
  ).toBe(true);
  expect(
    within(inner)
      .getByRole("button", { name: "Remove action 1" })
      .matches(":disabled"),
  ).toBe(true);
  await user.click(row);
  await user.keyboard("{Control>}x{/Control}{Delete}{Control>}v{/Control}");
  fireEvent.drop(document.querySelector(".sequence-row")!, {
    dataTransfer: {
      types: ["application/x-deckforge-action"],
      getData: () => "example",
    },
  });
  expect(onChange).not.toHaveBeenCalled();
  expect(clipboardRun).toHaveBeenCalledTimes(1);
});
it("copies the selected legacy child when same-UUID siblings lack ActionID", async () => {
  const user = userEvent.setup(),
    clipboardRun = vi.fn(async () => true);
  const records = [
    { UUID: "example", Name: "First", Settings: {} },
    { UUID: "example", Name: "Second", Settings: {} },
  ];
  const runtime = {
    uuid: STACK,
    context: "stack",
    raw: { Actions: records },
    stack_children: records.map((raw, i) => ({
      uuid: raw.UUID,
      name: raw.Name,
      raw,
      context: `child-${i}`,
    })),
  } as unknown as Slot;
  render(
    <Sequence
      raw={runtime.raw}
      uuid={STACK}
      runtime={runtime}
      readOnly
      actions={[]}
      outputs={[]}
      onError={vi.fn()}
      onChange={vi.fn()}
      clipboardRun={clipboardRun}
    />,
  );
  await user.click(screen.getByRole("button", { name: "2 Second" }));
  await user.click(screen.getByRole("button", { name: "Copy selected" }));
  expect(clipboardRun).toHaveBeenLastCalledWith({
    command: "slot_copy",
    context: "child-1",
  });
});
it("copies the selected live child and pastes after it without invoking canvas shortcuts", async () => {
  const user = userEvent.setup(),
    clipboardRun = vi.fn(async () => true),
    outer = vi.fn();
  const raw = {
    Actions: [
      { ActionID: "step", UUID: "example", Name: "Example", Settings: {} },
    ],
  };
  const runtime = {
    context: "stack",
    uuid: STACK,
    raw,
    stack_children: [
      {
        context: "live-child",
        uuid: "example",
        raw: raw.Actions[0],
        name: "Example",
      },
    ],
  } as unknown as Slot;
  const props = {
    raw,
    uuid: STACK,
    runtime,
    actions: [
      {
        uuid: "example",
        name: "Example",
        controllers: ["Encoder"],
        visible: true,
      } as Action,
    ],
    outputs: [] as [string, string][],
    onChange: vi.fn(),
    onError: vi.fn(),
    clipboardRun,
    clipboard: {
      context: "source",
      name: "Example",
      uuid: "example",
      controller: "Encoder" as const,
      mode: "copy" as const,
    },
  };
  const view = render(
    <div onKeyDown={outer}>
      <Sequence {...props} />
    </div>,
  );
  await user.click(screen.getByRole("button", { name: "1 Example" }));
  await user.keyboard("{Control>}c{/Control}");
  expect(clipboardRun).toHaveBeenLastCalledWith({
    command: "slot_copy",
    context: "live-child",
  });
  await user.keyboard("{Control>}x{/Control}");
  expect(clipboardRun).toHaveBeenLastCalledWith({
    command: "slot_cut",
    context: "live-child",
  });
  expect(outer.mock.calls.filter(([e]) => e.key === "c")).toHaveLength(0);
  await user.click(
    screen.getByRole("button", { name: "Paste after selected" }),
  );
  await waitFor(() =>
    expect(clipboardRun).toHaveBeenLastCalledWith({
      command: "child_paste",
      context: "stack",
      branch: 0,
      index: 1,
      expected: raw,
    }),
  );
  expect(props.onChange).not.toHaveBeenCalled();
  view.rerender(<Sequence {...props} clipboardReady={false} />);
  expect(
    (screen.getByRole("button", { name: "Copy selected" }) as HTMLButtonElement)
      .disabled,
  ).toBe(true);
  expect(
    (screen.getByRole("button", { name: "Cut selected" }) as HTMLButtonElement)
      .disabled,
  ).toBe(true);
  expect(
    (screen.getByRole("button", { name: /Paste/ }) as HTMLButtonElement)
      .disabled,
  ).toBe(true);
});
it("leaves text copy shortcuts to child text fields", async () => {
  const user = userEvent.setup(),
    clipboardRun = vi.fn(async () => true);
  render(
    <Sequence
      raw={{
        Actions: [
          { ActionID: "step", UUID: "example", Name: "Example", Settings: {} },
        ],
      }}
      uuid={STACK}
      actions={[]}
      outputs={[]}
      onError={vi.fn()}
      onChange={vi.fn()}
      clipboardRun={clipboardRun}
    />,
  );
  await user.click(screen.getByRole("button", { name: "1 Example" }));
  await user.click(screen.getByLabelText("Action label"));
  await user.keyboard("{Control>}c{/Control}");
  expect(clipboardRun).not.toHaveBeenCalled();
});
it("localizes default child labels and headings while preserving custom labels and raw action-label inputs", async () => {
  const action = {
    uuid: "example",
    name: "Zähler",
    source_name: "Counter",
    controllers: ["Encoder"],
    visible: true,
    plugin_name: "Probe",
  } as unknown as Action;
  const entry = (id: string, name: string) => ({
    ActionID: id,
    UUID: "example",
    Name: name,
    Settings: {},
  });
  const entries = [
    entry("a", "Counter"),
    entry("b", "My custom label"),
    entry("c", "Counter"),
  ];
  const runtime = {
    uuid: STACK,
    stack_children: [
      { uuid: "example", name: "Laufzeit Zähler", raw: { ActionID: "a" } },
      {
        uuid: "example",
        name: "Wrong runtime custom label",
        raw: { ActionID: "b" },
      },
    ],
  } as unknown as Slot;
  const onChange = vi.fn(),
    user = userEvent.setup();
  const view = render(
    <Sequence
      raw={{ Actions: entries }}
      uuid={STACK}
      actions={[action]}
      runtime={runtime}
      outputs={[]}
      onError={vi.fn()}
      onChange={onChange}
    />,
  );
  await user.click(screen.getByRole("button", { name: "1 Laufzeit Zähler" }));
  expect(
    document.querySelector(".child-editor .section-label")?.textContent,
  ).toBe("Laufzeit Zähler");
  expect(
    (screen.getByLabelText("Action label") as HTMLInputElement).value,
  ).toBe("Counter");
  expect(screen.getByRole("button", { name: "3 Zähler" })).toBeTruthy();
  await user.click(screen.getByRole("button", { name: "2 My custom label" }));
  expect(
    document.querySelector(".child-editor .section-label")?.textContent,
  ).toBe("My custom label");
  expect(
    (screen.getByLabelText("Action label") as HTMLInputElement).value,
  ).toBe("My custom label");
  expect(onChange).not.toHaveBeenCalled();
  view.rerender(
    <Sequence
      raw={{ Actions: entries }}
      uuid={STACK}
      actions={[{ ...action, name: "Compteur" }]}
      outputs={[]}
      onError={vi.fn()}
      onChange={onChange}
    />,
  );
  expect(screen.getByRole("button", { name: "1 Compteur" })).toBeTruthy();
  expect(
    screen.getByRole("button", { name: "2 My custom label" }),
  ).toBeTruthy();
});
it("adding a translated action uses original records while showing its translated picker label", async () => {
  const onChange = vi.fn(),
    user = userEvent.setup();
  const action = {
    uuid: "example",
    name: "Zähler",
    source_name: "Counter",
    source_states: [
      { Name: "Off", Title: "User title", Image: "states/original" },
    ],
    states: [{ Name: "Aus", Title: "User title", Image: "states/original" }],
    controllers: ["Encoder"],
    visible: true,
    plugin_name: "Probe",
    defaults: { step: 2 },
  } as unknown as Action;
  render(
    <Sequence
      raw={{ Actions: [] }}
      uuid={STACK}
      actions={[action]}
      outputs={[]}
      onError={vi.fn()}
      onChange={onChange}
    />,
  );
  await user.selectOptions(
    screen.getByRole("combobox", { name: "Action to add" }),
    "example",
  );
  expect(screen.getByRole("option", { name: "Zähler · Probe" })).toBeTruthy();
  await user.click(screen.getByRole("button", { name: "Add" }));
  expect(onChange.mock.calls[0][0].Actions[0]).toMatchObject({
    UUID: "example",
    Name: "Counter",
    States: action.source_states,
    Settings: { step: 2 },
  });
});

it("edits bounded opt-in action timing and removes the invalid automatic state sentinel", async () => {
  const user = userEvent.setup(),
    onChange = vi.fn();
  const entry = {
    ActionID: "one",
    UUID: "example",
    Name: "Example",
    Settings: { Future: true },
    States: [{ Title: "Off" }, { Title: "On" }],
    OverrideState: 1,
    Future: "keep",
  };
  const raw = { Actions: [{ Actions: [entry] }, { Actions: [] }] };
  const props = {
    uuid: "com.elgato.streamdeck.multiactions.routine",
    actions: [] as Action[],
    outputs: [] as [string, string][],
    onError: vi.fn(),
    onChange,
  };
  const view = render(<Sequence {...props} raw={raw} />);
  await user.click(screen.getByRole("button", { name: /1 Example/ }));
  expect(onChange).not.toHaveBeenCalled();
  expect(
    screen.queryByRole("spinbutton", { name: "Hold duration (milliseconds)" }),
  ).toBeNull();
  await user.selectOptions(screen.getByRole("combobox", { name: "State" }), "");
  expect(
    onChange.mock.calls.at(-1)![0].Actions[0].Actions[0].OverrideState,
  ).toBeUndefined();
  await user.click(
    screen.getByRole("checkbox", { name: "Custom action timing" }),
  );
  const changed = onChange.mock.calls.at(-1)![0];
  expect(changed.Actions[0].Actions[0].MultiActionDelays).toEqual({
    Delay1: 100,
    Delay2: 100,
  });
  expect(changed.Actions[0].Actions[0].Future).toBe("keep");
  changed.Actions[0].Actions[0].MultiActionDelays.FutureTiming = "preserve";
  view.rerender(<Sequence {...props} raw={changed} />);
  const held = screen.getByRole("spinbutton", {
    name: "Hold duration (milliseconds)",
  }) as HTMLInputElement;
  expect(held.min).toBe("1");
  expect(held.max).toBe("100");
  await user.clear(held);
  await user.type(held, "25");
  expect(
    onChange.mock.calls.at(-1)![0].Actions[0].Actions[0].MultiActionDelays,
  ).toEqual({ Delay1: 25, Delay2: 100, FutureTiming: "preserve" });
  await user.click(
    screen.getByRole("checkbox", { name: "Custom action timing" }),
  );
  expect(
    onChange.mock.calls.at(-1)![0].Actions[0].Actions[0].MultiActionDelays,
  ).toBeUndefined();
});

it("forwards folder page choices into nested action editors", async () => {
  const user = userEvent.setup(),
    onChange = vi.fn();
  render(
    <Sequence
      uuid="com.elgato.streamdeck.multiactions.routine"
      raw={{
        Actions: [
          {
            Actions: [
              {
                ActionID: "folder",
                UUID: "com.elgato.streamdeck.profile.openchild",
                Name: "Folder",
                Settings: {},
              },
            ],
          },
        ],
      }}
      actions={[]}
      outputs={[]}
      folderPages={[["target", "Folder destination"]]}
      onChange={onChange}
      onError={vi.fn()}
    />,
  );
  await user.click(screen.getByRole("button", { name: /1 Folder/ }));
  await user.selectOptions(
    screen.getByRole("combobox", { name: "Folder page" }),
    "target",
  );
  expect(onChange.mock.calls.at(-1)![0].Actions[0].Actions[0].Settings).toEqual(
    { ProfileUUID: "target" },
  );
});

it("keeps live previews attached to child identities when a draft reorders or replaces actions", () => {
  const uuid = "com.elgato.streamdeck.multiactions.routine";
  const entries = ["first", "second"].map((id) => ({
    UUID: "example",
    ActionID: id,
    Name: id,
    Settings: {},
  }));
  const runtime = {
    uuid,
    multi_children: [
      entries.map((raw) => ({
        uuid: raw.UUID,
        raw,
        name: raw.Name,
        preview: `${raw.ActionID}-png`,
      })),
    ],
  } as unknown as Slot;
  const props = {
    uuid,
    runtime,
    actions: [],
    outputs: [] as [string, string][],
    onError: vi.fn(),
    onChange: vi.fn(),
  };
  const view = render(
    <Sequence {...props} raw={{ Actions: [{ Actions: entries }] }} />,
  );
  const sources = () =>
    [...view.container.querySelectorAll("img.sequence-preview")].map((image) =>
      image.getAttribute("src"),
    );
  expect(sources()).toEqual([
    "data:image/png;base64,first-png",
    "data:image/png;base64,second-png",
  ]);
  view.rerender(
    <Sequence
      {...props}
      raw={{ Actions: [{ Actions: [entries[1], entries[0]] }] }}
    />,
  );
  expect(sources()).toEqual([
    "data:image/png;base64,second-png",
    "data:image/png;base64,first-png",
  ]);
  view.rerender(
    <Sequence
      {...props}
      raw={{ Actions: [{ Actions: [{ ...entries[0], UUID: "replacement" }] }] }}
    />,
  );
  expect(sources()).toEqual([]);
});
it("Key Logic edits fixed gestures and excludes manifest opt-outs", async () => {
  const user = userEvent.setup(),
    onChange = vi.fn();
  const logic = "com.elgato.streamdeck.keys.logic";
  const action = {
    uuid: "test.key",
    name: "Allowed",
    controllers: ["Keypad"],
    plugin_name: "Test",
    defaults: { value: 7 },
    states: [{}],
  } as unknown as Action;
  render(
    <Sequence
      uuid={logic}
      raw={{
        Future: 9,
        Actions: [{ Actions: [] }, { Actions: [] }, { Actions: [] }],
      }}
      actions={[
        action,
        {
          ...action,
          uuid: "test.hidden",
          name: "Opted out",
          supported_in_key_logic: false,
        },
      ]}
      outputs={[]}
      onError={vi.fn()}
      onChange={onChange}
    />,
  );
  expect(screen.getByText("Single press")).toBeTruthy();
  expect(screen.getByText("Press and hold")).toBeTruthy();
  expect(screen.queryByRole("option", { name: /Opted out/ })).toBeNull();
  await user.click(screen.getByRole("button", { name: /Double press/ }));
  await user.selectOptions(
    screen.getByRole("combobox", { name: "Action to add" }),
    "test.key",
  );
  await user.click(screen.getByRole("button", { name: "Assign" }));
  const changed = onChange.mock.calls.at(-1)![0];
  expect(changed.Actions[0]).toEqual({ Actions: [] });
  expect(changed.Actions[2]).toEqual({ Actions: [] });
  expect(changed.Actions[1].UUID).toBe("test.key");
  expect(changed.Future).toBe(9);
});
