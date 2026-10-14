// @vitest-environment jsdom
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { IconLibrary } from "./IconLibrary";
const pack = { id: "test.icons", name: "Test pack", author: "Author", version: "1", revision: "revision-1", pinned: false, can_hide: true, hidden: false, warnings: [], icons: [
  { path: "one.svg", name: "First icon", tags: ["accessibility"] }, { path: "two.png", name: "Second icon", tags: ["other"] },
] };
beforeEach(() => {
  HTMLDialogElement.prototype.showModal = function () { this.setAttribute("open", ""); };
  HTMLDialogElement.prototype.close = function () { this.removeAttribute("open"); };
});
afterEach(() => { cleanup(); delete window.deckforge; });
it("searches tags and resolves the selected pack asset without saving an action", async () => {
  const call = vi.fn(async (method: string, payload: any) => {
    if (method === "icon_packs" && payload.operation === "list") return { packs: [pack], errors: [] };
    if (payload.operation === "thumbnail") return "data:image/png;base64,cG5n";
    if (payload.operation === "asset") return "/managed/icons/one.svg";
  });
  window.deckforge = { request: call as any, onSnapshot: () => () => {} };
  const choose = vi.fn(), close = vi.fn(); render(<IconLibrary onChoose={choose} onClose={close} />);
  const user = userEvent.setup(); await screen.findByRole("button", { name: "First icon" });
  await user.type(screen.getByLabelText("Search icons"), "accessibility");
  expect(screen.queryByRole("button", { name: "Second icon" })).toBeNull();
  await user.click(screen.getByRole("button", { name: "First icon" }));
  expect(call).toHaveBeenCalledWith("icon_packs", { operation: "asset", id: "test.icons", revision: "revision-1", path: "one.svg" });
  expect(choose).toHaveBeenCalledWith("/managed/icons/one.svg"); expect(close).toHaveBeenCalledOnce();
  expect(call.mock.calls.some(([method]) => method === "command")).toBe(false);
});
it("cancelled file selection does not install; a selected pack refreshes its catalog", async () => {
  let path: string | null = null, installed = false;
  const call = vi.fn(async (method: string, payload: any) => {
    if (method === "pick_file") return path;
    if (payload.operation === "list") return { packs: installed ? [pack] : [], errors: [] };
    if (payload.operation === "install") { installed = true; return pack; }
    if (payload.operation === "thumbnail") return "data:image/png;base64,cG5n";
  });
  window.deckforge = { request: call as any, onSnapshot: () => () => {} };
  const submit = vi.fn((event) => event.preventDefault());
  render(<form onSubmit={submit}><IconLibrary onChoose={vi.fn()} onClose={vi.fn()} /></form>); const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: "Import pack…" }));
  expect(call.mock.calls.some(([,p]) => p.operation === "install")).toBe(false);
  path = "/owned.streamDeckIconPack";
  await user.click(screen.getByRole("button", { name: "Import pack…" }));
  await waitFor(() => expect(screen.getByRole("status").textContent).toBe("Imported Test pack"));
  expect((screen.getByLabelText("Icon pack") as HTMLSelectElement).value).toBe(pack.id);
  await user.type(screen.getByLabelText("Search icons"), "accessibility{Enter}");
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  expect(submit).not.toHaveBeenCalled();
});


it("hidden packs stay searchable and pinned packs cannot be removed", async () => {
  let current = { ...pack, pinned: true, hidden: true };
  const call = vi.fn(async (_method: string, p: any) => {
    if (p.operation === "list") return { packs: [current], errors: [], archives: [] };
    if (p.operation === "thumbnail") return "data:image/png;base64,cG5n";
    if (p.operation === "hide") { current = { ...current, hidden: p.hidden }; return null; }
  });
  window.deckforge = { request: call as any, onSnapshot: () => () => {} };
  render(<IconLibrary onChoose={vi.fn()} onClose={vi.fn()} />); const user = userEvent.setup();
  await screen.findByText(/Packs are hidden/);
  expect(screen.queryByRole("button", { name: "First icon" })).toBeNull();
  await user.type(screen.getByLabelText("Search icons"), "accessibility");
  expect(screen.getByRole("button", { name: "First icon" })).toBeTruthy();
  await user.selectOptions(screen.getByLabelText("Icon pack"), pack.id);
  expect((screen.getByRole("button", { name: "Remove pack…" }) as HTMLButtonElement).disabled).toBe(true);
  await user.click(screen.getByRole("button", { name: "Show pack" }));
  expect(call).toHaveBeenCalledWith("icon_packs", { operation: "hide", id: pack.id, revision: pack.revision, expected_hidden: true, hidden: false });
  await user.clear(screen.getByLabelText("Search icons"));
  expect(screen.getByRole("button", { name: "First icon" })).toBeTruthy();
});
it("reviews replacement and removal before applying and exposes archived restoration", async () => {
  let current: typeof pack | null = { ...pack }; let archives: any[] = [];
  const replacement = { id: pack.id, source: "/update.streamDeckIconPack", expected_revision: pack.revision, candidate_digest: "digest", current: { name: pack.name, version: "1", icons: 2 }, replacement: { name: pack.name, version: "2", icons: 2 } };
  const restore = { ...replacement, expected_revision: null, current: null };
  const call = vi.fn(async (method: string, p: any) => {
    if (method === "pick_file") return replacement.source;
    if (p.operation === "list") return { packs: current ? [current] : [], errors: [], archives };
    if (p.operation === "thumbnail") return "data:image/png;base64,cG5n";
    if (p.operation === "review") return replacement;
    if (p.operation === "review_restore") return restore;
    if (p.operation === "apply") { current = { ...pack, version: "2", revision: "revision-2" }; return current; }
    if (p.operation === "remove") { current = null; archives = [{ token: "removed", id: pack.id, name: pack.name, version: "2", icons: 2, kind: "remove", created: 0 }]; return null; }
  });
  window.deckforge = { request: call as any, onSnapshot: () => () => {} };
  const choose = vi.fn(); render(<IconLibrary onChoose={choose} onClose={vi.fn()} />); const user = userEvent.setup();
  await screen.findByRole("button", { name: "First icon" }); await user.selectOptions(screen.getByLabelText("Icon pack"), pack.id);
  await user.click(screen.getByRole("button", { name: "Update pack…" })); await screen.findByRole("button", { name: "Apply replacement" });
  expect(call.mock.calls.some(([,p]) => p.operation === "apply")).toBe(false);
  await user.click(screen.getByRole("button", { name: "Cancel change" }));
  await user.click(screen.getByRole("button", { name: "Update pack…" })); await user.click(await screen.findByRole("button", { name: "Apply replacement" }));
  expect(call).toHaveBeenCalledWith("icon_packs", { operation: "apply", review: replacement });
  await user.click(await screen.findByRole("button", { name: "Remove pack…" }));
  expect(call.mock.calls.some(([,p]) => p.operation === "remove")).toBe(false);
  await user.click(screen.getByRole("button", { name: "Remove pack" }));
  expect(call).toHaveBeenCalledWith("icon_packs", { operation: "remove", id: pack.id, revision: "revision-2" });
  await user.click(await screen.findByText("Archived versions (1)"));
  await user.click(screen.getByRole("button", { name: "Restore Test pack 2" }));
  await user.click(await screen.findByRole("button", { name: "Apply replacement" }));
  expect(call).toHaveBeenCalledWith("icon_packs", { operation: "apply", review: restore });
  expect(choose).not.toHaveBeenCalled();
});


it("refreshes external revisions without changing search or approving a stale review", async () => {
  let current = { ...pack }, listener: ((s: any) => void) | undefined;
  const review = { id: pack.id, source: "/replacement", expected_revision: pack.revision, candidate_digest: "digest", current: { name: pack.name, version: "1", icons: 2 }, replacement: { name: pack.name, version: "2", icons: 2 } };
  const call = vi.fn(async (method: string, p: any) => {
    if (method === "pick_file") return review.source;
    if (p.operation === "list") return { packs: [current], errors: [], archives: [] };
    if (p.operation === "thumbnail") return "data:image/png;base64,cG5n";
    if (p.operation === "review") return review;
  });
  window.deckforge = { request: call as any, onSnapshot: (cb) => { listener = cb; cb({ icon_library: { revision: 0, error: null } } as any); return () => { listener = undefined; }; } };
  render(<IconLibrary onChoose={vi.fn()} onClose={vi.fn()} />); const user = userEvent.setup();
  await screen.findByRole("button", { name: "First icon" }); await user.type(screen.getByLabelText("Search icons"), "accessibility");
  await user.selectOptions(screen.getByLabelText("Icon pack"), pack.id); await user.click(screen.getByRole("button", { name: "Update pack…" }));
  await screen.findByRole("button", { name: "Apply replacement" });
  current = { ...pack, revision: "externally-changed", version: "1.1" };
  await act(async () => listener?.({ icon_library: { revision: 1, error: null } }));
  await screen.findByText(/This pack changed while you reviewed it/);
  expect((screen.getByRole("button", { name: "Apply replacement" }) as HTMLButtonElement).disabled).toBe(true);
  expect(screen.getByText(/1 \(2 icons\) → 2/)).toBeTruthy();
  await user.click(screen.getByRole("button", { name: "Cancel change" }));
  expect((screen.getByLabelText("Search icons") as HTMLInputElement).value).toBe("accessibility");
  expect((screen.getByLabelText("Icon pack") as HTMLSelectElement).value).toBe(pack.id);
  expect(call.mock.calls.some(([,p]) => p.operation === "apply")).toBe(false);
});
