// @vitest-environment jsdom
import { useState } from "react";
import { afterEach, expect, it } from "vitest";
import { cleanup, render, screen, fireEvent } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { FolderExit } from "./FolderExit";
import { editDraft, reconcileDraft, saveRequests, type Slot } from "./model";
afterEach(cleanup);
it("edits inherited options and saves only exit intent beside live plugin settings", async () => {
  const user = userEvent.setup();
  const slot = {
    context: "counter",
    uuid: "counter",
    raw: { Future: 5 },
    settings: { count: 1 },
    title: "1",
    state: 0,
  } as unknown as Slot;
  let requests: any[] = [];
  function Editor() {
    const [draft, setDraft] = useState(reconcileDraft(undefined, slot));
    return (
      <>
        <FolderExit
          raw={draft.raw}
          parent={{ AutoExit: { Enabled: true, Delay: 10 } }}
          back={false}
          onChange={(update) => {
            const edited = editDraft(draft, { raw: update(draft.raw) });
            setDraft(edited);
            requests = saveRequests(
              { ...slot, settings: { count: 9 } },
              edited,
            );
          }}
        />
        <output>{JSON.stringify(draft.raw)}</output>
      </>
    );
  }
  render(<Editor />);
  await user.click(screen.getByText("Folder auto-exit"));
  const delay = screen.getByRole("spinbutton", {
    name: "Exit delay (seconds)",
  }) as HTMLInputElement;
  expect(delay.closest("fieldset[disabled]")).not.toBeNull();
  expect(delay.value).toBe("10");
  await user.click(
    screen.getByRole("checkbox", { name: "Follow folder settings" }),
  );
  await user.clear(delay);
  await user.type(delay, "2");
  expect(requests).toEqual([
    {
      command: "folder_exit",
      context: "counter",
      auto_exit: { Enabled: true, Delay: 2 },
    },
  ]);
  expect(screen.getByRole("status").textContent).toContain('"Future":5');
  await user.click(
    screen.getByRole("checkbox", { name: "Automatically exit folder" }),
  );
  expect(requests[0].auto_exit.Enabled).toBe(false);
  await user.click(
    screen.getByRole("checkbox", { name: "Follow folder settings" }),
  );
  expect(requests).toEqual([]);
});
it("saves removal of an existing explicit override", () => {
  const slot = {
    context: "key",
    uuid: "counter",
    raw: { AutoExit: { Enabled: false, Delay: 1 } },
    settings: {},
    title: "",
    state: 0,
  } as unknown as Slot;
  const draft = editDraft(reconcileDraft(undefined, slot), { raw: {} });
  expect(saveRequests(slot, draft)).toEqual([
    { command: "folder_exit", context: "key", auto_exit: null },
  ]);
});
it("requires confirmation for reset and cancellation keeps overrides", async () => {
  const user = userEvent.setup();
  let resets = 0;
  render(
    <FolderExit
      raw={{ AutoExit: { Enabled: true, Delay: 1 } }}
      back
      resetCount={2}
      onChange={() => {}}
      onReset={async () => {
        resets++;
      }}
    />,
  );
  await user.click(screen.getByText("Folder auto-exit"));
  await user.click(
    screen.getByRole("button", { name: "Reset action overrides (2)" }),
  );
  expect(resets).toBe(0);
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  expect(resets).toBe(0);
  await user.click(
    screen.getByRole("button", { name: "Reset action overrides (2)" }),
  );
  await user.click(screen.getByRole("button", { name: "Reset overrides" }));
  expect(resets).toBe(1);
});

it("merges rapid field edits against the latest draft even before props refresh", () => {
  let raw: Record<string, any> = { Future: 3 };
  render(
    <FolderExit
      raw={{ Future: 3 }}
      back
      onChange={(update) => {
        raw = update(raw);
      }}
    />,
  );
  fireEvent.click(screen.getByText("Folder auto-exit"));
  fireEvent.click(
    screen.getByRole("checkbox", { name: "Automatically exit folder" }),
  );
  fireEvent.change(
    screen.getByRole("spinbutton", { name: "Exit delay (seconds)" }),
    { target: { value: "0" } },
  );
  expect(raw).toEqual({ Future: 3, AutoExit: { Enabled: true, Delay: 0 } });
});
