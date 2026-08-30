import { describe, expect, it } from "vitest";
import {
  MAX_CALL_GRAPH_SYMBOL_LENGTH,
  callGraphCompletionTerm,
  parseCallGraphCompletionItems,
  parseCallGraphCompletionRequest,
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

  it("accepts bounded Unicode graph completion requests", () => {
    expect(parseCallGraphCompletionRequest({
      type: "completeSymbol",
      requestId: "completion-1",
      term: "支付::处理"
    })).toEqual({ requestId: "completion-1", term: "支付::处理" });
    expect(parseCallGraphCompletionRequest({
      type: "completeSymbol",
      requestId: "completion-2",
      term: "two words"
    })).toBeUndefined();
    expect(parseCallGraphCompletionRequest({
      type: "completeSymbol",
      requestId: "x".repeat(129),
      term: "Node"
    })).toBeUndefined();
    expect(callGraphCompletionTerm("  PaymentService  ")).toBe("PaymentService");
    expect(callGraphCompletionTerm("--graph")).toBeUndefined();
  });

  it("validates and bounds completion responses before rendering", () => {
    const item = {
      nodeId: "node-1",
      label: "crate::run",
      insertText: "crate::run",
      detail: "function · src/main.rs:4"
    };
    expect(parseCallGraphCompletionItems([item])).toEqual([item]);
    expect(parseCallGraphCompletionItems(Array(9).fill(item))).toBeUndefined();
    expect(parseCallGraphCompletionItems([{ ...item, label: "x".repeat(513) }]))
      .toBeUndefined();
  });
});
