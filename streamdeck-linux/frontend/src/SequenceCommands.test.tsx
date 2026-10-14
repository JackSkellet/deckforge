// @vitest-environment jsdom
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { Sequence } from "./Sequence";
import { LOGIC, TRIGGER, type RecordValue, type Slot } from "./model";
const request = vi.fn();
vi.mock("./bridge", () => ({ request: (...args: unknown[]) => request(...args) }));
const COMMAND = "com.deckforge.system.command", MULTI = "com.elgato.streamdeck.multiactions.routine", SWITCH = "com.elgato.streamdeck.multiactions.routine2";
const leaf = (name: string, id?: string): RecordValue => ({ UUID: COMMAND, Name: name, ...(id ? { ActionID: id } : {}), Settings: { mode: "shell", script: "true", output: "show" } });
const live = (raw: RecordValue, context: string): Slot => ({ uuid: COMMAND, raw, context, name: raw.Name, command_run: { id: `run-${context}`, state: "running", message: "Command running", can_copy: false, stdout_bytes: 0, stderr_bytes: 0, truncated: false } } as Slot);
beforeEach(() => request.mockReset().mockResolvedValue({}));
afterEach(cleanup);

it.each([MULTI, SWITCH, TRIGGER, LOGIC])("routes Stop from a read-only %s child to its saved command", async (uuid) => {
  const child = leaf("Nested command", "child");
  const trigger = uuid === TRIGGER || uuid === LOGIC;
  const raw = { Actions: trigger ? [child, {}, {}] : [{ Actions: [child] }, { Actions: [] }] };
  const runtime = { uuid, raw, context: "parent", ...(trigger ? { trigger_children: [live(child, "saved-child"), null, null] } : { multi_children: [[live(child, "saved-child")], []] }) } as unknown as Slot;
  const onChange = vi.fn();
  render(<Sequence raw={raw} uuid={uuid} runtime={runtime} device="deck-A" readOnly actions={[]} outputs={[]} onChange={onChange} onError={vi.fn()} />);
  await userEvent.click(document.querySelector('.sequence-select')!);
  expect(screen.getByLabelText("Action label").matches(":disabled")).toBe(true);
  await userEvent.click(screen.getByRole("button", { name: "Stop command" }));
  expect(request).toHaveBeenCalledWith("command", { device: "deck-A", context: "saved-child", id: "run-saved-child", command: "command_cancel" });
  expect(onChange).not.toHaveBeenCalled();
});

it("keeps a stable command owner after an unsaved reorder and suppresses ambiguous legacy children", async () => {
  const first = leaf("First", "one"), second = leaf("Second", "two");
  const runtime = { uuid: MULTI, raw: { Actions: [{ Actions: [first, second] }] }, context: "parent", multi_children: [[live(first, "first"), live(second, "second")]] } as unknown as Slot;
  const props = { uuid: MULTI, device: "deck-A", runtime, actions: [], outputs: [], onChange: vi.fn(), onError: vi.fn(), clipboardReady: false };
  const view = render(<Sequence {...props} raw={{ Actions: [{ Actions: [second, first] }] }} />);
  await userEvent.click(document.querySelector('.sequence-select')!);
  await userEvent.click(screen.getByRole("button", { name: "Stop command" }));
  expect(request).toHaveBeenLastCalledWith("command", { device: "deck-A", context: "second", id: "run-second", command: "command_cancel" });
  const legacyFirst = leaf("First"), legacySecond = leaf("Second");
  view.rerender(<Sequence {...props} raw={{ Actions: [{ Actions: [legacySecond, legacyFirst] }] }} runtime={{ ...runtime, multi_children: [[live(legacyFirst, "first"), live(legacySecond, "second")]] }} />);
  expect(screen.queryByRole("button", { name: "Stop command" })).toBeNull();
  expect(screen.getByText(/Save or revert the container/)).toBeTruthy();
});

it("matches saved duplicate legacy IDs by position instead of choosing the first sibling", async () => {
  const first = leaf("First", "duplicate"), second = leaf("Second", "duplicate");
  const runtime = { uuid: MULTI, raw: { Actions: [{ Actions: [first, second] }] }, context: "parent", multi_children: [[live(first, "first"), live(second, "second")]] } as unknown as Slot;
  render(<Sequence raw={runtime.raw} uuid={MULTI} runtime={runtime} device="deck-A" actions={[]} outputs={[]} onChange={vi.fn()} onError={vi.fn()} />);
  await userEvent.click(document.querySelectorAll('.sequence-select')[1]);
  await userEvent.click(screen.getByRole("button", { name: "Stop command" }));
  expect(request).toHaveBeenLastCalledWith("command", { device: "deck-A", context: "second", id: "run-second", command: "command_cancel" });
});
