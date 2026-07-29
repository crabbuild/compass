import { render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { ArchitectureHost } from "./ArchitectureFlow";
import { ArchitectureFlow } from "./ArchitectureFlow";
import type { ArchitectureOverview, ArchitectureSectionPage } from "../contracts/architecture";

const overview: ArchitectureOverview = {
  title: "Fixture",
  scope: "production",
  evidence: "all",
  sections: [{
    id: "api", name: "API", nodeCount: 2, totalNodeCount: 3,
    internalCallCount: 1, incomingCalls: 0, outgoingCalls: 1,
    scopes: { production: 2, test: 1, generated: 0, vendor: 0, unknown: 0 }
  }],
  routes: [],
  statistics: {
    visibleNodes: 2, totalNodes: 3, visibleCalls: 1, totalCalls: 2,
    communities: 1, extracted: 2, inferred: 0, ambiguous: 0
  },
  coverage: { internal: 1, crossSection: 0, unassigned: 1 },
  provenance: { projectName: "Fixture", builtAtCommit: null, generatedAt: null }
};

const page: ArchitectureSectionPage = {
  kind: "symbols",
  sectionId: "api",
  page: 1,
  pageSize: 100,
  pageCount: 1,
  total: 2,
  start: 1,
  end: 2,
  items: [
    {
      id: "a", label: "handler", kind: "function",
      sourceFile: "src/api.ts", scope: "production", sectionId: "api"
    },
    {
      id: "b", label: "helper", kind: "function",
      sourceFile: "src/helper.ts", scope: "production", sectionId: "api"
    }
  ]
};

function host(): ArchitectureHost {
  return {
    setFilters: vi.fn(),
    requestSection: vi.fn(),
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
        sectionPage={page}
        routePage={undefined}
        searchPage={undefined}
        loadingMessage={undefined}
        host={adapter}
      />
    );

    expect(screen.getByText(/Production · 2 of 3 symbols/i)).toBeInTheDocument();
    expect(screen.getByText(/1 unassigned calls disclosed/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "All code" })).toBeInTheDocument();
    expect(screen.getByText("handler")).toBeInTheDocument();
    await waitFor(() => {
      expect(adapter.requestSection).toHaveBeenCalledWith("api", "symbols", 1, "");
    });
  });
});
