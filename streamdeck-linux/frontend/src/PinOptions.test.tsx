// @vitest-environment jsdom
import { render, screen, fireEvent, cleanup } from "@testing-library/react";
import { expect, it, vi, afterEach } from "vitest";
import { PinOptions } from "./PinOptions";
import type { Slot } from "./model";
const slot = {
  context: "key",
  pinned: true,
  pin_excluded_pages: ["first"],
} as Slot;
const pages = [
  { id: "first", name: "Home" },
  { id: "middle", name: "Voyis" },
  { id: "last", name: "Tools" },
];
it("shows per-page exclusions for pinned controls and sends IDs plus the previous selection", () => {
  const run = vi.fn();
  render(<PinOptions slot={slot} pages={pages} run={run} />);
  expect(screen.getByText("Exclude pages (1)")).toBeTruthy();
  expect(
    (screen.getByLabelText("Page 1: Home") as HTMLInputElement).checked,
  ).toBe(true);
  fireEvent.click(screen.getByLabelText("Page 3: Tools"));
  expect(run).toHaveBeenCalledWith({
    command: "slot_pin_exclusions",
    context: "key",
    expected: ["first"],
    excluded_pages: ["first", "last"],
  });
});
it("page reordering retains the selected IDs", () => {
  render(
    <PinOptions
      slot={slot}
      pages={[pages[2], pages[1], pages[0]]}
      run={vi.fn()}
    />,
  );
  expect(
    (screen.getByLabelText("Page 3: Home") as HTMLInputElement).checked,
  ).toBe(true);
  expect(
    (screen.getByLabelText("Page 1: Tools") as HTMLInputElement).checked,
  ).toBe(false);
});
it("unpinned controls do not offer page exclusions", () => {
  render(
    <PinOptions
      slot={{ ...slot, pinned: false }}
      pages={pages}
      run={vi.fn()}
    />,
  );
  expect(screen.queryByText(/Exclude pages/)).toBeNull();
});

afterEach(cleanup);
