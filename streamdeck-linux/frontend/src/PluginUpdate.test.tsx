// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { PluginUpdate, type PluginUpdateReview } from "./PluginUpdate";
import { Plugins } from "./Panels";
import { exportFormDrafts, importFormDrafts } from "./transient";
import { scopedCommand } from "./bridge";
import type { Action, Snapshot } from "./model";
const plugin = {
  id: "a",
  name: "Alpha",
  path: "/state/plugins/a.sdPlugin",
  version: "1.0",
  removal_mode: "archive",
  enabled: false,
  rollback_available: true,
  rollback_version: "0.9",
};
const review: PluginUpdateReview = {
  ticket: "ticket-1",
  id: "a",
  name: "Alpha",
  current_version: "1.0",
  new_version: "2.0",
  comparison: "upgrade",
  removed_actions: ["a.old"],
  added_actions: ["a.new"],
  was_enabled: false,
  kind: "update",
};
const actions = [{ uuid: "a.old", name: "Old action" }] as Action[];
beforeEach(() => importFormDrafts({}));
afterEach(() => {
  cleanup();
  delete window.deckforge;
});
function setup(request: (method: string, payload?: any) => Promise<any>) {
  window.deckforge = { request, onSnapshot: () => () => {} };
}
it("captures plugin path/version before the picker and reviews changes without applying automatically", async () => {
  let choose: (value: string) => void = () => {};
  const request = vi.fn(async (method: string) =>
    method === "pick_file"
      ? new Promise((resolve) => (choose = resolve))
      : review,
  );
  setup(request);
  const run = vi.fn(async () => true),
    onBusy = vi.fn(),
    user = userEvent.setup();
  const view = render(
    <PluginUpdate
      plugin={plugin}
      actions={actions}
      busy={false}
      onBusy={onBusy}
      run={run}
    />,
  );
  await user.click(
    screen.getByRole("button", { name: "Update from package…" }),
  );
  view.rerender(
    <PluginUpdate
      plugin={{ ...plugin, path: "/new/path", version: "1.5" }}
      actions={actions}
      busy={false}
      onBusy={onBusy}
      run={run}
    />,
  );
  await act(async () => choose("/downloads/update.streamDeckPlugin"));
  expect(request).toHaveBeenCalledWith("plugin_update_prepare", {
    id: "a",
    path: "/downloads/update.streamDeckPlugin",
    expected_path: plugin.path,
    expected_version: "1.0",
  });
  expect(screen.getByText("1.0 → 2.0")).toBeTruthy();
  expect(screen.getByText("Old action · a.old")).toBeTruthy();
  expect(screen.getByText("a.new")).toBeTruthy();
  expect(screen.getByText(/will remain disabled/)).toBeTruthy();
  expect(run).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "Confirm update" }));
  const payload = {
    command: "plugin_update_apply",
    ticket: "ticket-1",
    confirm: true,
  };
  expect(run).toHaveBeenCalledExactlyOnceWith(payload);
  expect(scopedCommand(payload, "device")).toEqual(payload);
  expect(
    screen.queryByRole("region", { name: "Review Alpha update" }),
  ).toBeNull();
});
it("retains review across view recreation and failed Apply, then cancels its exact ticket", async () => {
  importFormDrafts({ "plugin-update:a": review });
  const run = vi.fn(async () => false),
    user = userEvent.setup();
  const view = render(
    <PluginUpdate
      plugin={plugin}
      actions={actions}
      busy={false}
      onBusy={vi.fn()}
      run={run}
    />,
  );
  await user.click(screen.getByRole("button", { name: "Confirm update" }));
  expect(screen.getByRole("alert").textContent).toContain("did not complete");
  expect(exportFormDrafts()["plugin-update:a"].ticket).toBe("ticket-1");
  const persisted = structuredClone(exportFormDrafts());
  view.unmount();
  importFormDrafts(persisted);
  run.mockResolvedValue(true);
  render(
    <PluginUpdate
      plugin={plugin}
      actions={actions}
      busy={false}
      onBusy={vi.fn()}
      run={run}
    />,
  );
  await user.click(screen.getByRole("button", { name: "Cancel review" }));
  expect(run).toHaveBeenLastCalledWith({
    command: "plugin_update_cancel",
    ticket: "ticket-1",
  });
  expect(exportFormDrafts()["plugin-update:a"]).toBeNull();
});
it("prepares rollback without a picker and explicitly labels a downgrade", async () => {
  const request = vi.fn(async () => ({
    ...review,
    kind: "rollback",
    comparison: "downgrade",
    new_version: "0.9",
  }));
  setup(request);
  const user = userEvent.setup(),
    run = vi.fn(async () => true);
  render(
    <PluginUpdate
      plugin={plugin}
      actions={actions}
      busy={false}
      onBusy={vi.fn()}
      run={run}
    />,
  );
  await user.click(
    screen.getByRole("button", { name: "Restore version 0.9…" }),
  );
  expect(request).toHaveBeenCalledExactlyOnceWith("plugin_rollback_prepare", {
    id: "a",
    expected_path: plugin.path,
    expected_version: "1.0",
  });
  expect(screen.getByText("This is an older version.")).toBeTruthy();
  await user.click(screen.getByRole("button", { name: "Confirm restore" }));
  expect(run).toHaveBeenCalledWith({
    command: "plugin_update_apply",
    ticket: "ticket-1",
    confirm: true,
  });
});
it("leaves external folders outside the package updater and preserves prior review on prepare errors", async () => {
  const user = userEvent.setup(),
    request = vi.fn(async () => {
      throw Error("Package validation failed");
    });
  setup(request);
  const props = {
    actions,
    busy: false,
    onBusy: vi.fn(),
    run: vi.fn(async () => true),
  };
  const view = render(
    <PluginUpdate {...props} plugin={{ ...plugin, removal_mode: "detach" }} />,
  );
  expect(screen.getByText(/Update unavailable/)).toBeTruthy();
  expect(
    screen.queryByRole("button", { name: "Update from package…" }),
  ).toBeNull();
  view.rerender(<PluginUpdate {...props} plugin={plugin} />);
  await user.click(
    screen.getByRole("button", { name: "Update from package…" }),
  );
  expect(await screen.findByRole("alert")).toHaveProperty(
    "textContent",
    "Package validation failed",
  );
  expect(props.run).not.toHaveBeenCalled();
});
it("locks lifecycle/removal controls only for the plugin whose package is being prepared", async () => {
  let choose: (value: null) => void = () => {};
  setup(async () => new Promise((resolve) => (choose = resolve)));
  const user = userEvent.setup();
  const snapshot = {
    plugins: [plugin, { ...plugin, id: "b", name: "Beta" }],
    actions: [],
  } as unknown as Snapshot;
  render(
    <Plugins
      snapshot={snapshot}
      run={vi.fn(async () => true)}
      onError={vi.fn()}
    />,
  );
  await user.click(
    screen.getAllByRole("button", { name: "Update from package…" })[0],
  );
  expect(
    (screen.getByRole("button", { name: "Enable Alpha" }) as HTMLButtonElement)
      .disabled,
  ).toBe(true);
  expect(
    (screen.getByRole("button", { name: "Remove Alpha" }) as HTMLButtonElement)
      .disabled,
  ).toBe(true);
  expect(
    (screen.getByRole("button", { name: "Enable Beta" }) as HTMLButtonElement)
      .disabled,
  ).toBe(false);
  await act(async () => choose(null));
  await waitFor(() =>
    expect(
      (
        screen.getByRole("button", {
          name: "Enable Alpha",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(false),
  );
});

it.each([
  ["same", "Same version: this replaces the current package contents."],
  ["unknown", "These version numbers could not be compared."],
] as const)("labels %s version review explicitly", (comparison, message) => {
  importFormDrafts({ "plugin-update:a": { ...review, comparison } });
  render(
    <PluginUpdate
      plugin={plugin}
      actions={actions}
      busy={false}
      onBusy={vi.fn()}
      run={vi.fn(async () => true)}
    />,
  );
  expect(screen.getByText(message)).toBeTruthy();
});
