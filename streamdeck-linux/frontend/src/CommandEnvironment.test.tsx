// @vitest-environment jsdom
import { useState } from "react";
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import { CommandEnvironment } from "./CommandEnvironment";
import type { RecordValue } from "./model";
afterEach(cleanup);
it("edits literal values and retains them plus unknown fields while unset is toggled", async () => {
  let current: RecordValue[] = [];
  function Harness() {
    const [rows, setRows] = useState<RecordValue[]>([{ name: "DECKFORGE_FIXTURE", value: "original", Future: 7 }]); current = rows;
    return <CommandEnvironment value={rows} onChange={setRows} />;
  }
  render(<Harness />); const user = userEvent.setup();
  await user.click(screen.getByText("Environment variables (1)"));
  await user.clear(screen.getByLabelText("Variable 1 value"));
  await user.type(screen.getByLabelText("Variable 1 value"), "$PATH 'literal'{Enter}🌍");
  await user.click(screen.getByLabelText("Unset variable 1"));
  expect((screen.getByLabelText("Variable 1 value") as HTMLTextAreaElement).disabled).toBe(true);
  expect(current[0]).toEqual({ name: "DECKFORGE_FIXTURE", value: "$PATH 'literal'\n🌍", Future: 7, unset: true });
  await user.click(screen.getByLabelText("Unset variable 1"));
  expect((screen.getByLabelText("Variable 1 value") as HTMLTextAreaElement).value).toBe("$PATH 'literal'\n🌍");
  await user.click(screen.getByRole("button", { name: "Add environment variable" }));
  await user.type(screen.getByLabelText("Variable 2 name"), "DECKFORGE_SECOND");
  await user.click(screen.getByRole("button", { name: "Remove variable 1" }));
  expect(current).toEqual([{ name: "DECKFORGE_SECOND", value: "", unset: false }]);
});
it("preserves unsupported imported data until explicitly replaced", async () => {
  const onChange = vi.fn();
  render(<CommandEnvironment value={{ FutureFormat: 1 }} onChange={onChange} />);
  expect(onChange).not.toHaveBeenCalled();
  await userEvent.click(screen.getByRole("button", { name: "Replace with empty environment list" }));
  expect(onChange).toHaveBeenCalledWith([]);
});
