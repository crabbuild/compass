import { describe, expect, it } from "vitest";
import type { GraphViewModel } from "../contracts/graph";
import {
  COMMUNITY_CONTROL_LIMIT,
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
