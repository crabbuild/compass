import { describe, expect, it } from "vitest";
import {
  MAX_CALL_GRAPH_SYMBOL_LENGTH,
  parseCallGraphSymbolRequest
} from "./callGraphGuideMessages";

describe("call graph guide messages", () => {
  it("accepts and trims a direct symbol lookup", () => {
    expect(parseCallGraphSymbolRequest({
      type: "openSymbol",
      symbol: "  globset::GlobMatcher::is_match  ",
      direction: "both"
    })).toEqual({
      symbol: "globset::GlobMatcher::is_match",
      direction: "both"
    });
  });

  it("rejects empty, oversized, and malformed lookups", () => {
    expect(parseCallGraphSymbolRequest({
      type: "openSymbol",
      symbol: "   ",
      direction: "both"
    })).toBeUndefined();
    expect(parseCallGraphSymbolRequest({
      type: "openSymbol",
      symbol: "x".repeat(MAX_CALL_GRAPH_SYMBOL_LENGTH + 1),
      direction: "both"
    })).toBeUndefined();
    expect(parseCallGraphSymbolRequest({
      type: "openSymbol",
      symbol: "is_match",
      direction: "sideways"
    })).toBeUndefined();
  });
});
