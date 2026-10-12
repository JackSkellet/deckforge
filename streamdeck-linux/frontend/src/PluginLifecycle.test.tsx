// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import App from "./App";
import { Plugins } from "./Panels";
import { installFixture } from "./fixture";
import { importFormDrafts, importTextDrafts } from "./transient";
import { scopedCommand } from "./bridge";
import {
  allowed,
  childAllowed,
  filteredActions,
  STACK,
  type Action,
  type Snapshot,
} from "./model";
beforeEach(() => {
  importFormDrafts({});
  importTextDrafts({});
  HTMLDialogElement.prototype.showModal = function () {
    this.setAttribute("open", "");
  };
  HTMLDialogElement.prototype.close = function () {
    this.removeAttribute("open");
  };
});
afterEach(() => {
  cleanup();
  delete window.deckforge;
});
it("uses captured enabled state and per-plugin busy controls without disabling other plugin controls", async () => {
  const snapshot = {
    plugins: [
      { id: "a", name: "Alpha", connected: true },
      {
        id: "b",
        name: "Beta",
        enabled: false,
        connected: false,
        status: "disabled",
      },
    ],
  } as unknown as Snapshot;
  let finish: (value: boolean) => void = () => {};
  const run = vi.fn(
    () =>
      new Promise<boolean>((resolve) => {
        finish = resolve;
      }),
  );
  const user = userEvent.setup();
  const view = render(
    <Plugins snapshot={snapshot} run={run} onError={vi.fn()} />,
  );
  await user.click(screen.getByRole("button", { name: "Disable Alpha" }));
  const payload = {
    command: "plugin_enabled",
    id: "a",
    enabled: false,
    expected_enabled: true,
  };
  expect(run).toHaveBeenCalledExactlyOnceWith(payload);
  expect(scopedCommand(payload, "device-A")).toEqual(payload);
  expect(
    (screen.getByRole("button", { name: "Disable Alpha" }) as HTMLButtonElement)
      .disabled,
  ).toBe(true);
  expect(
    (screen.getByRole("button", { name: "Restart Alpha" }) as HTMLButtonElement)
      .disabled,
  ).toBe(true);
  expect(
    (screen.getByRole("button", { name: "Enable Beta" }) as HTMLButtonElement)
      .disabled,
  ).toBe(false);
  expect(
    (screen.getByRole("button", { name: "Restart Beta" }) as HTMLButtonElement)
      .disabled,
  ).toBe(true);
  finish(false);
  await waitFor(() =>
    expect(
      (
        screen.getByRole("button", {
          name: "Disable Alpha",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(false),
  );
  view.rerender(
    <Plugins
      snapshot={{
        ...snapshot,
        plugins: [
          {
            ...snapshot.plugins[0],
            enabled: false,
            connected: false,
            status: "disabled",
          },
          snapshot.plugins[1],
        ],
      }}
      run={vi.fn(async () => true)}
      onError={vi.fn()}
    />,
  );
  expect(screen.getByRole("button", { name: "Enable Alpha" })).toBeTruthy();
});
it("keeps disabled actions discoverable but blocks assignment and new nested children", () => {
  const action = {
    uuid: "plugin.action",
    name: "Visible action",
    plugin_name: "Plugin",
    category: "Tools",
    enabled: false,
    visible: true,
    multi_only: false,
    supported_in_multi: true,
    controllers: ["Keypad", "Encoder"],
  } as Action;
  expect(filteredActions([action], "", "All")).toEqual([action]);
  expect(allowed(action, "Keypad")).toBe(false);
  expect(childAllowed(action, STACK)).toBe(false);
});
it("toggles in the real Plugins panel while preserving the selected draft and blocking disabled library drag/PI/input", async () => {
  installFixture();
  const original = window.deckforge!.request;
  const request = vi.fn(original);
  window.deckforge!.request = request;
  const user = userEvent.setup();
  render(<App />);
  await screen.findByRole("button", { name: "Key 1: Counter" });
  const increment = screen.getByLabelText("Increment by");
  await user.clear(increment);
  await user.type(increment, "3");
  await user.click(screen.getByRole("button", { name: /^Plugins/ }));
  await user.click(screen.getByRole("button", { name: "Disable Counter" }));
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "Enable Counter" })).toBeTruthy(),
  );
  expect(request).toHaveBeenCalledWith("command", {
    command: "plugin_enabled",
    id: "sample",
    enabled: false,
    expected_enabled: true,
  });
  expect(
    (
      screen.getByRole("button", {
        name: "Restart Counter",
      }) as HTMLButtonElement
    ).disabled,
  ).toBe(true);
  await user.click(screen.getByRole("button", { name: "Close Plugins" }));
  expect(
    screen.getByRole("button", { name: "Enable plugin in Plugins" }),
  ).toBeTruthy();
  expect(
    (screen.getByLabelText("Increment by") as HTMLInputElement).value,
  ).toBe("3");
  const assign = screen.getByRole("button", { name: "Assign Counter" });
  expect((assign as HTMLButtonElement).disabled).toBe(true);
  const row = assign.closest(".library-action")!;
  expect(row.getAttribute("draggable")).toBe("false");
  expect(row.textContent).toContain("Disabled");
  const dataTransfer = { setData: vi.fn(), types: [], getData: () => "" };
  fireEvent.dragStart(row, { dataTransfer });
  expect(dataTransfer.setData).not.toHaveBeenCalled();
  expect(
    (screen.getByRole("button", { name: "Test press" }) as HTMLButtonElement)
      .disabled,
  ).toBe(true);
  expect(
    screen.queryByRole("button", { name: "Plugin settings ↗" }),
  ).toBeNull();
  expect(
    (
      screen.getByRole("button", {
        name: "Assign Website",
      }) as HTMLButtonElement
    ).disabled,
  ).toBe(false);
  expect(
    (screen.getByRole("button", { name: "Save changes" }) as HTMLButtonElement)
      .disabled,
  ).toBe(false);
  await user.click(
    screen.getByRole("button", { name: "Enable plugin in Plugins" }),
  );
  await user.click(screen.getByRole("button", { name: "Enable Counter" }));
  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "Disable Counter" }),
    ).toBeTruthy(),
  );
  expect(request).toHaveBeenCalledWith("command", {
    command: "plugin_enabled",
    id: "sample",
    enabled: true,
    expected_enabled: false,
  });
  await user.click(screen.getByRole("button", { name: "Close Plugins" }));
  expect(
    (screen.getByLabelText("Increment by") as HTMLInputElement).value,
  ).toBe("3");
  expect(
    (
      screen.getByRole("button", {
        name: "Assign Counter",
      }) as HTMLButtonElement
    ).disabled,
  ).toBe(false);
});
