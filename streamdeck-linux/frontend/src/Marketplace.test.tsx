// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Marketplace } from "./Marketplace";
import type { MarketplaceState } from "./model";
const signedOut: MarketplaceState = {
  busy: false,
  authenticated: false,
  account: null,
  message: "",
  error: null,
  pending: null,
  review: null,
};
afterEach(() => {
  cleanup();
  delete window.deckforge;
});
it("opens sign-in through the native bridge and uses host state for authentication", async () => {
  const request = vi.fn(async (): Promise<any> => null);
  window.deckforge = { request, onSnapshot: () => () => {} };
  const view = render(<Marketplace state={signedOut} />);
  await userEvent.click(
    screen.getByRole("button", { name: "Sign in to Marketplace" }),
  );
  expect(request).toHaveBeenCalledWith("marketplace", { action: "login" });
  expect(screen.getByText("Not signed in")).toBeTruthy();
  view.rerender(
    <Marketplace
      state={{ ...signedOut, busy: true, message: "Complete sign-in" }}
    />,
  );
  expect(
    (
      screen.getByRole("button", {
        name: "Sign in to Marketplace",
      }) as HTMLButtonElement
    ).disabled,
  ).toBe(true);
  view.rerender(
    <Marketplace
      state={{ ...signedOut, authenticated: true, account: "Example" }}
    />,
  );
  expect(screen.getByText("Signed in · Example")).toBeTruthy();
});
it("retains a failed install for retry and reports bridge failures", async () => {
  const request = vi.fn(async () => {
    throw Error("Request already in progress");
  });
  window.deckforge = { request, onSnapshot: () => () => {} };
  render(
    <Marketplace
      state={{ ...signedOut, pending: "variant", error: "Session expired" }}
    />,
  );
  expect(screen.getByRole("alert").textContent).toBe("Session expired");
  await userEvent.click(
    screen.getByRole("button", { name: "Retry plugin install" }),
  );
  await waitFor(() =>
    expect(screen.getByRole("alert").textContent).toBe(
      "Request already in progress",
    ),
  );
  expect(request).toHaveBeenCalledWith("marketplace", { action: "retry" });
});
it("reviews a downloaded update before applying and supports cancellation", async () => {
  const request = vi.fn(async (): Promise<any> => null);
  window.deckforge = { request, onSnapshot: () => () => {} };
  render(
    <Marketplace
      state={{
        ...signedOut,
        review: {
          ticket: "ticket",
          id: "plugin",
          name: "Music",
          current_version: "2",
          new_version: "1",
          comparison: "downgrade",
          removed_actions: ["plugin.removed"],
          added_actions: [],
          was_enabled: false,
          kind: "update",
        },
      }}
    />,
  );
  expect(screen.getByText(/older version/)).toBeTruthy();
  expect(screen.getByText("plugin.removed")).toBeTruthy();
  expect(
    (
      screen.getByRole("button", {
        name: "Sign out on this device",
      }) as HTMLButtonElement
    ).disabled,
  ).toBe(true);
  await userEvent.click(screen.getByRole("button", { name: "Install update" }));
  expect(request).toHaveBeenCalledWith("marketplace", { action: "apply" });
  await userEvent.click(screen.getByRole("button", { name: "Cancel update" }));
  expect(request).toHaveBeenCalledWith("marketplace", { action: "cancel" });
});
