import { describe, expect, it } from "vitest";
import { graphStaticLoadingMarkup } from "./graphLoadingMarkup";

describe("graphStaticLoadingMarkup", () => {
  it("provides an accessible first paint without inline executable content", () => {
    const markup = graphStaticLoadingMarkup();

    expect(markup).toContain('role="status"');
    expect(markup).toContain('aria-live="polite"');
    expect(markup).toContain("Mapping your codebase");
    expect(markup).toContain("Reading graph");
    expect(markup).toContain("compass-load-progress");
    expect(markup).toContain("compass-load-logo");
    expect(markup).not.toContain("compass-load-graph");
    expect(markup).not.toContain("<script");
    expect(markup).not.toContain("<style");
  });
});
