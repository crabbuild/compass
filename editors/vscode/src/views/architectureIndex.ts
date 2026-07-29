import type {
  ArchitectureCall,
  ArchitectureEvidence,
  ArchitectureOverview,
  ArchitectureRoutePage,
  ArchitectureSearchPage,
  ArchitectureSearchResult,
  ArchitectureScope,
  ArchitectureSectionPage,
  ArchitectureSourceScope,
  ArchitectureSymbol
} from "@compass/viewer/contracts/architecture";
import type { CallflowViewModel } from "@compass/viewer/contracts/callflow";

type SourceScope = ArchitectureSourceScope;

export type PageRequest = {
  page: number;
  pageSize: number;
  query?: string | undefined;
  scope: ArchitectureScope;
  evidence: ArchitectureEvidence;
};

export type SectionPageRequest = PageRequest & {
  sectionId: string;
  kind: "symbols" | "calls";
};

export type RoutePageRequest = PageRequest & {
  routeId: string;
};

export type SearchRequest = PageRequest & {
  query: string;
};

type IndexedNode = ArchitectureSymbol & { normalized: string };
type IndexedCall = ArchitectureCall & { normalized: string };

const EMPTY_SCOPES: Record<SourceScope, number> = {
  production: 0,
  test: 0,
  generated: 0,
  vendor: 0,
  unknown: 0
};

export class ArchitectureIndex {
  private readonly sections;
  private readonly nodes = new Map<string, IndexedNode>();
  private readonly internalCalls = new Map<string, IndexedCall[]>();
  private readonly crossCalls: IndexedCall[];
  private readonly routes = new Map<string, IndexedCall[]>();

  constructor(private readonly model: CallflowViewModel) {
    this.sections = model.sections.filter((section) => section.id !== "overview");
    for (const section of this.sections) {
      for (const node of section.nodes) {
        this.nodes.set(node.id, {
          ...node,
          sectionId: section.id,
          normalized: normalize([node.label, node.kind, node.sourceFile ?? "", section.name])
        });
      }
    }
    for (const section of this.sections) {
      this.internalCalls.set(
        section.id,
        section.edges.map((edge, index) =>
          this.indexedCall(edge, section.id, section.id, `internal:${section.id}:${index}`)
        )
      );
    }
    this.crossCalls = model.crossSectionCalls.map((call, index) =>
      this.indexedCall(
        call,
        call.sourceSection,
        call.targetSection,
        `cross:${call.sourceSection}:${call.targetSection}:${index}`
      )
    );
    for (const call of this.crossCalls) {
      const id = routeId(call.sourceSection, call.targetSection);
      const calls = this.routes.get(id) ?? [];
      calls.push(call);
      this.routes.set(id, calls);
    }
  }

  overview(
    scope: ArchitectureScope,
    evidence: ArchitectureEvidence
  ): ArchitectureOverview {
    const sectionCounts = new Map<string, { incoming: number; outgoing: number }>();
    const routes = [...this.routes.entries()]
      .map(([id, calls]) => {
        const visible = calls.filter((call) => this.includeCall(call, scope, evidence));
        if (visible.length === 0) return undefined;
        sectionCounts.set(visible[0]!.sourceSection, {
          incoming: sectionCounts.get(visible[0]!.sourceSection)?.incoming ?? 0,
          outgoing:
            (sectionCounts.get(visible[0]!.sourceSection)?.outgoing ?? 0) + visible.length
        });
        sectionCounts.set(visible[0]!.targetSection, {
          incoming:
            (sectionCounts.get(visible[0]!.targetSection)?.incoming ?? 0) + visible.length,
          outgoing: sectionCounts.get(visible[0]!.targetSection)?.outgoing ?? 0
        });
        return {
          id,
          sourceSection: visible[0]!.sourceSection,
          targetSection: visible[0]!.targetSection,
          calls: visible.length,
          extracted: visible.filter((call) => call.confidence === "extracted").length,
          inferred: visible.filter((call) => call.confidence === "inferred").length,
          ambiguous: visible.filter((call) => call.confidence === "ambiguous").length
        };
      })
      .filter((route) => route !== undefined)
      .sort((left, right) => right.calls - left.calls || left.id.localeCompare(right.id));

    const sections = this.sections.map((section) => {
      const allNodes = section.nodes.map((node) => this.nodes.get(node.id)!);
      const visibleNodes = allNodes.filter((node) => this.includeNode(node, scope));
      const calls = (this.internalCalls.get(section.id) ?? [])
        .filter((call) => this.includeCall(call, scope, evidence));
      const scopes = { ...EMPTY_SCOPES };
      for (const node of allNodes) scopes[node.scope] += 1;
      return {
        id: section.id,
        name: section.name,
        nodeCount: visibleNodes.length,
        totalNodeCount: allNodes.length,
        internalCallCount: calls.length,
        incomingCalls: sectionCounts.get(section.id)?.incoming ?? 0,
        outgoingCalls: sectionCounts.get(section.id)?.outgoing ?? 0,
        scopes
      };
    });
    const visibleNodes = [...this.nodes.values()]
      .filter((node) => this.includeNode(node, scope)).length;
    const visibleInternal = [...this.internalCalls.values()].flat()
      .filter((call) => this.includeCall(call, scope, evidence)).length;
    const visibleCross = routes.reduce((total, route) => total + route.calls, 0);
    return {
      title: this.model.title,
      scope,
      evidence,
      sections,
      routes,
      statistics: {
        visibleNodes,
        totalNodes: this.model.statistics.nodes,
        visibleCalls: visibleInternal + visibleCross,
        totalCalls: this.model.statistics.edges,
        communities: this.model.statistics.communities,
        extracted: this.model.statistics.extracted,
        inferred: this.model.statistics.inferred,
        ambiguous: this.model.statistics.ambiguous
      },
      coverage: this.model.coverage,
      provenance: this.model.provenance
    };
  }

  sectionPage(request: SectionPageRequest): ArchitectureSectionPage {
    const section = this.sections.find((candidate) => candidate.id === request.sectionId);
    if (!section) throw new Error(`Unknown architecture section '${request.sectionId}'`);
    const query = normalized(request.query);
    if (request.kind === "symbols") {
      const items = section.nodes
        .map((node) => this.nodes.get(node.id)!)
        .filter((node) => this.includeNode(node, request.scope))
        .filter((node) => !query || node.normalized.includes(query))
        .sort((left, right) => left.label.localeCompare(right.label))
        .map(stripNormalized);
      return { kind: "symbols", sectionId: section.id, ...page(items, request) };
    }
    const items = (this.internalCalls.get(section.id) ?? [])
      .filter((call) => this.includeCall(call, request.scope, request.evidence))
      .filter((call) => !query || call.normalized.includes(query))
      .sort(compareCalls)
      .map(stripNormalized);
    return { kind: "calls", sectionId: section.id, ...page(items, request) };
  }

  routePage(request: RoutePageRequest): ArchitectureRoutePage {
    const calls = this.routes.get(request.routeId);
    if (!calls) throw new Error(`Unknown architecture route '${request.routeId}'`);
    const query = normalized(request.query);
    const items = calls
      .filter((call) => this.includeCall(call, request.scope, request.evidence))
      .filter((call) => !query || call.normalized.includes(query))
      .sort(compareCalls)
      .map(stripNormalized);
    return {
      routeId: request.routeId,
      sourceSection: calls[0]!.sourceSection,
      targetSection: calls[0]!.targetSection,
      ...page(items, request)
    };
  }

  search(request: SearchRequest): ArchitectureSearchPage {
    const query = normalized(request.query);
    if (!query) return { query: request.query, ...page([], request) };
    const results: ArchitectureSearchResult[] = this.sections
      .filter((section) => normalized(section.name).includes(query))
      .map((section) => ({
        id: `section:${section.id}`,
        kind: "section" as const,
        label: section.name,
        detail: "Subsystem",
        sectionId: section.id,
        routeId: null,
        sourceFile: null
      }));
    for (const node of this.nodes.values()) {
      if (!this.includeNode(node, request.scope) || !node.normalized.includes(query)) continue;
      results.push({
        id: `symbol:${node.id}`,
        kind: "symbol",
        label: node.label,
        detail: [node.kind || "symbol", node.sourceFile].filter(Boolean).join(" · "),
        sectionId: node.sectionId,
        routeId: null,
        sourceFile: node.sourceFile
      });
    }
    for (const call of [
      ...this.crossCalls,
      ...[...this.internalCalls.values()].flat()
    ]) {
      if (
        !this.includeCall(call, request.scope, request.evidence)
        || !call.normalized.includes(query)
      ) continue;
      results.push({
        id: `call:${call.id}`,
        kind: "call",
        label: `${call.sourceLabel} → ${call.targetLabel}`,
        detail: `${call.relation} · ${call.confidence}`,
        sectionId: call.sourceSection,
        routeId: call.sourceSection === call.targetSection
          ? null
          : routeId(call.sourceSection, call.targetSection),
        sourceFile: call.sourceFile
      });
    }
    results.sort((left, right) => {
      const rank = { section: 0, symbol: 1, call: 2 };
      return rank[left.kind] - rank[right.kind] || left.label.localeCompare(right.label);
    });
    return { query: request.query, ...page(results.slice(0, 100), request) };
  }

  private indexedCall(
    edge: {
      source: string;
      target: string;
      relation: string;
      confidence: "extracted" | "inferred" | "ambiguous";
    },
    sourceSection: string,
    targetSection: string,
    id: string
  ): IndexedCall {
    const source = this.nodes.get(edge.source);
    const target = this.nodes.get(edge.target);
    const call: ArchitectureCall = {
      id,
      source: edge.source,
      target: edge.target,
      sourceLabel: source?.label ?? edge.source,
      targetLabel: target?.label ?? edge.target,
      sourceFile: source?.sourceFile ?? null,
      targetFile: target?.sourceFile ?? null,
      sourceSection,
      targetSection,
      relation: edge.relation,
      confidence: edge.confidence
    };
    return {
      ...call,
      normalized: normalize([
        call.sourceLabel,
        call.targetLabel,
        call.sourceFile ?? "",
        call.targetFile ?? "",
        call.relation,
        call.confidence
      ])
    };
  }

  private includeNode(node: IndexedNode | undefined, scope: ArchitectureScope): boolean {
    return Boolean(node && (scope === "all" || node.scope === "production"));
  }

  private includeCall(
    call: IndexedCall,
    scope: ArchitectureScope,
    evidence: ArchitectureEvidence
  ): boolean {
    return this.includeNode(this.nodes.get(call.source), scope)
      && this.includeNode(this.nodes.get(call.target), scope)
      && (evidence === "all" || call.confidence === evidence);
  }
}

export function routeId(sourceSection: string, targetSection: string): string {
  return `${encodeURIComponent(sourceSection)}→${encodeURIComponent(targetSection)}`;
}

function normalize(values: readonly string[]): string {
  return values.join("\u0000").toLocaleLowerCase();
}

function normalized(value: string | undefined): string {
  return value?.trim().toLocaleLowerCase() ?? "";
}

function stripNormalized<T extends { normalized: string }>({
  normalized: _normalized,
  ...value
}: T): Omit<T, "normalized"> {
  return value;
}

function compareCalls(left: IndexedCall, right: IndexedCall): number {
  return left.sourceLabel.localeCompare(right.sourceLabel)
    || left.targetLabel.localeCompare(right.targetLabel)
    || left.relation.localeCompare(right.relation);
}

function page<T>(
  items: readonly T[],
  request: Pick<PageRequest, "page" | "pageSize">
) {
  const pageSize = Math.min(100, Math.max(1, Math.trunc(request.pageSize)));
  const pageCount = Math.max(1, Math.ceil(items.length / pageSize));
  const current = Math.min(pageCount, Math.max(1, Math.trunc(request.page)));
  const offset = (current - 1) * pageSize;
  const visible = items.slice(offset, offset + pageSize);
  return {
    items: visible,
    page: current,
    pageSize,
    pageCount,
    total: items.length,
    start: visible.length === 0 ? 0 : offset + 1,
    end: offset + visible.length
  };
}
