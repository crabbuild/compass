import {
  architectureOverview,
  evidenceAdmits,
  lensAdmits
} from "@compass/viewer/architecture/projection";
import type {
  ArchitectureRelationshipRecord,
  ArchitectureEvidence,
  ArchitectureLens,
  ArchitectureOverview,
  ArchitectureRawGroup,
  ArchitectureRawProjection,
  ArchitectureRoutePage,
  ArchitectureScope,
  ArchitectureSearchPage,
  ArchitectureSearchResult,
  ArchitectureGroupPage,
  ArchitectureSymbol,
  ArchitectureViewModel
} from "@compass/viewer/contracts/architecture";

export type PageRequest = {
  page: number;
  pageSize: number;
  query?: string | undefined;
  scope: ArchitectureScope;
  evidence: ArchitectureEvidence;
  lens?: ArchitectureLens | undefined;
};

export type GroupPageRequest = PageRequest & {
  groupId: string;
  kind: "symbols" | "relationships";
};

export type RoutePageRequest = PageRequest & { routeId: string };
export type SearchRequest = PageRequest & { query: string };

type IndexedSymbol = ArchitectureSymbol & { normalized: string };
type IndexedRelationship = ArchitectureRelationshipRecord & { normalized: string };

export class ArchitectureIndex {
  private readonly nodeById;

  constructor(private readonly model: ArchitectureViewModel) {
    this.nodeById = new Map(model.nodes.map((node) => [node.id, node]));
  }

  overview(
    scope: ArchitectureScope,
    evidence: ArchitectureEvidence,
    lens: ArchitectureLens = "architecture"
  ): ArchitectureOverview {
    return architectureOverview(this.model, { scope, evidence, lens });
  }

  groupPage(request: GroupPageRequest): ArchitectureGroupPage {
    const context = this.context(request.scope);
    const group = context.groupById.get(request.groupId);
    if (!group) throw new Error(`Unknown architecture group '${request.groupId}'`);
    const query = normalized(request.query);
    if (request.kind === "symbols") {
      const items = this.groupNodeIds(group, context)
        .flatMap((id) => {
          const node = this.nodeById.get(id);
          if (!node) return [];
          const symbol: IndexedSymbol = {
            id: node.id,
            label: node.label,
            kind: node.kind,
            sourceFile: node.sourceFile,
            scope: node.sourceScope,
            groupId: group.id,
            normalized: normalize([node.label, node.kind, node.sourceFile ?? "", group.name.value])
          };
          return [symbol];
        })
        .filter((item) => !query || item.normalized.includes(query))
        .sort((left, right) => left.label.localeCompare(right.label) || left.id.localeCompare(right.id))
        .map(stripNormalized);
      return { kind: "symbols", groupId: group.id, ...page(items, request) };
    }
    const memberIds = new Set(this.groupNodeIds(group, context));
    const items = this.model.relationships
      .filter((item) => memberIds.has(item.source) && memberIds.has(item.target))
      .filter((item) => this.includeRelationship(item, request))
      .map((item) => this.indexedRelationship(item, group.id, group.id))
      .filter((item) => !query || item.normalized.includes(query))
      .sort(compareRelationships)
      .map(stripNormalized);
    return { kind: "relationships", groupId: group.id, ...page(items, request) };
  }

  routePage(request: RoutePageRequest): ArchitectureRoutePage {
    const context = this.context(request.scope);
    const [sourceGroup, targetGroup] = splitRouteId(request.routeId);
    if (!context.groupById.has(sourceGroup) || !context.groupById.has(targetGroup)) {
      throw new Error(`Unknown architecture route '${request.routeId}'`);
    }
    const query = normalized(request.query);
    const items = this.model.relationships
      .filter((item) =>
        context.overviewByNode.get(item.source) === sourceGroup
        && context.overviewByNode.get(item.target) === targetGroup)
      .filter((item) => this.includeRelationship(item, request))
      .map((item) => this.indexedRelationship(item, sourceGroup, targetGroup))
      .filter((item) => !query || item.normalized.includes(query))
      .sort(compareRelationships)
      .map(stripNormalized);
    return {
      routeId: request.routeId,
      sourceGroup: sourceGroup,
      targetGroup: targetGroup,
      ...page(items, request)
    };
  }

  search(request: SearchRequest): ArchitectureSearchPage {
    const query = normalized(request.query);
    const context = this.context(request.scope);
    const results: ArchitectureSearchResult[] = [];
    for (const group of context.projection.groups) {
      if (query && !normalize([group.name.value, group.ownerKey]).includes(query)) continue;
      results.push({
        id: `group:${group.id}`,
        kind: "group",
        label: group.name.value,
        detail: group.parentId ? "Subsystem" : group.kind === "owner" ? "Owner" : "Subsystem",
        groupId: group.id,
        routeId: null,
        sourceFile: null
      });
    }
    for (const node of query ? this.model.nodes : []) {
      const groupId = context.leafByNode.get(node.id);
      if (!groupId || !normalize([node.label, node.kind, node.sourceFile ?? ""]).includes(query)) continue;
      results.push({
        id: `symbol:${node.id}`,
        kind: "symbol",
        label: node.label,
        detail: [node.kind || "symbol", node.sourceFile].filter(Boolean).join(" · "),
        groupId: groupId,
        routeId: null,
        sourceFile: node.sourceFile
      });
    }
    for (const relationship of query ? this.model.relationships : []) {
      if (!this.includeRelationship(relationship, request)) continue;
      const source = this.nodeById.get(relationship.source);
      const target = this.nodeById.get(relationship.target);
      if (!normalize([
        source?.label ?? relationship.source,
        target?.label ?? relationship.target,
        relationship.relation,
        relationship.relationClass
      ]).includes(query)) continue;
      const sourceGroup = context.overviewByNode.get(relationship.source);
      const targetGroup = context.overviewByNode.get(relationship.target);
      results.push({
        id: `relationship:${relationship.id}`,
        kind: "relationship",
        label: `${source?.label ?? relationship.source} → ${target?.label ?? relationship.target}`,
        detail: `${relationship.relation} · ${relationship.relationClass} · ${relationship.confidence}`,
        groupId: context.leafByNode.get(relationship.source) ?? null,
        routeId: sourceGroup && targetGroup && sourceGroup !== targetGroup
          ? routeId(sourceGroup, targetGroup)
          : null,
        sourceFile: source?.sourceFile ?? null
      });
    }
    const rank = { group: 0, symbol: 1, relationship: 2 };
    results.sort((left, right) => rank[left.kind] - rank[right.kind] || left.label.localeCompare(right.label));
    return { query: request.query, ...page(results, request) };
  }

  private context(scope: ArchitectureScope) {
    const rawScope = scope === "all" ? "all_code" : "production";
    const projection = this.model.projections.find((candidate) => candidate.scope === rawScope);
    if (!projection) throw new Error(`Architecture projection is missing scope '${scope}'`);
    const groupById = new Map(projection.groups.map((group) => [group.id, group]));
    const leafByNode = new Map(projection.memberships.map((item) => [
      this.model.nodes[item.nodeIndex]!.id,
      projection.groups[item.groupIndex]!.id
    ]));
    const overviewByNode = new Map<string, string>();
    for (const [nodeId, leafId] of leafByNode) {
      const group = groupById.get(leafId);
      overviewByNode.set(nodeId, group?.parentId ?? leafId);
    }
    return { projection, groupById, leafByNode, overviewByNode };
  }

  private groupNodeIds(
    group: ArchitectureRawGroup,
    context: {
      projection: ArchitectureRawProjection;
      leafByNode: Map<string, string>;
      groupById: Map<string, ArchitectureRawGroup>;
    }
  ): string[] {
    const children = new Set(context.projection.groups
      .filter((candidate) => candidate.parentId === group.id)
      .map((candidate) => candidate.id));
    return [...context.leafByNode.entries()]
      .filter(([, groupId]) => groupId === group.id || children.has(groupId))
      .map(([nodeId]) => nodeId);
  }

  private includeRelationship(
    relationship: ArchitectureViewModel["relationships"][number],
    request: Pick<PageRequest, "evidence" | "lens">
  ): boolean {
    return evidenceAdmits(request.evidence, relationship.confidence)
      && lensAdmits(request.lens ?? "architecture", relationship.relationClass);
  }

  private indexedRelationship(
    relationship: ArchitectureViewModel["relationships"][number],
    sourceGroup: string,
    targetGroup: string
  ): IndexedRelationship {
    const source = this.nodeById.get(relationship.source);
    const target = this.nodeById.get(relationship.target);
    const record: ArchitectureRelationshipRecord = {
      id: relationship.id,
      source: relationship.source,
      target: relationship.target,
      sourceLabel: source?.label ?? relationship.source,
      targetLabel: target?.label ?? relationship.target,
      sourceFile: source?.sourceFile ?? null,
      targetFile: target?.sourceFile ?? null,
      sourceGroup,
      targetGroup,
      relation: relationship.relation,
      relationClass: relationship.relationClass,
      confidence: relationship.confidence
    };
    return {
      ...record,
      normalized: normalize([
        record.sourceLabel,
        record.targetLabel,
        record.sourceFile ?? "",
        record.targetFile ?? "",
        record.relation,
        record.relationClass,
        record.confidence
      ])
    };
  }
}

export function routeId(sourceGroup: string, targetGroup: string): string {
  return `${sourceGroup}->${targetGroup}`;
}

function splitRouteId(id: string): [string, string] {
  const parts = id.split("->");
  if (parts.length !== 2 || !parts[0] || !parts[1]) {
    throw new Error(`Invalid architecture route '${id}'`);
  }
  return [parts[0], parts[1]];
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

function compareRelationships(left: IndexedRelationship, right: IndexedRelationship): number {
  return left.sourceLabel.localeCompare(right.sourceLabel)
    || left.targetLabel.localeCompare(right.targetLabel)
    || left.relation.localeCompare(right.relation)
    || left.id.localeCompare(right.id);
}

function page<T>(items: readonly T[], request: Pick<PageRequest, "page" | "pageSize">) {
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
