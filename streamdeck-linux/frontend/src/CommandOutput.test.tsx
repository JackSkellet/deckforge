// @vitest-environment jsdom
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { CommandOutput } from "./CommandOutput";
import type { CommandRun, Slot } from "./model";
const request = vi.fn();
vi.mock("./bridge", () => ({ request: (...args: unknown[]) => request(...args) }));
const summary: CommandRun = { id: "run-1", state: "succeeded", message: "Command finished", stdout_bytes: 12, stderr_bytes: 0, truncated: false, can_copy: true };
const slot = { context: "command-A", command_run: summary } as Slot;
beforeEach(() => {
  request.mockReset();
  HTMLDialogElement.prototype.showModal = function () { this.setAttribute("open", ""); };
  HTMLDialogElement.prototype.close = function () { this.removeAttribute("open"); };
});
afterEach(cleanup);
it("loads output on demand as text and copies with captured device, context and run", async () => {
  const user = userEvent.setup(), submit = vi.fn((e) => e.preventDefault());
  request.mockResolvedValue({ summary, stdout: '<img src=x onerror="alert(1)">', stderr: "oops", preview_truncated: false });
  render(<form onSubmit={submit}><CommandOutput slot={slot} device="deck-A" /></form>);
  expect(request).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "Show output" }));
  expect(request).toHaveBeenCalledWith("command_output", { device: "deck-A", context: "command-A", id: "run-1" });
  expect(screen.getByLabelText("Standard output").textContent).toContain("<img");
  expect(screen.queryByRole("img")).toBeNull();
  await user.click(screen.getByRole("button", { name: "Close Command output" }));
  expect(submit).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "Copy output" }));
  expect(request).toHaveBeenLastCalledWith("command", { device: "deck-A", context: "command-A", id: "run-1", command: "command_copy_output" });
});
it("stops only the displayed running command and disables copying", async () => {
  request.mockResolvedValue({});
  render(<CommandOutput slot={{ ...slot, command_run: { ...summary, state: "running", can_copy: false } }} device="deck-A" />);
  expect((screen.getByRole("button", { name: "Copy output" }) as HTMLButtonElement).disabled).toBe(true);
  await userEvent.click(screen.getByRole("button", { name: "Stop command" }));
  expect(request).toHaveBeenCalledWith("command", { device: "deck-A", context: "command-A", id: "run-1", command: "command_cancel" });
});
it("ignores a late output reply after a newer run replaces it", async () => {
  let resolve!: (value: unknown) => void;
  request.mockImplementation(() => new Promise((r) => { resolve = r; }));
  const view = render(<CommandOutput slot={slot} device="deck-A" />);
  await userEvent.click(screen.getByRole("button", { name: "Show output" }));
  view.rerender(<CommandOutput slot={{ ...slot, command_run: { ...summary, id: "run-2", state: "running", can_copy: false } }} device="deck-A" />);
  await act(async () => resolve({ summary, stdout: "stale", stderr: "", preview_truncated: false }));
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
});
