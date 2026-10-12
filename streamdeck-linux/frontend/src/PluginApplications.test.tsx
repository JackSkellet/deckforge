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
import {
  PluginApplications,
  type MonitoredApplication,
} from "./PluginApplications";
import { exportFormDrafts, importFormDrafts } from "./transient";
import { scopedCommand } from "./bridge";
import { Plugins } from "./Panels";
import type { Snapshot } from "./model";
const applications: MonitoredApplication[] = [
  { application: "Example.exe", platforms: ["windows"], executable: "" },
  { application: "org.example.Editor", platforms: ["mac"], executable: "" },
  {
    application: "linux-editor",
    platforms: ["linux"],
    executable: "linux-editor",
  },
];
beforeEach(() => importFormDrafts({}));
afterEach(cleanup);
const input = (name = "Example.exe") =>
  screen.getByRole("textbox", {
    name: `Linux executable for ${name}`,
    hidden: true,
  }) as HTMLInputElement;
function expand() {
  fireEvent.click(screen.getByText("Monitored applications"));
}
it("applies explicit mappings globally, retains unknown overrides, and represents blank Linux overrides", async () => {
  const run = vi.fn(async () => true);
  const user = userEvent.setup();
  render(
    <PluginApplications
      id="plugin"
      applications={applications}
      mappings={{ retained: "/opt/keep" }}
      run={run}
    />,
  );
  expand();
  expect(input().value).toBe("");
  expect(screen.getAllByText("Not mapped")).toHaveLength(2);
  expect(input("linux-editor").value).toBe("linux-editor");
  await user.type(input(), "/usr/bin/editor");
  await user.clear(input("linux-editor"));
  await user.click(screen.getByRole("button", { name: "Apply mappings" }));
  const payload = {
    command: "plugin_applications",
    id: "plugin",
    mappings: {
      retained: "/opt/keep",
      "Example.exe": "/usr/bin/editor",
      "linux-editor": "",
    },
    baseline: { retained: "/opt/keep" },
  };
  expect(run).toHaveBeenCalledWith(payload);
  expect(scopedCommand(payload, "selected-device")).toEqual(payload);
  expect(await screen.findByText("Application mappings saved")).toBeTruthy();
});
it("keeps dirty edits through snapshot changes and native recreation until explicit reload", async () => {
  const run = vi.fn(async () => true);
  const user = userEvent.setup();
  const props = { id: "plugin", applications, mappings: {}, run };
  const view = render(<PluginApplications {...props} />);
  expand();
  await user.type(input(), "my-editor");
  const updated = {
    ...props,
    applications: [
      { ...applications[0], executable: "remote-editor" },
      ...applications.slice(1),
    ],
    mappings: { "Example.exe": "remote-editor" },
  };
  view.rerender(<PluginApplications {...updated} />);
  expect(input().value).toBe("my-editor");
  expect(
    screen.getByText(/Saved mappings changed while you were editing/),
  ).toBeTruthy();
  expect(
    (
      screen.getByRole("button", {
        name: "Apply mappings",
      }) as HTMLButtonElement
    ).disabled,
  ).toBe(true);
  const persisted = structuredClone(exportFormDrafts());
  view.unmount();
  importFormDrafts(persisted);
  render(<PluginApplications {...updated} />);
  await waitFor(() => expect(input().value).toBe("my-editor"));
  expect(document.querySelector("details")?.open).toBe(true);
  await user.click(screen.getByRole("button", { name: "Reload mappings" }));
  expect(input().value).toBe("remote-editor");
  expect(run).not.toHaveBeenCalled();
  expect(
    (
      screen.getByRole("button", {
        name: "Apply mappings",
      }) as HTMLButtonElement
    ).disabled,
  ).toBe(true);
});
it("retains failed drafts, refreshes clean maps, and captures the original plugin while saving", async () => {
  const run = vi.fn(async () => false);
  const user = userEvent.setup();
  const view = render(
    <PluginApplications
      id="plugin-A"
      applications={applications}
      mappings={{}}
      run={run}
    />,
  );
  expand();
  await user.type(input(), "editor-A");
  await user.click(screen.getByRole("button", { name: "Apply mappings" }));
  expect(input().value).toBe("editor-A");
  expect(screen.getByText(/Mappings were not saved/)).toBeTruthy();
  expect(run).toHaveBeenCalledWith({
    command: "plugin_applications",
    id: "plugin-A",
    mappings: { "Example.exe": "editor-A" },
    baseline: {},
  });
  await user.click(screen.getByRole("button", { name: "Revert" }));
  view.rerender(
    <PluginApplications
      id="plugin-A"
      applications={applications}
      mappings={{ "Example.exe": "external" }}
      run={run}
    />,
  );
  await waitFor(() => expect(input().value).toBe("external"));
});

it("shows partial observer warnings without disabling mappings or claiming all monitoring stopped", () => {
  const snapshot = {
    plugins: [],
    application_monitor: {
      enabled: true,
      error: "Some process identities could not be read",
    },
  } as unknown as Snapshot;
  const props = { snapshot, run: vi.fn(async () => true), onError: vi.fn() };
  const view = render(<Plugins {...props} />);
  expect(
    screen.getByText(
      "Application monitoring: Some process identities could not be read",
    ),
  ).toBeTruthy();
  view.rerender(
    <Plugins
      {...props}
      snapshot={{
        ...snapshot,
        application_monitor: { enabled: true, error: null },
      }}
    />,
  );
  expect(screen.queryByText(/Application monitoring:/)).toBeNull();
});
