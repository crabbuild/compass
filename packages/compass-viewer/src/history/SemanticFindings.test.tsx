import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { describe, expect, it } from "vitest";
import { SemanticFindings } from "./SemanticFindings";

describe("SemanticFindings", () => {
  it("renders source-only changes as readable evidence instead of a raw report dump", () => {
    const container = document.createElement("div");
    const root = createRoot(container);
    flushSync(() => root.render(
        <SemanticFindings
          report={{
            schema: "compass.semantic_diff.report/1",
            comparison: { old_revision: "parent", new_revision: "current" },
            source_changes: [{
              old_path: "Cargo.toml",
              new_path: "Cargo.toml",
              status: "modified",
              hunks: [{ old_start: 5, old_lines: 1, new_start: 5, new_lines: 1 }],
              patch: "-version = \"3.1.6\"\n+version = \"3.1.7\""
            }],
            findings: [],
            completeness: {
              identity: "complete",
              source_delta: "complete",
              call_resolution: "unavailable",
              test_mapping: "unavailable"
            }
          }}
        />
    ));

    expect(container.querySelector("h2")?.textContent).toBe("Source changes");
    expect(container.textContent).toContain("Cargo.toml");
    expect(container.textContent).toContain("version = \"3.1.7\"");
    expect(container.textContent).toContain("No semantic graph findings for this comparison.");
    expect(container.textContent).not.toContain("\"source_changes\"");
    root.unmount();
  });
});
