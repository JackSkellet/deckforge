import { afterEach, expect, it, vi } from "vitest";
import { newId } from "./ids";
const nativeCrypto = globalThis.crypto;
afterEach(() => vi.unstubAllGlobals());
it("uses CSPRNG bytes and sets UUID version4/variant bits when randomUUID is unavailable", () => {
  const getRandomValues = vi.fn((bytes: Uint8Array) => bytes.fill(255));
  vi.stubGlobal("crypto", { getRandomValues });
  expect(newId()).toBe("ffffffff-ffff-4fff-bfff-ffffffffffff");
  expect(getRandomValues).toHaveBeenCalledExactlyOnceWith(
    expect.any(Uint8Array),
  );
});
it("produces distinct valid IDs through the actual secure random fallback", () => {
  vi.stubGlobal("crypto", {
    getRandomValues: nativeCrypto.getRandomValues.bind(nativeCrypto),
  });
  const values = Array.from({ length: 1000 }, newId);
  expect(new Set(values).size).toBe(1000);
  expect(
    values.every((value) =>
      /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(
        value,
      ),
    ),
  ).toBe(true);
});
it("keeps the native UUID implementation when it is available", () => {
  const value = "3b014ae2-72cd-4ea5-853f-45a682f4675b";
  const randomUUID = vi.fn(() => value);
  vi.stubGlobal("crypto", { randomUUID });
  expect(newId()).toBe(value);
  expect(randomUUID).toHaveBeenCalledOnce();
});
