// @vitest-environment jsdom
import { useState } from "react";
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, it, expect, vi } from "vitest";
import { ImageCropper } from "./ImageCropper";
import { fit, pan, slider, zoom, type CropEdit } from "./crop";
let request: ReturnType<typeof vi.fn>;
beforeEach(() => {
  HTMLDialogElement.prototype.showModal = function () {
    this.setAttribute("open", "");
  };
  HTMLDialogElement.prototype.close = function () {
    this.removeAttribute("open");
  };
  request = vi.fn(async (method: string, payload: any) =>
    method === "image_crop" && payload.mode === "load"
      ? {
          preview: "AA==",
          width: 400,
          height: 100,
          frames: 2,
          frame: payload.frame,
          source_hash: "hash",
        }
      : null,
  );
  Object.defineProperty(window, "deckforge", {
    configurable: true,
    value: { request },
  });
});
afterEach(cleanup);
const initial: CropEdit = {
  device: "A",
  context: "dial",
  expected: null,
  path: "/owned.png",
  frame: 0,
};
it("fits, snaps and preserves the image center under quadratic zoom", () => {
  expect(fit(400, 100, "cover")).toEqual({
    x: -100,
    y: 0,
    width: 400,
    height: 100,
  });
  expect(fit(400, 100, "width")).toEqual({
    x: 0,
    y: 25,
    width: 200,
    height: 50,
  });
  expect(pan({ x: 4, y: 2, width: 200, height: 100 }, true)).toEqual({
    x: 0,
    y: 0,
    width: 200,
    height: 100,
  });
  const r = { x: -100, y: 0, width: 400, height: 100 };
  expect(zoom(r, 4, slider(r), false)).toEqual(r);
  const wide = zoom(r, 40.5, 4000, true);
  expect(Math.max(wide.width, wide.height)).toBeLessThanOrEqual(4000);
});
it("keeps frame controls mounted, supports keyboard pan, and saves captured ownership", async () => {
  const closed = vi.fn(),
    saved = vi.fn();
  function Harness() {
    const [edit, setEdit] = useState(initial);
    return (
      <ImageCropper
        edit={edit}
        onChange={setEdit}
        onClose={closed}
        onSaved={saved}
      />
    );
  }
  render(<Harness />);
  await waitFor(() =>
    expect(
      (screen.getByRole("button", { name: "Set Image" }) as HTMLButtonElement)
        .disabled,
    ).toBe(false),
  );
  const frame = screen.getByLabelText("Frame");
  fireEvent.change(frame, { target: { value: "2" } });
  await waitFor(() =>
    expect(request).toHaveBeenCalledWith(
      "image_crop",
      expect.objectContaining({ mode: "load", frame: 1, source_hash: "hash" }),
    ),
  );
  await waitFor(() => expect((frame as HTMLInputElement).disabled).toBe(false));
  expect(screen.getByLabelText("Frame")).toBe(frame);
  fireEvent.keyDown(screen.getByRole("img"), {
    key: "ArrowLeft",
    shiftKey: true,
  });
  fireEvent.click(screen.getByRole("button", { name: "Set Image" }));
  await waitFor(() => expect(saved).toHaveBeenCalled());
  expect(closed).toHaveBeenCalled();
  expect(request).toHaveBeenCalledWith(
    "image_crop",
    expect.objectContaining({
      mode: "save",
      device: "A",
      context: "dial",
      expected: null,
      frame: 1,
      source_hash: "hash",
      rect: { x: -110, y: 0, width: 400, height: 100 },
    }),
  );
});
it("retains errors for recovery and ignores a file-picker result after closing", async () => {
  const changed = vi.fn();
  let resolve!: (path: string) => void;
  request.mockImplementation((method: string) =>
    method === "pick_file"
      ? new Promise<string>((r) => {
          resolve = r;
        })
      : Promise.reject(new Error("Bad image")),
  );
  const view = render(
    <ImageCropper
      edit={initial}
      onChange={changed}
      onClose={() => {}}
      onSaved={() => {}}
    />,
  );
  await screen.findByRole("alert");
  expect(screen.getByRole("alert").textContent).toContain("Bad image");
  fireEvent.click(screen.getByRole("button", { name: "Choose image…" }));
  view.unmount();
  await act(async () => resolve("/late.png"));
  expect(changed).not.toHaveBeenCalled();
});
