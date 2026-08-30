import { describe, expect, it } from "vitest";
import type { GraphViewModel } from "../contracts/graph";
import {
  COMMUNITY_CONTROL_LIMIT,
  groupDirectionalRelationships,
  visibleCommunityControls
} from "./GraphInspector";

const communities: GraphViewModel["communities"] = Array.from(
  { length: 3_376 },
  (_, id) => ({
    id,
    label: id === 2_731 ? "Django request handlers" : `Community ${id}`,
    color: "#4e79a7",
    hidden: false
  })
);

describe("visibleCommunityControls", () => {
  it("bounds the default inspector DOM for repositories with thousands of communities", () => {
    expect(visibleCommunityControls(communities, "")).toHaveLength(COMMUNITY_CONTROL_LIMIT);
  });

  it("can find a community outside the initial bounded window", () => {
    expect(visibleCommunityControls(communities, "request handlers")).toEqual([
      communities[2_731]
    ]);
    expect(visibleCommunityControls(communities, "2731")).toEqual([
      communities[2_731]
    ]);
  });
});

describe("groupDirectionalRelationships", () => {
  it("groups directed parallel edges without losing relation multiplicity", () => {
    const selected = {
      id: "selected",
      label: "Selected",
      community: 0
    };
    const caller = {
      id: "caller",
      label: "Caller",
      community: 0
    };
    const callee = {
      id: "callee",
      label: "Callee",
      community: 0
    };
    const nodes = new Map([
      [selected.id, selected],
      [caller.id, caller],
      [callee.id, callee]
    ]);
    const relationships = groupDirectionalRelationships("selected", [
      { id: "in-2", source: "caller", target: "selected", relation: "calls" },
      { id: "out-1", source: "selected", target: "callee", relation: "imports" },
      { id: "in-1", source: "caller", target: "selected", relation: "calls" }
    ], nodes);

    expect(relationships.incoming).toHaveLength(1);
    expect(relationships.incoming[0]?.node.id).toBe("caller");
    expect(relationships.incoming[0]?.edges.map((edge) => edge.id)).toEqual(["in-1", "in-2"]);
    expect(relationships.outgoing).toHaveLength(1);
    expect(relationships.outgoing[0]?.node.id).toBe("callee");
  });

  it("represents a self-loop in both directions and ignores non-incident edges", () => {
    const selected = {
      id: "selected",
      label: "Selected",
      community: 0
    };
    const nodes = new Map([[selected.id, selected]]);
    const relationships = groupDirectionalRelationships("selected", [
      { id: "self", source: "selected", target: "selected", relation: "recurses" },
      { id: "other", source: "a", target: "b", relation: "calls" }
    ], nodes);

    expect(relationships.incoming[0]?.edges[0]?.id).toBe("self");
    expect(relationships.outgoing[0]?.edges[0]?.id).toBe("self");
  });
});
