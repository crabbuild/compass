import type {
  ArchitectureEvidence,
  ArchitectureLens,
  ArchitectureOverview,
  ArchitectureScope,
  ArchitectureViewModel
} from "../contracts/architecture";

export type ArchitectureOverviewFilters = {
  scope: ArchitectureScope;
  evidence: ArchitectureEvidence;
  lens: ArchitectureLens;
};

export function architectureOverview(
  model: ArchitectureViewModel,
  filters: ArchitectureOverviewFilters
): ArchitectureOverview {
  const projection = model.projections.find((candidate) =>
    candidate.scope === (filters.scope === "all" ? "all_code" : "production"));
  if (!projection) throw new Error(`Architecture projection is missing scope '${filters.scope}'`);
  const groupById = new Map(projection.groups.map((group) => [group.id, group]));
  const shown = new Set(projection.overviewGroupIds);
  const leafByNode = new Map(projection.memberships.map((item) => [
    model.nodes[item.nodeIndex]!.id,
    projection.groups[item.groupIndex]!.id
  ]));
  const overviewGroup = (nodeId: string): string | undefined => {
    const leaf = leafByNode.get(nodeId);
    if (!leaf) return undefined;
    const group = groupById.get(leaf);
    return group?.parentId ?? leaf;
  };
  const admitted = model.relationships.filter((relationship) =>
    evidenceAdmits(filters.evidence, relationship.confidence)
    && lensAdmits(filters.lens, relationship.relationClass)
    && overviewGroup(relationship.source)
    && overviewGroup(relationship.target));
  const internal = new Map<string, number>();
  const routeBuckets = new Map<string, typeof admitted>();
  for (const relationship of admitted) {
    const source = overviewGroup(relationship.source)!;
    const target = overviewGroup(relationship.target)!;
    if (source === target) {
      internal.set(source, (internal.get(source) ?? 0) + 1);
      continue;
    }
    if (!shown.has(source) || !shown.has(target)) continue;
    const id = `${source}->${target}`;
    const bucket = routeBuckets.get(id) ?? [];
    bucket.push(relationship);
    routeBuckets.set(id, bucket);
  }
  const routes = [...routeBuckets.entries()].map(([id, relationships]) => ({
    id,
    sourceGroup: overviewGroup(relationships[0]!.source)!,
    targetGroup: overviewGroup(relationships[0]!.target)!,
    relationships: relationships.length,
    extracted: relationships.filter((item) => item.confidence === "extracted").length,
    inferred: relationships.filter((item) => item.confidence === "inferred").length,
    ambiguous: relationships.filter((item) => item.confidence === "ambiguous").length
  })).sort((left, right) => right.relationships - left.relationships || left.id.localeCompare(right.id));
  const incoming = new Map<string, number>();
  const outgoing = new Map<string, number>();
  for (const route of routes) {
    outgoing.set(route.sourceGroup, (outgoing.get(route.sourceGroup) ?? 0) + route.relationships);
    incoming.set(route.targetGroup, (incoming.get(route.targetGroup) ?? 0) + route.relationships);
  }
  const groups = projection.overviewGroupIds.flatMap((id) => {
    const group = groupById.get(id);
    if (!group) return [];
    return [{
      id: group.id,
      name: group.name.value,
      nodeCount: group.nodeCount,
      totalNodeCount: group.nodeCount,
      internalRelationshipCount: internal.get(group.id) ?? 0,
      incomingRelationships: incoming.get(group.id) ?? 0,
      outgoingRelationships: outgoing.get(group.id) ?? 0,
      scopes: group.sourceScopes
    }];
  });
  const shownNodes = groups.reduce((sum, group) => sum + group.nodeCount, 0);
  const shownRelationships = groups.reduce((sum, group) => sum + group.internalRelationshipCount, 0)
    + routes.reduce((sum, route) => sum + route.relationships, 0);
  return {
    title: model.title,
    scope: filters.scope,
    evidence: filters.evidence,
    lens: filters.lens,
    groups,
    routes,
    statistics: {
      visibleNodes: shownNodes,
      totalNodes: projection.quality.metrics.sourceScopes.production
        + projection.quality.metrics.sourceScopes.test
        + projection.quality.metrics.sourceScopes.generated
        + projection.quality.metrics.sourceScopes.vendor
        + projection.quality.metrics.sourceScopes.unknown,
      visibleRelationships: shownRelationships,
      totalRelationships: admitted.length,
      communities: model.statistics.communities,
      extracted: model.statistics.extracted,
      inferred: model.statistics.inferred,
      ambiguous: model.statistics.ambiguous
    },
    coverage: {
      internal: projection.coverage.internal,
      crossGroup: projection.coverage.crossGroup,
      unassigned: projection.coverage.unassigned
    },
    omissions: projection.omissions,
    quality: projection.quality,
    provenance: model.provenance
  };
}

export function evidenceAdmits(filter: ArchitectureEvidence, confidence: string): boolean {
  return filter === "all" || filter === confidence;
}

export function lensAdmits(lens: ArchitectureLens, relationClass: string): boolean {
  if (lens === "all") return relationClass !== "unknown";
  if (lens === "architecture") return relationClass === "execution" || relationClass === "dependency";
  return lens === relationClass;
}
