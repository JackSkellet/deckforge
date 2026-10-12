// @vitest-environment jsdom
import { useState } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ReferenceInstaller } from "./ReferenceInstaller";
import { importFormDrafts } from "./transient";
const prior = "/private/existing/AppExecutable";
beforeEach(() => importFormDrafts({}));
afterEach(() => {
  cleanup();
  delete window.deckforge;
});
function setup(request: (method: string, payload?: unknown) => Promise<any>) {
  window.deckforge = { request, onSnapshot: () => () => {} };
}
function Harness({
  configured = prior,
  run = async () => true,
}: {
  configured?: string;
  run?: (payload: any) => Promise<boolean>;
}) {
  const [reference, setReference] = useState(prior);
  return (
    <ReferenceInstaller
      configuredPath={configured}
      reference={reference}
      onReferenceChange={setReference}
      run={run}
    />
  );
}
it("picks an installer and locks reference changes until validated import returns Ready metadata", async () => {
  let complete: (value: any) => void = () => {};
  const request = vi.fn(async (method: string) =>
    method === "pick_file"
      ? "/downloads/Stream_Deck.pkg"
      : new Promise((resolve) => {
          complete = resolve;
        }),
  );
  setup(request);
  const user = userEvent.setup();
  render(<Harness />);
  await user.click(screen.getByText("Advanced: use an extracted executable"));
  await user.click(
    screen.getByRole("button", { name: "Import Elgato installer" }),
  );
  expect(request).toHaveBeenCalledWith("pick_file", {
    kind: "reference_installer",
  });
  expect(request).toHaveBeenCalledWith("reference_import", {
    path: "/downloads/Stream_Deck.pkg",
    expected: prior,
  });
  expect(
    (
      screen.getByRole("button", {
        name: "Clear reference",
      }) as HTMLButtonElement
    ).disabled,
  ).toBe(true);
  expect(
    screen.getByRole("button", { name: "Save reference" }).matches(":disabled"),
  ).toBe(true);
  expect(
    (
      screen.getByLabelText(
        "Original Stream Deck executable",
      ) as HTMLInputElement
    ).value,
  ).toBe(prior);
  await act(async () =>
    complete({
      path: "/private/cache/StreamDeck",
      platform: "macos",
      version: "7.6.0.23012",
    }),
  );
  expect(
    await screen.findByText("Ready · Stream Deck 7.6.0.23012 · macOS"),
  ).toBeTruthy();
  expect(
    (
      screen.getByLabelText(
        "Original Stream Deck executable",
      ) as HTMLInputElement
    ).value,
  ).toBe("/private/cache/StreamDeck");
  expect(screen.getByText(/without running vendor code/)).toBeTruthy();
  expect(screen.getByText(/No account login is required/)).toBeTruthy();
});
it("preserves the prior reference and exposes import errors without requiring manual typing", async () => {
  const request = vi.fn(async (method: string) => {
    if (method === "pick_file") return "/downloads/bad.pkg";
    throw Error("Unsupported installer archive");
  });
  setup(request);
  const run = vi.fn(async () => true),
    user = userEvent.setup();
  render(<Harness run={run} />);
  await user.click(
    screen.getByRole("button", { name: "Import Elgato installer" }),
  );
  expect(await screen.findByRole("alert")).toHaveProperty(
    "textContent",
    "Unsupported installer archive",
  );
  expect(
    (
      screen.getByLabelText(
        "Original Stream Deck executable",
      ) as HTMLInputElement
    ).value,
  ).toBe(prior);
  expect(
    screen.getByText(
      "Configured · ready for local protected-package verification",
    ),
  ).toBeTruthy();
  expect(run).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "Clear reference" }));
  expect(run).toHaveBeenCalledWith({ command: "reference_app", path: null });
});
it("captures the prior path before the native picker and treats cancellation as no change", async () => {
  let finish: (value: string | null) => void = () => {};
  const request = vi.fn(async (method: string) => {
    if (method === "pick_file")
      return new Promise((resolve) => {
        finish = resolve;
      });
    return { path: "/private/cache/new", version: "7.6", platform: "windows" };
  });
  setup(request);
  const user = userEvent.setup();
  const view = render(<Harness />);
  await user.click(
    screen.getByRole("button", { name: "Import Elgato installer" }),
  );
  view.rerender(<Harness configured="/external/new-reference" />);
  await act(async () => finish("/downloads/chosen.pkg"));
  expect(request).toHaveBeenCalledWith("reference_import", {
    path: "/downloads/chosen.pkg",
    expected: prior,
  });
  request.mockClear();
  await user.click(
    screen.getByRole("button", { name: "Import Elgato installer" }),
  );
  await act(async () => finish(null));
  expect(request).toHaveBeenCalledTimes(1);
  await waitFor(() =>
    expect(
      (
        screen.getByRole("button", {
          name: "Import Elgato installer",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(false),
  );
});
it("keeps the direct executable fallback and saves it through the existing global command", async () => {
  const request = vi.fn(async () => "/owned/extracted/AppExecutable");
  setup(request);
  const run = vi.fn(async () => true),
    user = userEvent.setup();
  render(<Harness run={run} />);
  await user.click(screen.getByText("Advanced: use an extracted executable"));
  await user.click(screen.getByRole("button", { name: "Choose executable…" }));
  expect(request).toHaveBeenCalledWith("pick_file", { kind: "file" });
  await user.click(screen.getByRole("button", { name: "Save reference" }));
  expect(run).toHaveBeenCalledWith({
    command: "reference_app",
    path: "/owned/extracted/AppExecutable",
  });
});
