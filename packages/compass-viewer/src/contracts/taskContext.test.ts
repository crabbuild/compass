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
    for (const id of [
      "django-python",
      "fastapi-python",
      "flask-python",
      "pydantic-python",
      "starlette-python",
    ]) {
      const python = frameworkContext();
      python.packs[0]!.id = id;
      expect(FrameworkContextSchema.parse(python).packs.at(0)?.id).toBe(id);
    }
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

  it("round-trips dependency and security stages and rejects unknown stages", () => {
    const withStages = frameworkContext() as Record<string, unknown>;
    withStages.routes = [{
      nodeId: "route:users",
      framework: "fastapi",
      operation: "GET",
      path: "/users",
      declaringScope: "app.routes",
      resolution: "exact",
      stages: ["dependency", "security"].map((stage, position) => ({
        stage,
        position,
        reference: `${stage}_provider`,
        resolution: "exact",
        source: null,
        target: `${stage}:provider`,
        provenance: []
      })),
      provenance: []
    }];
    const decoded = FrameworkContextSchema.parse(withStages);
    expect(decoded.routes[0]?.stages.map((stage) => stage.stage)).toEqual([
      "dependency", "security"
    ]);

    const unknownStage = structuredClone(withStages) as Record<string, unknown>;
    const routes = unknownStage.routes as Array<Record<string, unknown>>;
    const stages = routes[0]?.stages as Array<Record<string, unknown>>;
    stages[0]!.stage = "authorization";
    expect(() => FrameworkContextSchema.parse(unknownStage)).toThrow();
  });
});
