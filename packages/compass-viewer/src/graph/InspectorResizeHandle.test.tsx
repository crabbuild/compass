import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { InspectorResizeHandle } from "./InspectorResizeHandle";

describe("InspectorResizeHandle", () => {
  it("renders an accessible separator with its current range", () => {
    const markup = renderToStaticMarkup(
      <InspectorResizeHandle width={340} onResize={vi.fn()} />
    );

    expect(markup).toContain('role="separator"');
    expect(markup).toContain('aria-label="Resize graph inspector"');
    expect(markup).toContain('aria-orientation="vertical"');
    expect(markup).toContain('aria-valuemin="280"');
    expect(markup).toContain('aria-valuemax="560"');
    expect(markup).toContain('aria-valuenow="340"');
    expect(markup).toContain('tabindex="0"');
  });
});
