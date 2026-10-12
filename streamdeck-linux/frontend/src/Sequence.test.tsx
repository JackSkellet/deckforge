// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Sequence } from "./Sequence";
import { STACK, type Action, type Slot } from "./model";
afterEach(cleanup);
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
