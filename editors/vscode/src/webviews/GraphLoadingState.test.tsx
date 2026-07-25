import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { GraphLoadingState } from "./GraphLoadingState";

describe("GraphLoadingState", () => {
  it("announces mapping while keeping the constellation decorative", () => {
    const markup = renderToStaticMarkup(
      <GraphLoadingState
        state={{ kind: "loading" }}
        onRetry={vi.fn()}
        onShowOutput={vi.fn()}
      />
    );

    expect(markup).toContain('role="status"');
    expect(markup).toContain("Mapping your codebase");
    expect(markup).toContain("Arranging relationships");
    expect(markup).toContain('data-testid="graph-constellation"');
    expect(markup).toContain('aria-hidden="true"');
  });

  it("shows a recoverable error with both host actions", () => {
    const markup = renderToStaticMarkup(
      <GraphLoadingState
        state={{ kind: "error", message: "viewer export failed" }}
        onRetry={vi.fn()}
        onShowOutput={vi.fn()}
      />
    );

    expect(markup).toContain('role="alert"');
    expect(markup).toContain("viewer export failed");
    expect(markup).toContain("Retry");
    expect(markup).toContain("Show Compass output");
  });

  it("accepts purpose-specific loading copy", () => {
    const markup = renderToStaticMarkup(
      <GraphLoadingState
        state={{ kind: "loading" }}
        loadingCopy={{
          eyebrow: "Compass call graph",
          title: "Resolving the function under your cursor",
          steps: ["Locating symbol", "Tracing callers", "Tracing callees"]
        }}
        onRetry={vi.fn()}
        onShowOutput={vi.fn()}
      />
    );

    expect(markup).toContain("Compass call graph");
    expect(markup).toContain("Resolving the function under your cursor");
    expect(markup).toContain("Tracing callers");
    expect(markup).toContain("Tracing callees");
  });
});
