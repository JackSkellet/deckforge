// @vitest-environment jsdom
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { BundledPlugins } from "./BundledPlugins";
const bundle = {
  id: "com.elgato.discord",
  name: "Discord for Deckforge",
  version: "0.1.0",
  path: "/distribution/bundled-plugins/discord.streamDeckPlugin",
};
afterEach(() => {
  cleanup();
  delete window.deckforge;
});
it("installs a bundled package through the normal lifecycle without passing account data", async () => {
  window.deckforge = {
    request: vi.fn().mockResolvedValue([bundle]),
    onSnapshot: () => () => {},
  };
  const run = vi.fn().mockResolvedValue(true);
  const view = render(<BundledPlugins installed={[]} run={run} />);
  fireEvent.click(
    await screen.findByRole("button", {
      name: "Install Discord for Deckforge",
    }),
  );
  await waitFor(() =>
    expect(run).toHaveBeenCalledWith({
      command: "install_plugin",
      path: bundle.path,
    }),
  );
  view.rerender(<BundledPlugins installed={[bundle.id]} run={run} />);
  expect(
    (
      screen.getByRole("button", {
        name: "Already installed",
      }) as HTMLButtonElement
    ).disabled,
  ).toBe(true);
});
it("reports a damaged bundle and leaves existing installations alone", async () => {
  window.deckforge = {
    request: vi.fn().mockRejectedValue(new Error("Invalid bundled manifest")),
    onSnapshot: () => () => {},
  };
  const run = vi.fn();
  render(<BundledPlugins installed={[bundle.id]} run={run} />);
  await screen.findByRole("alert");
  expect(screen.getByRole("alert").textContent).toContain(
    "Invalid bundled manifest",
  );
  expect(run).not.toHaveBeenCalled();
});
