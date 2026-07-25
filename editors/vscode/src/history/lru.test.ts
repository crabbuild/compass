import { describe, expect, it } from "vitest";
import { LruCache } from "./lru";

describe("LruCache", () => {
  it("keeps only the three newest decoded revisions", () => {
    const cache = new LruCache<string, object>(3);
    ["a", "b", "c", "d"].forEach((key) => cache.set(key, {}));
    expect(cache.keys()).toEqual(["d", "c", "b"]);
  });
});
