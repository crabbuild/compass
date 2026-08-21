import { describe, expect, it } from "vitest";
import { queryFailureMessage } from "./queryExecution";

describe("queryFailureMessage", () => {
  it("extracts actionable text from a machine-readable error envelope", () => {
    expect(queryFailureMessage(
      '{"error":{"code":"no_match","message":"No exact symbol matched Checkout.run"}}',
      1
    )).toBe("No exact symbol matched Checkout.run");
  });

  it("uses typed diagnostics instead of displaying the JSON blob", () => {
    expect(queryFailureMessage(
      '{"diagnostics":[{"code":"ambiguous_match","message":"Use a full node ID"}]}',
      1
    )).toBe("Use a full node ID");
  });

  it("preserves concise plain-text CLI failures", () => {
    expect(queryFailureMessage("error: graph is unavailable\n", 2))
      .toBe("Graph is unavailable");
  });

  it("does not expose an unknown machine envelope as raw JSON", () => {
    const message = queryFailureMessage('{"code":"future_error","payload":{"node":"run"}}', 1);
    expect(message).toContain("could not interpret");
    expect(message).not.toContain("future_error");
    expect(message).not.toContain("payload");
  });
});
