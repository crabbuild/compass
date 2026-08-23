import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { ArchitectureHost } from "./ArchitectureFlow";
import { ArchitectureFlow } from "./ArchitectureFlow";
import type { ArchitectureOverview, ArchitectureGroupPage } from "../contracts/architecture";

const overview: ArchitectureOverview = {
  title: "Fixture",
  scope: "production",
  evidence: "all",
  lens: "architecture",
  omissions: {
    totalGroups: 1, shownGroups: 1, omittedGroups: 0,
    representedNodes: 3, omittedNodes: 0,
    representedRelationships: 2, omittedRelationships: 0,
    witnessGroupIds: [], maxOverviewGroups: 24, maxOverviewRoutes: 64
  },
  quality: {
    status: "good",
    metrics: {
      sourceScopes: { production: 2, test: 1, generated: 0, vendor: 0, documentation: 0, unknown: 0 },
      unknownSourceFraction: 0, generatedVendorLeakage: 0,
      representedNodeFraction: 1, representedRelationshipFraction: 1,
      duplicateNames: 0, fallbackNames: 0, largestGroupFraction: 1,
      unknownRelations: 0, unassignedNodes: 0, unassignedRelationships: 0
    },
    diagnostics: []
  },
  groups: [{
    id: "api", name: "API", nodeCount: 2, totalNodeCount: 3,
    internalRelationshipCount: 1, incomingRelationships: 0, outgoingRelationships: 1,
    scopes: { production: 2, test: 1, generated: 0, vendor: 0, documentation: 0, unknown: 0 }
  }],
  routes: [],
  statistics: {
    visibleNodes: 2, totalNodes: 3, visibleRelationships: 1, totalRelationships: 2,
    communities: 1, extracted: 2, inferred: 0, ambiguous: 0
  },
  coverage: { internal: 1, crossGroup: 0, unassigned: 1 },
  provenance: { projectName: "Fixture", builtAtCommit: null, generatedAt: null }
};

const page: ArchitectureGroupPage = {
  kind: "symbols",
  groupId: "api",
  page: 1,
  pageSize: 100,
  pageCount: 1,
  total: 2,
  start: 1,
  end: 2,
  items: [
    {
      id: "a", label: "handler", kind: "function",
      sourceFile: "src/api.ts", scope: "production", groupId: "api"
    },
    {
      id: "b", label: "helper", kind: "function",
      sourceFile: "src/helper.ts", scope: "production", groupId: "api"
    }
  ]
};

function host(): ArchitectureHost {
  return {
    setFilters: vi.fn(),
    requestGroup: vi.fn(),
    requestRoute: vi.fn(),
    search: vi.fn(),
    openSource: vi.fn()
  };
}

describe("ArchitectureFlow", () => {
  it("discloses production scope, full totals, and unassigned coverage", async () => {
    const adapter = host();
    render(
      <ArchitectureFlow
        overview={overview}
        groupPage={page}
        routePage={undefined}
        searchPage={undefined}
        loadingMessage={undefined}
        host={adapter}
      />
    );

    expect(screen.getByText(/Production · 2 of 3 symbols/i)).toBeInTheDocument();
    expect(screen.getByText(/1 unassigned relationships disclosed/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "All code" })).toBeInTheDocument();
    expect(screen.getByText("handler")).toBeInTheDocument();
    await waitFor(() => {
      expect(adapter.requestGroup).toHaveBeenCalledWith("api", "symbols", 1, "");
    });
  });

  it("lets the map reclaim the detail panel space", () => {
    render(
      <ArchitectureFlow
        overview={overview}
        groupPage={page}
        routePage={undefined}
        searchPage={undefined}
        loadingMessage={undefined}
        host={host()}
      />
    );

    expect(screen.getByLabelText("Architecture selection details")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Hide architecture details" }));
    expect(screen.queryByLabelText("Architecture selection details")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Show architecture details" }))
      .toBeInTheDocument();
  });

  it("shows a useful empty state when a subsystem filter has no symbol matches", () => {
    render(
      <ArchitectureFlow
        overview={overview}
        groupPage={{ ...page, total: 0, start: 0, end: 0, items: [] }}
        routePage={undefined}
        searchPage={undefined}
        loadingMessage={undefined}
        host={host()}
      />
    );

    expect(screen.getByText("No symbols match this filter.")).toBeInTheDocument();
  });

  it("opens a paged directory for groups omitted from the map", () => {
    const adapter = host();
    render(
      <ArchitectureFlow
        overview={{
          ...overview,
          omissions: { ...overview.omissions, totalGroups: 101, omittedGroups: 100 }
        }}
        groupPage={page}
        routePage={undefined}
        searchPage={{
          query: "", page: 1, pageSize: 100, pageCount: 2,
          total: 101, start: 1, end: 1,
          items: [{
            id: "group:hidden", kind: "group", label: "Hidden Runtime",
            detail: "Subsystem", groupId: "hidden", routeId: null, sourceFile: null
          }]
        }}
        loadingMessage={undefined}
        host={adapter}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: "Browse all groups" }));
    expect(adapter.search).toHaveBeenCalledWith("", 1);
    fireEvent.click(screen.getByRole("button", { name: "Next" }));
    expect(adapter.search).toHaveBeenCalledWith("", 2);
  });

});
