// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { PluginSetup } from "./PluginSetup";
import { scopedCommand } from "./bridge";
afterEach(() => {
  cleanup();
  delete window.deckforge;
});
const plugin = {
  id: "test.plugin",
  name: "Example",
  connected: true,
  setup_actions: [
    { uuid: "test.one", name: "Account" },
    { uuid: "test.two", name: "Options" },
  ],
};
it("opens a chosen setup page without assigning a key", async () => {
  const user = userEvent.setup(),
    run = vi.fn().mockResolvedValue(true);
  render(<PluginSetup plugin={plugin} run={run} busy={false} />);
  await user.click(screen.getByRole("button", { name: "Configure Example" }));
  await user.selectOptions(screen.getByRole("combobox"), "test.two");
  await user.click(
    screen.getByRole("button", { name: "Open plugin settings" }),
  );
  const payload = {
    command: "plugin_setup",
    id: "test.plugin",
    uuid: "test.two",
  };
  expect(run).toHaveBeenCalledExactlyOnceWith(payload);
  expect(scopedCommand(payload, "another-device")).toEqual(payload);
  expect(screen.getByRole("status").textContent).toContain("separate window");
});
it("requires a connected enabled plugin and explains missing pages", async () => {
  const user = userEvent.setup(),
    run = vi.fn();
  const view = render(
    <PluginSetup
      plugin={{ ...plugin, enabled: false }}
      run={run}
      busy={false}
    />,
  );
  await user.click(screen.getByRole("button", { name: "Configure Example" }));
  expect(
    (
      screen.getByRole("button", {
        name: "Open plugin settings",
      }) as HTMLButtonElement
    ).disabled,
  ).toBe(true);
  view.rerender(
    <PluginSetup
      plugin={{ ...plugin, setup_actions: [] }}
      run={run}
      busy={false}
    />,
  );
  expect(
    screen.getByText("This plugin does not provide a settings page."),
  ).toBeTruthy();
  expect(run).not.toHaveBeenCalled();
});
it("installs OBS separately and reports the verified bridge connection", async () => {
  const user = userEvent.setup();
  const request = vi
    .fn()
    .mockResolvedValueOnce({
      available: true,
      installed: false,
      connection: null,
    })
    .mockResolvedValueOnce({
      available: true,
      installed: true,
      connection: null,
    })
    .mockResolvedValueOnce({
      available: true,
      installed: true,
      connection: { obs_version: "32.2.1", bridge_version: "5.5.4" },
    });
  window.deckforge = { request, onSnapshot: () => () => {} };
  render(
    <PluginSetup
      plugin={{ ...plugin, id: "com.elgato.obsstudio" }}
      run={vi.fn()}
      busy={false}
    />,
  );
  await user.click(screen.getByRole("button", { name: "Configure Example" }));
  await user.click(
    await screen.findByRole("button", { name: "Install OBS bridge" }),
  );
  await waitFor(() =>
    expect(screen.getByRole("status").textContent).toContain(
      "Fully quit and reopen OBS",
    ),
  );
  expect(request).toHaveBeenNthCalledWith(2, "obs_bridge", {
    operation: "install",
  });
  await user.click(
    screen.getByRole("button", { name: "Check OBS connection" }),
  );
  await waitFor(() =>
    expect(screen.getByRole("status").textContent).toContain(
      "OBS 32.2.1 connected",
    ),
  );
});
