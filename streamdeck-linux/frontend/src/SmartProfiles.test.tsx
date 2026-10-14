// @vitest-environment jsdom
import { useState } from "react";
import { afterEach, expect, it, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { SmartProfiles, type SmartDraft } from "./SmartProfiles";
import type { Profile } from "./model";
afterEach(() => {
  cleanup();
  delete window.deckforge;
});
it("chooses explicit observed Linux identities without mutating the saved baseline or imported target", async () => {
  const call = vi.fn(async (method: string) =>
    method === "application_catalog"
      ? [
          {
            id: "org.editor",
            name: "Editor",
            desktop: "/apps/editor.desktop",
            executable: "/bin/editor",
          },
          {
            id: "org.other",
            name: "Other editor",
            desktop: "/apps/other.desktop",
            executable: "/bin/other",
            window_class: "DeclaredClass",
          },
        ]
      : null,
  );
  window.deckforge = { request: call as any, onSnapshot: () => () => {} };
  HTMLDialogElement.prototype.showModal = function () {
    this.setAttribute("open", "");
  };
  HTMLDialogElement.prototype.close = function () {
    this.removeAttribute("open");
  };
  const initial: SmartDraft = {
    device: "A",
    deviceName: "Desk",
    profile: "target",
    profiles: [
      { id: "base", name: "Default", readonly: false },
      {
        id: "target",
        name: "Editor profile",
        readonly: true,
        app_identifier: "C:\\Editor.exe",
      },
    ] as Profile[],
    expected: { enabled: false, fallback: null, bindings: {} },
    settings: { enabled: false, fallback: null, bindings: {} },
  };
  let latest = initial;
  const saved = vi.fn(),
    closed = vi.fn();
  function Harness() {
    const [value, setValue] = useState(initial);
    return (
      <SmartProfiles
        draft={value}
        busy={false}
        error=""
        status={{
          config: initial.settings,
          backend: "KDE KWin",
          observing: true,
          paused: true,
          focus: null,
          error: null,
          recent: [
            {
              pid: 9,
              desktop_id: "",
              window_class: "Editor",
              executable: "/actual/editor",
            },
          ],
        }}
        onChange={(v) => {
          latest = v;
          setValue(v);
        }}
        onSave={saved}
        onClose={closed}
        onError={vi.fn()}
      />
    );
  }
  const view = render(<Harness />);
  const user = userEvent.setup();
  expect(screen.getByText(/C:\\Editor.exe/)).toBeTruthy();
  await user.click(
    screen.getByRole("checkbox", { name: "Switch profiles automatically" }),
  );
  await user.selectOptions(screen.getByLabelText("Default profile"), "base");
  await waitFor(() =>
    expect(screen.getByRole("radio", { name: "Editor" })).toBeTruthy(),
  );
  await user.click(screen.getByRole("radio", { name: "Editor" }));
  expect(latest.settings.bindings.target).toEqual({
    kind: "desktop_id",
    value: "org.editor",
    name: "Editor",
  });
  await user.click(screen.getByRole("radio", { name: "Other editor" }));
  expect(latest.settings.bindings.target).toEqual({
    kind: "window_class",
    value: "DeclaredClass",
    name: "Other editor",
  });
  await user.selectOptions(
    screen.getByLabelText("Recently focused application"),
    "executable:/actual/editor",
  );
  expect(latest.settings.bindings.target).toEqual({
    kind: "executable",
    value: "/actual/editor",
    name: "Editor",
  });
  expect(initial.expected).toEqual({
    enabled: false,
    fallback: null,
    bindings: {},
  });
  expect(initial.profiles[1].app_identifier).toBe("C:\\Editor.exe");
  await user.click(screen.getByRole("button", { name: "Save Smart Profiles" }));
  expect(saved).toHaveBeenCalledTimes(1);
  view.unmount();
  expect(call).toHaveBeenCalledWith("command", {
    command: "smart_preview",
    enabled: false,
  });
});
