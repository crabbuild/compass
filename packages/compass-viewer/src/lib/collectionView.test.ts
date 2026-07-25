import { describe, expect, it } from "vitest";
import { clampPage, paginate } from "./collectionView";

describe("collection view", () => {
  it("returns the requested page and visible range", () => {
    expect(paginate([1, 2, 3, 4, 5], 2, 2)).toEqual({
      items: [3, 4],
      page: 2,
      pageCount: 3,
      pageSize: 2,
      total: 5,
      start: 3,
      end: 4
    });
  });

  it("clamps empty and out-of-range pages", () => {
    expect(clampPage(0, 0)).toBe(1);
    expect(paginate(["a"], 9, 25).page).toBe(1);
  });
});
