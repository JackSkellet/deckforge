// @vitest-environment jsdom
import { useState } from "react";
import { afterEach, expect, it, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ApplicationPicker, ApplicationIcon } from "./ApplicationPicker";
import { request } from "./bridge";
afterEach(() => { cleanup(); delete window.deckforge; vi.unstubAllGlobals(); });
const applications = Array.from({ length: 45 }, (_, i) => ({
  id: `org.app.${i}`, name: `Application ${i}`, desktop: `/apps/${i}.desktop`, executable: `/bin/app${i}`,
}));
it("supports radio keyboard selection, filtering, paging and missing-icon fallback", async () => {
  // jsdom lacks CSS.escape, which user-event uses for React's radio group ID.
  vi.stubGlobal("CSS", { escape: (s: string) => s.replace(/[^a-zA-Z0-9_-]/g, "\\$&") });
  window.deckforge = { request: vi.fn(async () => null) as any, onSnapshot: () => () => {} };
  let selected = "";
  function Harness() {
    const [value, setValue] = useState("");
    const [search, setSearch] = useState("");
    return <><input aria-label="Search" value={search} onChange={(e) => setSearch(e.target.value)} />
      <ApplicationPicker applications={applications} search={search} value={value}
        onSelect={(app) => { selected = app.desktop; setValue(app.desktop); }} /></>;
  }
  render(<Harness />); const user = userEvent.setup();
  expect(screen.getAllByRole("radio")).toHaveLength(40);
  await user.click(screen.getByRole("radio", { name: "Application 0" }));
  await user.keyboard("{ArrowDown}");
  expect(selected).toBe("/apps/1.desktop");
  expect((screen.getByRole("radio", { name: "Application 1" }) as HTMLInputElement).checked).toBe(true);
  await user.click(screen.getByRole("button", { name: /Show more/ }));
  expect(screen.getAllByRole("radio")).toHaveLength(45);
  await user.type(screen.getByLabelText("Search"), "org.app.44");
  expect(screen.getAllByRole("radio")).toHaveLength(1);
  await user.click(screen.getByRole("radio", { name: "Application 44" }));
  expect(selected).toBe("/apps/44.desktop");
  expect(document.querySelector(".application-icon")?.textContent).toBe("A");
  await user.clear(screen.getByLabelText("Search"));
  await user.type(screen.getByLabelText("Search"), "does not exist");
  expect(screen.getByRole("status").textContent).toContain("No matching");
});
it("does not display a late icon belonging to the previously selected application", async () => {
  let resolve!: (v: string) => void;
  window.deckforge = { request: vi.fn((_method, p: any) => p.desktop === "old"
    ? new Promise<string>((r) => { resolve = r; }) : Promise.resolve(null)) as any, onSnapshot: () => () => {} };
  const view = render(<ApplicationIcon desktop="old" name="Old" />);
  view.rerender(<ApplicationIcon desktop="new" name="New" />);
  resolve("data:image/png;base64,b2xk");
  await waitFor(() => expect(document.querySelector(".application-icon")?.textContent).toBe("N"));
  expect(document.querySelector("img")).toBeNull();
});
it("shares the four-request icon limit across application and action images without conflating them", async () => {
  const releases: Array<() => void> = []; let active = 0, peak = 0;
  const call = vi.fn((method: string) => new Promise((resolve) => {
    active++; peak = Math.max(peak, active);
    releases.push(() => { active--; resolve(method); });
  }));
  window.deckforge = { request: call as any, onSnapshot: () => () => {} };
  const pending = Array.from({ length: 10 }, (_, i) => request(i % 2 ? "application_icon" : "action_icon", { key: Math.floor(i / 2) }));
  expect(call).toHaveBeenCalledTimes(4);
  const duplicate = request("application_icon", { key: 0 });
  for (let i = 0; i < 10; i++) { await waitFor(() => expect(releases.length).toBeGreaterThan(i)); releases[i](); }
  expect(await Promise.all(pending)).toEqual(Array.from({ length: 10 }, (_, i) => i % 2 ? "application_icon" : "action_icon"));
  expect(await duplicate).toBe("application_icon"); expect(peak).toBe(4); expect(call).toHaveBeenCalledTimes(10);
});
