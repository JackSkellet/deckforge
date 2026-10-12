// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Plugins } from "./Panels";
import App from "./App";
import { installFixture } from "./fixture";
import { importFormDrafts, importTextDrafts } from "./transient";
import { scopedCommand } from "./bridge";
import type { Snapshot } from "./model";
const plugin = {
  id: "a",
  name: "Alpha",
  path: "/private/plugins/alpha",
  enabled: true,
  connected: true,
  removal_mode: "archive",
};
const snapshot = {
  plugins: [
    plugin,
    {
      id: "b",
      name: "Beta",
      path: "/external/beta",
      enabled: true,
      connected: true,
      removal_mode: "detach",
    },
  ],
} as unknown as Snapshot;
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
it("requires an inline exact-plugin confirmation and keeps failure/preconditions when the snapshot changes", async () => {
  const run = vi.fn(async () => false),
    user = userEvent.setup();
  const view = render(
    <Plugins snapshot={snapshot} run={run} onError={vi.fn()} />,
  );
  await user.click(screen.getByRole("button", { name: "Remove Alpha" }));
  expect(run).not.toHaveBeenCalled();
  expect(screen.getByText(/managed plugin folder will be moved/)).toBeTruthy();
  expect(screen.getByText(/private plugin data are retained/)).toBeTruthy();
  expect(document.activeElement).toBe(
    screen.getByRole("button", { name: "Cancel" }),
  );
  view.rerender(
    <Plugins
      snapshot={{
        ...snapshot,
        plugins: [
          { ...plugin, path: "/replacement/alpha", enabled: false },
          snapshot.plugins[1],
        ],
      }}
      run={run}
      onError={vi.fn()}
    />,
  );
  await user.click(
    screen.getByRole("button", { name: "Confirm removal of Alpha" }),
  );
  const payload = {
    command: "uninstall_plugin",
    id: "a",
    expected_path: "/private/plugins/alpha",
    expected_enabled: true,
    confirm: true,
  };
  expect(run).toHaveBeenCalledExactlyOnceWith(payload);
  expect(scopedCommand(payload, "device-A")).toEqual(payload);
  expect(
    screen.getByRole("region", { name: "Remove Alpha confirmation" }),
  ).toBeTruthy();
  expect(screen.getByRole("alert").textContent).toContain("did not complete");
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  expect(
    screen.queryByRole("region", { name: "Remove Alpha confirmation" }),
  ).toBeNull();
});
it("locks same-plugin lifecycle controls during removal and closes the confirmation only on success", async () => {
  let complete: (value: boolean) => void = () => {};
  const run = vi.fn(
      () => new Promise<boolean>((resolve) => (complete = resolve)),
    ),
    user = userEvent.setup();
  render(<Plugins snapshot={snapshot} run={run} onError={vi.fn()} />);
  await user.click(screen.getByRole("button", { name: "Remove Alpha" }));
  await user.click(
    screen.getByRole("button", { name: "Confirm removal of Alpha" }),
  );
  for (const name of [
    "Remove Alpha",
    "Disable Alpha",
    "Restart Alpha",
    "Cancel",
  ])
    expect(
      (screen.getByRole("button", { name }) as HTMLButtonElement).disabled,
    ).toBe(true);
  expect(
    (screen.getByRole("button", { name: "Disable Beta" }) as HTMLButtonElement)
      .disabled,
  ).toBe(false);
  await act(async () => complete(true));
  expect(
    screen.queryByRole("region", { name: "Remove Alpha confirmation" }),
  ).toBeNull();
});
it("explains external and missing removal modes without claiming their files are archived", async () => {
  const user = userEvent.setup();
  const view = render(
    <Plugins
      snapshot={snapshot}
      run={vi.fn(async () => true)}
      onError={vi.fn()}
    />,
  );
  await user.click(screen.getByRole("button", { name: "Remove Beta" }));
  expect(
    screen.getByText(/external plugin folder will be left untouched/),
  ).toBeTruthy();
  await user.keyboard("{Escape}");
  expect(
    screen.queryByRole("region", { name: "Remove Beta confirmation" }),
  ).toBeNull();
  view.rerender(
    <Plugins
      snapshot={{
        ...snapshot,
        plugins: [
          {
            ...plugin,
            removal_mode: "missing",
            removal_reason: "The folder is no longer present",
          },
        ],
      }}
      run={vi.fn(async () => true)}
      onError={vi.fn()}
    />,
  );
  await user.click(screen.getByRole("button", { name: "Remove Alpha" }));
  expect(
    screen.getByText(/unavailable plugin entry will be removed/),
  ).toBeTruthy();
  expect(screen.getByText("The folder is no longer present")).toBeTruthy();
});
it("retains assigned action settings and drafts after removing a plugin through the real panel", async () => {
  installFixture();
  const original = window.deckforge!.request;
  const request = vi.fn(original);
  window.deckforge!.request = request;
  const user = userEvent.setup();
  render(<App />);
  await screen.findByRole("button", { name: "Key 1: Counter" });
  await user.clear(screen.getByLabelText("Increment by"));
  await user.type(screen.getByLabelText("Increment by"), "4");
  await user.click(screen.getByRole("button", { name: /^Plugins/ }));
  await user.click(screen.getByRole("button", { name: "Remove Counter" }));
  await user.click(
    screen.getByRole("button", { name: "Confirm removal of Counter" }),
  );
  await waitFor(() =>
    expect(screen.queryByRole("button", { name: "Remove Counter" })).toBeNull(),
  );
  await user.click(screen.getByRole("button", { name: "Close Plugins" }));
  expect(screen.getByText(/Reinstall the plugin in Plugins/)).toBeTruthy();
  expect(
    (screen.getByLabelText("Increment by") as HTMLInputElement).value,
  ).toBe("4");
  expect(screen.getByRole("button", { name: "Key 1: Counter" })).toBeTruthy();
  expect(screen.queryByRole("button", { name: "Assign Counter" })).toBeNull();
  expect(request).toHaveBeenCalledWith("command", {
    command: "uninstall_plugin",
    id: "sample",
    expected_path: "/fixture/plugins/counter",
    expected_enabled: true,
    confirm: true,
  });
});
