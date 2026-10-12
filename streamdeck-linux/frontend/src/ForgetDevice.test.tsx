// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ForgetDevice } from "./Panels";
import type { Snapshot } from "./model";

const offline = {
  device: "A",
  device_name: "Studio",
  device_online: false,
  connected: false,
  library: { profiles: [{ id: "one" }, { id: "two" }] },
} as Snapshot;
afterEach(cleanup);

it("requires confirmation, supports keyboard cancel, and submits the captured device", async () => {
  const user = userEvent.setup();
  const run = vi.fn().mockResolvedValue(true);
  render(<ForgetDevice snapshot={offline} run={run} />);
  await user.click(screen.getByRole("button", { name: "Forget device…" }));
  expect(run).not.toHaveBeenCalled();
  expect(screen.getByRole("alert").textContent).toContain(
    "Studio and remove 2 saved profiles",
  );
  expect(document.activeElement).toBe(
    screen.getByRole("button", { name: "Cancel" }),
  );
  await user.keyboard("{Enter}");
  expect(document.activeElement).toBe(
    screen.getByRole("button", { name: "Forget device…" }),
  );
  expect(run).not.toHaveBeenCalled();
  await user.keyboard("{Enter}");
  await user.click(
    screen.getByRole("button", { name: "Forget device and profiles" }),
  );
  expect(run).toHaveBeenCalledExactlyOnceWith({
    command: "device_forget",
    device: "A",
    confirm: true,
  });
});

it("blocks connected devices and rechecks connection while the confirmation is open", async () => {
  const run = vi.fn();
  const { rerender } = render(
    <ForgetDevice snapshot={{ ...offline, device_online: true }} run={run} />,
  );
  expect((screen.getByRole("button") as HTMLButtonElement).disabled).toBe(true);
  rerender(<ForgetDevice snapshot={offline} run={run} />);
  await userEvent.click(screen.getByRole("button"));
  rerender(
    <ForgetDevice snapshot={{ ...offline, device_online: true }} run={run} />,
  );
  expect(
    (
      screen.getByRole("button", {
        name: "Forget device and profiles",
      }) as HTMLButtonElement
    ).disabled,
  ).toBe(true);
  expect(run).not.toHaveBeenCalled();
});

it("keeps the confirmation open with an error when the host refuses removal", async () => {
  const run = vi.fn().mockResolvedValue(false);
  render(<ForgetDevice snapshot={offline} run={run} />);
  await userEvent.click(screen.getByRole("button"));
  await userEvent.click(
    screen.getByRole("button", { name: "Forget device and profiles" }),
  );
  await waitFor(() =>
    expect(screen.getByRole("status").textContent).toContain(
      "Could not forget",
    ),
  );
  expect(
    (
      screen.getByRole("button", {
        name: "Forget device and profiles",
      }) as HTMLButtonElement
    ).disabled,
  ).toBe(false);
});
