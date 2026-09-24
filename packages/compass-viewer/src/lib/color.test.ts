import { describe, expect, it } from "vitest";
import {
  INK_ON_DARK,
  INK_ON_LIGHT,
  blendColor,
  contrastRatio,
  isDarkColor,
  parseColor,
  readableInk,
  relativeLuminance
} from "./color";

describe("parseColor", () => {
  it("reads short and long hex plus rgb()", () => {
    expect(parseColor("#2788C1")).toEqual([0x27, 0x88, 0xC1]);
    expect(parseColor("#abc")).toEqual([0xAA, 0xBB, 0xCC]);
    expect(parseColor("rgb(1, 2, 3)")).toEqual([1, 2, 3]);
    expect(parseColor("color-mix(in srgb, red, blue)")).toBeUndefined();
  });
});

describe("blendColor", () => {
  it("mixes toward the foreground and stays parseable", () => {
    expect(blendColor("#000000", "#FFFFFF", 0.5)).toBe("rgb(128, 128, 128)");
    expect(blendColor("#FBFCFD", "#2788C1", 0.82)).toBe("rgb(77, 157, 204)");
    expect(blendColor("not-a-color", "#FFFFFF", 0.5)).toBe("#FFFFFF");
  });
});

describe("readableInk", () => {
  it("picks light ink on dark surfaces and dark ink on light ones", () => {
    expect(readableInk("#101820", "#FBFCFD")).toBe(INK_ON_DARK);
    expect(readableInk("#F6F7F9", "#0C1015")).toBe(INK_ON_LIGHT);
  });

  it("maximizes measured contrast on translucent community tiles", () => {
    // The Compass palette sits in a mid-tone band where ink beats white, which
    // is why the area map and tier bars label with dark text.
    for (const fill of ["#2788C1", "#AE8C46", "#C56775", "#8C9047", "#8176BA"]) {
      expect(readableInk(fill, "#FBFCFD", 0.82)).toBe(INK_ON_LIGHT);
    }
    // A genuinely dark surface still flips to light ink.
    expect(readableInk("#123048", "#0C1015", 0.82)).toBe(INK_ON_DARK);
  });
});

describe("contrastRatio", () => {
  it("matches the WCAG reference values", () => {
    expect(contrastRatio("#FFFFFF", "#000000")).toBeCloseTo(21, 1);
    expect(contrastRatio("#FFFFFF", "#FFFFFF")).toBeCloseTo(1, 5);
    expect(contrastRatio("#0E1319", "#2788C1")).toBeGreaterThan(4.4);
  });
});

describe("relativeLuminance and isDarkColor", () => {
  it("orders light above dark and agrees with the threshold helper", () => {
    expect(relativeLuminance("#FFFFFF")).toBeCloseTo(1, 5);
    expect(relativeLuminance("#000000")).toBeCloseTo(0, 5);
    expect(relativeLuminance("#F6F7F9")).toBeGreaterThan(relativeLuminance("#0C1015"));
    expect(isDarkColor("#0C1015")).toBe(true);
    expect(isDarkColor("#FBFCFD")).toBe(false);
    expect(isDarkColor("not-a-color")).toBe(true);
  });
});
