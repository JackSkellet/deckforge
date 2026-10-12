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
import { DeviceNameForm } from "./Panels";
import { importFormDrafts } from "./transient";
import type { Snapshot } from "./model";

const snapshot = {
  device: "A",
  device_name: "Desk",
  device_model_name: "Stream Deck +",
  device_custom_name: "Desk",
} as Snapshot;
beforeEach(() => importFormDrafts({}));
afterEach(cleanup);

it("saves a captured device, resets to the model name, and disables repeated saves", async () => {
  const user = userEvent.setup();
  const run = vi.fn().mockResolvedValue(true);
  const { rerender } = render(<DeviceNameForm snapshot={snapshot} run={run} />);
  expect(
    (screen.getByRole("button", { name: "Save name" }) as HTMLButtonElement)
      .disabled,
  ).toBe(true);
  const field = screen.getByRole("textbox", { name: "Device name" });
  await user.clear(field);
  await user.type(field, "Studio");
  await user.keyboard("{Enter}");
  expect(run).toHaveBeenCalledWith({
    command: "device_rename",
    device: "A",
    name: "Studio",
  });
  rerender(
    <DeviceNameForm
      snapshot={{
        ...snapshot,
        device_custom_name: "Studio",
        device_name: "Studio",
      }}
      run={run}
    />,
  );
  await waitFor(() =>
    expect(
      (screen.getByRole("button", { name: "Save name" }) as HTMLButtonElement)
        .disabled,
    ).toBe(true),
  );
  await user.click(screen.getByRole("button", { name: "Use default name" }));
  expect(run).toHaveBeenLastCalledWith({
    command: "device_rename",
    device: "A",
    name: "",
  });
  expect((field as HTMLInputElement).value).toBe("");
});

it("retains per-device unsaved names when preferences are closed and reopened", async () => {
  const run = vi.fn().mockResolvedValue(false);
  const a = render(<DeviceNameForm snapshot={snapshot} run={run} />);
  fireEvent.change(screen.getByRole("textbox"), {
    target: { value: "A draft" },
  });
  a.unmount();
  const b = render(
    <DeviceNameForm
      snapshot={{ ...snapshot, device: "B", device_custom_name: "B name" }}
      run={run}
    />,
  );
  expect((screen.getByRole("textbox") as HTMLInputElement).value).toBe(
    "B name",
  );
  b.unmount();
  render(<DeviceNameForm snapshot={snapshot} run={run} />);
  expect((screen.getByRole("textbox") as HTMLInputElement).value).toBe(
    "A draft",
  );
  await userEvent.click(screen.getByRole("button", { name: "Save name" }));
  expect(screen.getByRole("status").textContent).toContain(
    "draft is preserved",
  );
  expect((screen.getByRole("textbox") as HTMLInputElement).value).toBe(
    "A draft",
  );
});

it("rejects long names while allowing 64 Unicode characters", () => {
  const run = vi.fn();
  render(<DeviceNameForm snapshot={snapshot} run={run} />);
  const field = screen.getByRole("textbox");
  fireEvent.change(field, { target: { value: "🎛".repeat(64) } });
  expect(field.getAttribute("aria-invalid")).toBe("false");
  fireEvent.change(field, { target: { value: "🎛".repeat(65) } });
  expect(field.getAttribute("aria-invalid")).toBe("true");
  expect(
    (screen.getByRole("button", { name: "Save name" }) as HTMLButtonElement)
      .disabled,
  ).toBe(true);
});
