import { describe, expect, it } from "vitest";

import { FrameworkContextSchema } from "./taskContext";

function frameworkContext() {
  return {
    schema: "compass.framework-context/1",
    graphIdentity: "sha256:graph",
    buildGenerationIdentity: "sha256:generation",
    focusNodeId: null,
    packs: [{
      id: "react-ui",
      version: 2,
      qualification: "qualifying",
      capabilities: ["ui"],
      observedNodes: 1,
      observedRelations: 1
    }],
    routes: [],
    relations: [],
    renderedBy: [],
    renders: [],
    configDependencies: [],
    runtimeBoundaries: [],
    unsupported: [],
    incomplete: [],
    ambiguities: [],
    truncated: false,
    recordLimit: 256,
    byteLimit: 262144
  };
}

describe("framework task context contract", () => {
  it("accepts the versioned bounded framework section", () => {
    expect(FrameworkContextSchema.parse(frameworkContext()).packs.at(0)?.id).toBe("react-ui");
  });

  it("fails closed for unknown pack IDs and qualification states", () => {
    const unknownPack = frameworkContext();
    unknownPack.packs.at(0)!.id = "future-framework";
    expect(() => FrameworkContextSchema.parse(unknownPack)).toThrow();

    const unknownState = frameworkContext() as Record<string, unknown>;
    (unknownState.packs as Array<Record<string, unknown>>).at(0)!.qualification = "future";
    expect(() => FrameworkContextSchema.parse(unknownState)).toThrow();
  });

  it("rejects unknown fields instead of silently widening the contract", () => {
    expect(() => FrameworkContextSchema.parse({ ...frameworkContext(), extra: true })).toThrow();
  });
});
