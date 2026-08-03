import { describe, expect, it } from "vitest";
import { SemanticDiffReportSchema } from "./history";

function currentReport() {
  return {
    schema: "compass.semantic_diff.report/1",
    comparison: {
      old_commit: "23e35d792b9154f922b8b575b12596a4d8664c65",
      new_commit: "578eeb702ec0fbb6b9780f3d4147b1076630d633",
      fingerprint: "fixture-fingerprint"
    },
    findings: [{
      id: "sd1-hash",
      finding_type: "behavior_change",
      subject: "sha256:hash",
      origin: "direct",
      headline: "Hash() implementation changed",
      explanation: "An implementation digest changed without supported semantic evidence.",
      compatibility: "indeterminate",
      confidence: "unknown",
      review_priority: 3,
      public_surface: false,
      routine: false,
      before: {
        body_digest: "0e8905",
        signature_digest: "unchanged",
        source_file: "util/hash.cc"
      },
      after: {
        body_digest: "3286cd",
        signature_digest: "unchanged",
        source_file: "util/hash.cc"
      },
      affected_consumers: [],
      witness_paths: [],
      verification: {
        state: "partial",
        exact_tests: [],
        recommended_tests: [],
        reason: "Test mapping is partial."
      },
      reviewer_action: "Inspect the body-only change.",
      evidence: [{
        source_file: "util/hash.cc",
        start_byte: 17,
        end_byte: 64,
        record_key: "sha256:hash",
        capability: "implementation"
      }],
      completeness: {
        call_resolution: "partial",
        implementation: "partial",
        signature: "complete",
        test_mapping: "partial"
      }
    }],
    feature_groups: [],
    collapsed_groups: [],
    source_changes: [{
      old_path: "util/hash.cc",
      new_path: "util/hash.cc",
      status: "modified",
      hunks: [{ old_start: 1, old_lines: 3, new_start: 1, new_lines: 3 }],
      patch: "@@ -1,3 +1,3 @@\n-old\n+new"
    }],
    graph_delta: {
      added_nodes: [],
      removed_nodes: [],
      changed_nodes: [{
        id: "sha256:hash",
        label: "Hash",
        kind: "function",
        source_file: "util/hash.cc",
        changed_fields: ["details"]
      }],
      added_edges: [],
      removed_edges: [],
      changed_edges: [],
      collapsed_attribute_changes: {}
    },
    entity_display_names: { "sha256:hash": "Hash" },
    completeness: {
      identity: "complete",
      source_delta: "complete",
      call_resolution: "partial",
      test_mapping: "partial"
    },
    limitations: ["Call mapping is partial."]
  };
}

describe("SemanticDiffReportSchema", () => {
  it("accepts the current Rust report including exact graph delta evidence", () => {
    const parsed = SemanticDiffReportSchema.parse(currentReport());

    expect(parsed.findings[0]?.headline).toBe("Hash() implementation changed");
    expect(parsed.graph_delta.changed_nodes).toEqual([expect.objectContaining({
      id: "sha256:hash",
      changed_fields: ["details"]
    })]);
    expect(parsed.graph_delta.changed_edges).toEqual([]);
  });

  it("rejects unknown versions and malformed edge records", () => {
    const unknownVersion = currentReport();
    unknownVersion.schema = "compass.semantic_diff.report/2";
    expect(SemanticDiffReportSchema.safeParse(unknownVersion).success).toBe(false);

    const malformed = currentReport();
    malformed.graph_delta.added_edges = [{
      source: "a",
      target: "b",
      relation: "calls",
      key: "a-b",
      source_file: "src/lib.rs",
      changed_fields: "confidence"
    }] as never;
    expect(SemanticDiffReportSchema.safeParse(malformed).success).toBe(false);
  });
});
