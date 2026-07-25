import { describe, expect, it } from "vitest";
import {
  DEFAULT_INSPECTOR_LAYOUT,
  normalizeInspectorLayout,
  resizeInspectorByKeyboard,
  resizeInspectorFromPointer
} from "./inspectorLayout";

describe("inspector layout", () => {
  it("normalizes absent and stored values into the supported range", () => {
    expect(normalizeInspectorLayout(undefined)).toEqual(DEFAULT_INSPECTOR_LAYOUT);
    expect(normalizeInspectorLayout({ width: 40, collapsed: false })).toEqual({
      width: 280,
      collapsed: false
    });
    expect(normalizeInspectorLayout({ width: 900, collapsed: true })).toEqual({
      width: 560,
      collapsed: true
    });
    expect(normalizeInspectorLayout({ width: Number.NaN })).toEqual({
      width: 340,
      collapsed: false
    });
  });

  it("resizes from a right-docked pointer position", () => {
    expect(resizeInspectorFromPointer(1200, 850)).toBe(350);
    expect(resizeInspectorFromPointer(1200, 1100)).toBe(280);
    expect(resizeInspectorFromPointer(1200, 400)).toBe(560);
  });

  it("supports keyboard resizing in consistent increments", () => {
    expect(resizeInspectorByKeyboard(340, "ArrowLeft")).toBe(364);
    expect(resizeInspectorByKeyboard(340, "ArrowRight")).toBe(316);
    expect(resizeInspectorByKeyboard(280, "ArrowRight")).toBe(280);
    expect(resizeInspectorByKeyboard(560, "ArrowLeft")).toBe(560);
    expect(resizeInspectorByKeyboard(340, "Enter")).toBe(340);
  });
});
