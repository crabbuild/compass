import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { describe, expect, it } from "vitest";
import type { SemanticDiffReport } from "../contracts/history";
import { SemanticFindings, SourceChangeEvidence } from "./SemanticFindings";
import { normalizeSourcePatch } from "./SourceChanges";

globalThis.ResizeObserver = class {
  observe() {}
  unobserve() {}
  disconnect() {}
};

function report(overrides: Partial<SemanticDiffReport> = {}): SemanticDiffReport {
  return {
    schema: "compass.semantic_diff.report/1",
    comparison: { old_commit: "parent", new_commit: "current", fingerprint: "fixture" },
    findings: [],
    feature_groups: [],
    collapsed_groups: [],
    source_changes: [],
    graph_delta: {
      added_nodes: [],
      removed_nodes: [],
      changed_nodes: [],
      added_edges: [],
      removed_edges: [],
      changed_edges: [],
      collapsed_attribute_changes: {}
    },
    entity_display_names: {},
    completeness: {},
    limitations: [],
    ...overrides
  };
}

function finding(
  id: string,
  headline: string,
  overrides: Partial<SemanticDiffReport["findings"][number]> = {}
): SemanticDiffReport["findings"][number] {
  return {
    id,
    finding_type: "behavior_change",
    subject: "resolver",
    origin: "direct",
    headline,
    explanation: "The implementation changed.",
    compatibility: "indeterminate",
    confidence: "unknown",
    review_priority: 3,
    public_surface: false,
    routine: false,
    affected_consumers: [],
    witness_paths: [],
    verification: {
      state: "partial",
      exact_tests: [],
      recommended_tests: ["resolver tests"],
      reason: "Test mapping is partial."
    },
    reviewer_action: "Inspect the body-only change.",
    evidence: [],
    completeness: { implementation: "partial" },
    ...overrides
  };
}

describe("SemanticFindings", () => {
  it("normalizes GitHub hunk fragments into a complete one-file patch", () => {
    expect(normalizeSourcePatch({
      old_path: "config/files.go",
      new_path: "config/files.go",
      status: "modified",
      patch: "@@ -2,2 +2,2 @@\n-old\n+new\n"
    })).toContain(
      "diff --git a/config/files.go b/config/files.go\n"
      + "--- a/config/files.go\n"
      + "+++ b/config/files.go\n"
      + "@@ -2,2 +2,2 @@"
    );
  });

  it("renders source-only changes as readable evidence instead of a raw report dump", () => {
    const container = document.createElement("div");
    const root = createRoot(container);
    flushSync(() => root.render(
        <SourceChangeEvidence
          report={report({
            source_changes: [{
              old_path: "Cargo.toml",
              new_path: "Cargo.toml",
              status: "modified",
              hunks: [{ old_start: 5, old_lines: 1, new_start: 5, new_lines: 1 }],
              patch: "-version = \"3.1.6\"\n+version = \"3.1.7\""
            }]
          })}
        />
    ));

    expect(container.querySelector("h2")?.textContent).toBe("Source changes");
    expect(container.textContent).toContain("Cargo.toml");
    expect(container.textContent).toContain("+1−1");
    expect(container.textContent).not.toContain("\"source_changes\"");
    root.unmount();
  });

  it("renders semantic evidence as comfortable expandable fields", () => {
    const container = document.createElement("div");
    const root = createRoot(container);
    flushSync(() => root.render(
      <SemanticFindings
        report={report({
          findings: [
            finding("sd1-resolver", "Resolver now falls back to the parent organization", {
              affected_consumers: [{
                symbol_id: "resolveOrgRef",
                display_name: "resolveOrgRef",
                source_file: "resolver.ts",
                distance: 1
              }]
            }),
            finding("sd1-tests", "Tests cover the new fallback", {
              evidence: [{
                source_file: "resolveref_test.go",
                start_byte: 42,
                capability: "test_mapping"
              }]
            })
          ]
        })}
      />
    ));

    const cards = container.querySelectorAll<HTMLDetailsElement>(
      ".history-finding-list details"
    );
    expect(cards).toHaveLength(2);
    expect(cards[0]?.open).toBe(true);
    expect(cards[1]?.open).toBe(false);
    expect(container.textContent).toContain("Resolver now falls back");
    expect(container.textContent).not.toContain("Finding 1");
    expect(container.textContent).toContain("Affected consumers");
    expect(container.textContent).toContain("resolveOrgRef");
    expect(container.textContent).toContain("Reviewer action");
    expect(container.textContent).toContain("Inspect the body-only change");
    root.unmount();
  });

});
