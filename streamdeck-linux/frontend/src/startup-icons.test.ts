// @vitest-environment jsdom
import { afterEach, expect, it } from "vitest";
import { request, type DesktopBridge } from "./bridge";

afterEach(() => {
  delete window.deckforge;
});

it("loads a large library without filling the native queue or blocking draft saves", async () => {
  let active = 0,
    peak = 0;
  const jobs: Array<() => void> = [];
  window.deckforge = {
    onSnapshot: () => () => {},
    request: (method: string) => {
      if (active >= 64)
        return Promise.reject(new Error("Editor worker is busy; try again"));
      active++;
      peak = Math.max(peak, active);
      return new Promise<any>((resolve) =>
        jobs.push(() => {
          active--;
          resolve(method);
        }),
      );
    },
  } as DesktopBridge;
  const icons = Array.from({ length: 160 }, (_, uuid) =>
    request("action_icon", { uuid: String(uuid) }),
  );
  const save = request("ui_state_set", { state: { draft: "retained" } });
  const results = Promise.allSettled([...icons, save]);
  for (let n = 0; n < 200; n++) {
    jobs.splice(0).forEach((finish) => finish());
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
  expect(
    (await results).filter((result) => result.status === "rejected"),
  ).toEqual([]);
  expect(peak).toBeLessThanOrEqual(5);
});

it("shares in-flight icons, keeps generations separate and drains after failure", async () => {
  const jobs: Array<{
    resolve: (value: unknown) => void;
    reject: (error: Error) => void;
  }> = [];
  window.deckforge = {
    onSnapshot: () => () => {},
    request: () =>
      new Promise<any>((resolve, reject) => jobs.push({ resolve, reject })),
  } as DesktopBridge;
  const first = request("action_icon", { uuid: "same", generation: 1 });
  const duplicate = request("action_icon", { uuid: "same", generation: 1 });
  const updated = request("action_icon", { uuid: "same", generation: 2 });
  const more = Array.from({ length: 8 }, (_, uuid) =>
    request("action_icon", { uuid }),
  );
  const settled = Promise.allSettled([first, duplicate, updated, ...more]);
  expect(first).toBe(duplicate);
  expect(jobs).toHaveLength(4);
  jobs.shift()!.reject(new Error("Missing icon"));
  for (let n = 0; n < 10; n++) {
    jobs.splice(0).forEach((job) => job.resolve("image"));
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
  expect(
    (await settled).filter((result) => result.status === "rejected"),
  ).toHaveLength(2);
  const retry = request("action_icon", { uuid: "same", generation: 1 });
  jobs.shift()!.resolve("retried");
  await expect(retry).resolves.toBe("retried");
});
