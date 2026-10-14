// @vitest-environment jsdom
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import type { Snapshot } from "./model";
import { useIconCatalog } from "./useIconCatalog";
afterEach(() => { cleanup(); delete window.deckforge; });
function View({ busy = false }: { busy?: boolean }) {
  const { catalog, refresh, watchError } = useIconCatalog<{ value: string }>(busy);
  return <><span data-testid="catalog">{catalog?.value || "loading"}</span><span data-testid="error">{watchError}</span>
    <button onClick={() => void refresh().catch(() => {})}>Reload</button></>;
}
it("coalesces notifications during a read and ignores older manual replies", async () => {
  const resolves: Array<(v: { value: string }) => void> = [];
  let listener!: (s: Snapshot) => void; const stop = vi.fn();
  const request = vi.fn(() => new Promise<{ value: string }>((r) => resolves.push(r)));
  window.deckforge = { request: request as any, onSnapshot: (cb) => { listener = cb; cb({ icon_library: { revision: 0, error: null } } as Snapshot); return stop; } };
  const view = render(<View />); await waitFor(() => expect(resolves).toHaveLength(1));
  await act(async () => listener({ icon_library: { revision: 1, error: null } } as Snapshot));
  await act(async () => { await new Promise((r) => setTimeout(r, 140)); });
  expect(resolves).toHaveLength(1);
  await act(async () => resolves[0]({ value: "old" }));
  await waitFor(() => expect(resolves).toHaveLength(2));
  await act(async () => resolves[1]({ value: "new" })); expect(screen.getByTestId("catalog").textContent).toBe("new");
  const user = userEvent.setup(); await user.click(screen.getByRole("button", { name: "Reload" })); await user.click(screen.getByRole("button", { name: "Reload" }));
  await act(async () => resolves[3]({ value: "newest" })); await act(async () => resolves[2]({ value: "outdated" }));
  expect(screen.getByTestId("catalog").textContent).toBe("newest");
  view.unmount(); expect(stop).toHaveBeenCalledOnce();
});
it("defers changes while busy and retries transient errors without losing its catalog", async () => {
  let listener!: (s: Snapshot) => void, fail = false;
  const request = vi.fn(async () => { if (fail) { fail = false; throw Error("Icon library is busy"); } return { value: "catalog" }; });
  window.deckforge = { request: request as any, onSnapshot: (cb) => { listener = cb; cb({ icon_library: { revision: 0, error: null } } as Snapshot); return () => {}; } };
  const view = render(<View />); await screen.findByText("catalog"); const reads = request.mock.calls.length;
  view.rerender(<View busy />);
  await act(async () => listener({ icon_library: { revision: 1, error: null } } as Snapshot));
  await act(async () => { await new Promise((r) => setTimeout(r, 180)); }); expect(request).toHaveBeenCalledTimes(reads);
  fail = true; view.rerender(<View />);
  await waitFor(() => expect(screen.getByTestId("error").textContent).toContain("busy"));
  expect(screen.getByTestId("catalog").textContent).toBe("catalog");
  await waitFor(() => expect(screen.getByTestId("error").textContent).toBe(""));
  expect(request).toHaveBeenCalledTimes(reads + 2);
});
