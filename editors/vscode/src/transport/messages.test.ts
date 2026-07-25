import { describe, expect, it } from "vitest";
import {
  GraphToHostMessageSchema,
  HostToGraphMessageSchema
} from "./messages";

describe("community graph messages", () => {
  it("accepts non-negative community requests and rejects invalid IDs", () => {
    expect(GraphToHostMessageSchema.safeParse({
      type: "openCommunity",
      requestId: "request-1",
      repositoryId: "repository-1",
      communityId: 7
    }).success).toBe(true);
    expect(GraphToHostMessageSchema.safeParse({
      type: "openCommunity",
      requestId: "request-1",
      repositoryId: "repository-1",
      communityId: -1
    }).success).toBe(false);
  });

  it("requires response identity for stale-response protection", () => {
    expect(HostToGraphMessageSchema.safeParse({
      type: "communityError",
      requestId: "request-1",
      communityId: 7,
      message: "failed"
    }).success).toBe(true);
    expect(HostToGraphMessageSchema.safeParse({
      type: "communityError",
      communityId: 7,
      message: "failed"
    }).success).toBe(false);
  });
});
