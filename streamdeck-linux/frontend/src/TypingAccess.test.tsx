// @vitest-environment jsdom
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import { TypingAccess } from "./TypingAccess";
import { containsDesktopTyping } from "./App";
import type { Slot } from "./model";
const request = vi.fn().mockResolvedValue({});
vi.mock("./bridge", () => ({ request: (...args: unknown[]) => request(...args) }));
afterEach(() => { cleanup(); request.mockClear(); });
it("keeps editor Test press from sending typing back into itself, including nested text", () => {
  const text = { uuid: "com.elgato.streamdeck.system.text", settings: { isTypingMode: true } } as unknown as Slot;
  expect(containsDesktopTyping(text)).toBe(true);
  expect(containsDesktopTyping({ ...text, settings: null as any })).toBe(false);
  expect(containsDesktopTyping({ ...text, settings: { isTypingMode: false } })).toBe(false);
  expect(containsDesktopTyping({ uuid: "container", settings: {}, multi_children: [[text]] } as Slot)).toBe(true);
});
it("requests keyboard permission only from Enable, without editing or triggering text", async () => {
  render(<TypingAccess status={{ phase: "disabled", message: "Enable typing" }} onError={vi.fn()} />);
  expect(request).not.toHaveBeenCalled();
  await userEvent.click(screen.getByRole("button", { name: "Enable desktop typing" }));
  expect(request).toHaveBeenCalledWith("command", { command: "text_typing_setup" });
});
it("cancels pending permission and stops active typing using the global command", async () => {
  const view = render(<TypingAccess status={{ phase: "requesting", message: "Allow keyboard" }} onError={vi.fn()} />);
  await userEvent.click(screen.getByRole("button", { name: "Cancel permission request" }));
  expect(request).toHaveBeenLastCalledWith("command", { command: "text_typing_disable" });
  view.rerender(<TypingAccess status={{ phase: "typing", message: "Typing" }} onError={vi.fn()} />);
  await userEvent.click(screen.getByRole("button", { name: "Stop typing" }));
  expect(request).toHaveBeenCalledTimes(2);
  view.rerender(<TypingAccess status={{ phase: "stopping", message: "Stopping" }} onError={vi.fn()} />);
  expect((screen.getByRole("button", { name: "Stopping typing…" }) as HTMLButtonElement).disabled).toBe(true);
});
