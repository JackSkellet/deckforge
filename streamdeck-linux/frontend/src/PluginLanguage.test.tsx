// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { PluginLanguage } from "./PluginLanguage";
import { Plugins } from "./Panels";
import type { PluginLanguageState, Snapshot } from "./model";
import { exportFormDrafts, importFormDrafts } from "./transient";
import { scopedCommand } from "./bridge";
const language: PluginLanguageState = {
  preference: "system",
  active: "en",
  resolved: "en",
  restart_required: false,
  options: [
    { value: "system", label: "System" },
    { value: "en", label: "English" },
    { value: "de", label: "Deutsch" },
    { value: "zh_TW", label: "Traditional Chinese" },
  ],
};
beforeEach(() => importFormDrafts({}));
afterEach(cleanup);
it("saves only the global pending language preference and shows the full restart boundary", async () => {
  const run = vi.fn(async () => true);
  const user = userEvent.setup();
  const view = render(<PluginLanguage language={language} run={run} />);
  await user.selectOptions(
    screen.getByRole("combobox", { name: "Plugin language" }),
    "de",
  );
  expect(run).not.toHaveBeenCalled();
  await user.click(
    screen.getByRole("button", { name: "Save plugin language" }),
  );
  const payload = {
    command: "plugin_language",
    value: "de",
    baseline: "system",
  };
  expect(run).toHaveBeenCalledExactlyOnceWith(payload);
  expect(scopedCommand(payload, "device-A")).toEqual(payload);
  view.rerender(
    <PluginLanguage
      language={{
        ...language,
        preference: "de",
        resolved: "de",
        restart_required: true,
      }}
      run={run}
    />,
  );
  expect(
    screen.getByText("Restart Deckforge to apply the saved plugin language."),
  ).toBeTruthy();
  expect(
    screen.getByText(
      "Running plugins: English. Saved selection resolves to: Deutsch.",
    ),
  ).toBeTruthy();
  expect(screen.getByText(/Deckforge menus remain English/)).toBeTruthy();
});
it("retains language drafts across native recreation and requires explicit reload after a stale preference", async () => {
  const run = vi.fn(async () => true);
  const user = userEvent.setup();
  const view = render(<PluginLanguage language={language} run={run} />);
  await user.selectOptions(screen.getByRole("combobox"), "zh_TW");
  const stored = structuredClone(exportFormDrafts());
  view.unmount();
  importFormDrafts(stored);
  render(
    <PluginLanguage
      language={{
        ...language,
        preference: "de",
        resolved: "de",
        restart_required: true,
      }}
      run={run}
    />,
  );
  expect((screen.getByRole("combobox") as HTMLSelectElement).value).toBe(
    "zh_TW",
  );
  expect(
    (
      screen.getByRole("button", {
        name: "Save plugin language",
      }) as HTMLButtonElement
    ).disabled,
  ).toBe(true);
  await user.click(screen.getByRole("button", { name: "Reload language" }));
  expect((screen.getByRole("combobox") as HTMLSelectElement).value).toBe("de");
  expect(run).not.toHaveBeenCalled();
});
it("keeps failed language saves as drafts and accepts refreshed clean preferences", async () => {
  const run = vi.fn(async () => false);
  const user = userEvent.setup();
  const view = render(<PluginLanguage language={language} run={run} />);
  await user.selectOptions(screen.getByRole("combobox"), "de");
  await user.click(
    screen.getByRole("button", { name: "Save plugin language" }),
  );
  expect(screen.getByText(/Language preference was not saved/)).toBeTruthy();
  expect(exportFormDrafts()["plugin-language"].dirty).toBe(true);
  await user.click(screen.getByRole("button", { name: "Revert language" }));
  view.rerender(
    <PluginLanguage language={{ ...language, preference: "en" }} run={run} />,
  );
  await waitFor(() =>
    expect((screen.getByRole("combobox") as HTMLSelectElement).value).toBe(
      "en",
    ),
  );
});
it("shows localization warnings in runtime details without changing connected plugin status", () => {
  const snapshot = {
    plugins: [
      {
        id: "example",
        name: "Example",
        connected: true,
        localization_warning:
          "A translation file is invalid; original labels are retained",
      },
    ],
  } as unknown as Snapshot;
  render(
    <Plugins
      snapshot={snapshot}
      run={vi.fn(async () => true)}
      onError={vi.fn()}
    />,
  );
  expect(
    screen.getByText(
      "Localization: A translation file is invalid; original labels are retained",
    ),
  ).toBeTruthy();
  expect(screen.getByText("Connected").className).toContain("good");
});
