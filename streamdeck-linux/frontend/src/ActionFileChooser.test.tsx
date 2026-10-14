// @vitest-environment jsdom
import { useState } from "react";
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ActionFileChooser } from "./ActionFileChooser";
import type { ActionFileChoice, PendingActionFile } from "./model";
beforeEach(() => {
  HTMLDialogElement.prototype.showModal = function () { this.setAttribute("open", ""); };
  HTMLDialogElement.prototype.close = function () { this.removeAttribute("open"); };
});
afterEach(cleanup);
const pending: PendingActionFile = { id: "choice", device: "A", destination: { controller: "Keypad", index: 1, expected_target: null, expected_profile: "p", expected_page: "page" }, info: { path: "/shared.streamDeckAction", source_hash: "hash", roots: [
  { index: 0, name: "First action", uuid: "unknown.key", controller: "Keypad", page: "root", column: 0, row: 0 },
  { index: 1, name: "Second action", uuid: "unknown.key", controller: "Keypad", page: "root", column: 1, row: 0 },
  { index: 2, name: "Dial action", uuid: "unknown.dial", controller: "Encoder", page: "root", column: 0, row: 0 },
] } };
it("captures the destination, filters choices and never commits a hidden or incompatible root", async () => {
  let current!: ActionFileChoice; const choose = vi.fn(), cancel = vi.fn(), submit = vi.fn((e) => e.preventDefault());
  function Harness() { const [choice, setChoice] = useState({ id: pending.id, root: 0, query: "" }); current = choice;
    return <form onSubmit={submit}><ActionFileChooser pending={pending} choice={choice} deviceName="Deck A" actions={[]} busy={false} error="" onChange={setChoice} onChoose={choose} onClose={cancel} /></form>;
  }
  render(<Harness />); const user = userEvent.setup();
  expect(screen.getByText(/Key 2 on Deck A/)).toBeTruthy();
  expect((screen.getByRole("option", { name: /Dial action/ }) as HTMLOptionElement).disabled).toBe(true);
  await user.type(screen.getByLabelText("Find action"), "Second{Enter}"); expect(current.root).toBe(1); expect(submit).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "Import action" })); expect(choose).toHaveBeenCalledOnce();
  await user.clear(screen.getByLabelText("Find action")); await user.type(screen.getByLabelText("Find action"), "no matches");
  expect((screen.getByRole("button", { name: "Import action" }) as HTMLButtonElement).disabled).toBe(true);
  await user.click(screen.getByRole("button", { name: "Cancel" })); expect(cancel).toHaveBeenCalledOnce();
});
it("clipboard selection can retain a dial root independently of a canvas destination", () => {
  render(<ActionFileChooser pending={{ ...pending, destination: null }} choice={{ id: pending.id, root: 2, query: "" }} deviceName="Deck A" actions={[]} busy={false} error="" onChange={vi.fn()} onChoose={vi.fn()} onClose={vi.fn()} />);
  expect((screen.getByRole("button", { name: "Copy to clipboard" }) as HTMLButtonElement).disabled).toBe(false);
});
