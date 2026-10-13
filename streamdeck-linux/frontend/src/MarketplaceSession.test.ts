// @vitest-environment node
import source from "../../src/marketplace_session.js?raw";
import { expect, it, vi } from "vitest";
async function probe(
  operation: string,
  responses: { status: number; data?: unknown }[],
  origin = "https://marketplace.elgato.com",
) {
  const messages: any[] = [];
  const fetch = vi.fn(async (_url: string, _options: unknown) => {
    const next = responses.shift()!;
    return {
      status: next.status,
      ok: next.status >= 200 && next.status < 300,
      json: async () => next.data,
    };
  });
  const window: any = {
    ipc: {
      postMessage: (value: string) => {
        const envelope = JSON.parse(value);
        expect(envelope.nonce).toBe("test-capability");
        messages.push(envelope.reply);
      },
    },
  };
  window.top = window;
  const context = {
    window,
    location: { origin, pathname: "/account/products" },
    fetch,
    AbortController,
    setTimeout,
    clearTimeout,
  };
  const script = source
    .replace("__NONCE__", JSON.stringify("test-capability"))
    .replace("__OPERATION__", JSON.stringify(operation))
    .replace(
      "__VARIANT__",
      JSON.stringify("30bbf05a-ee28-45cd-b3a6-b072d3d56351"),
    );
  new Function("window", "location", "fetch", script)(
    context.window,
    context.location,
    context.fetch,
  );
  new Function("window", "location", "fetch", script)(
    context.window,
    context.location,
    context.fetch,
  ); // repeated native probes must not duplicate requests
  for (let n = 0; n < 30; n++) await Promise.resolve();
  return { messages, fetch };
}
it("uses the persistent same-origin session and resolves the captured variant", async () => {
  const { messages, fetch } = await probe("install", [
    {
      status: 200,
      data: { user: { name: "Example", accessToken: "MUST_NOT_LEAVE_PAGE" } },
    },
    {
      status: 200,
      data: {
        direct_link: "https://mp-cdn.elgato.com/plugin?signature=PRIVATE",
      },
    },
  ]);
  expect(fetch).toHaveBeenCalledTimes(2);
  expect(fetch.mock.calls[1]?.[0]).toBe(
    "/api/proxy/purchase-link/30bbf05a-ee28-45cd-b3a6-b072d3d56351",
  );
  expect((fetch.mock.calls[0] as any)[1].credentials).toBe("include");
  expect(messages).toEqual([
    {
      kind: "download",
      name: "Example",
      url: "https://mp-cdn.elgato.com/plugin?signature=PRIVATE",
    },
  ]);
  expect(JSON.stringify(messages)).not.toContain("MUST_NOT_LEAVE_PAGE");
});
it("requests sign-in for a missing or expired session and never purchases products", async () => {
  expect(
    (await probe("install", [{ status: 200, data: { user: null } }])).messages,
  ).toEqual([{ kind: "signed_out" }]);
  expect(
    (
      await probe("install", [
        { status: 200, data: { user: { name: "Example" } } },
        { status: 401 },
      ])
    ).messages,
  ).toEqual([{ kind: "signed_out" }]);
  const denied = await probe("install", [
    { status: 200, data: { user: { name: "Example" } } },
    { status: 403 },
  ]);
  expect(denied.messages[0].error).toContain("Get it in Marketplace");
  expect(denied.fetch).toHaveBeenCalledTimes(2);
});
it("does not expose a bridge on identity-provider pages and separates checks from downloads", async () => {
  const outside = await probe("install", [], "https://account.elgato.com");
  expect(outside.fetch).not.toHaveBeenCalled();
  const check = await probe("check", [
    { status: 200, data: { user: { name: "Example" } } },
  ]);
  expect(check.fetch).toHaveBeenCalledTimes(1);
  expect(check.messages).toEqual([{ kind: "authenticated", name: "Example" }]);
});
