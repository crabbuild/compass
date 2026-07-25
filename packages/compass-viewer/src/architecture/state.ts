import type { CallflowViewModel } from "../contracts/callflow";

export type ArchitectureSection = CallflowViewModel["sections"][number];
export type ArchitectureEdge = ArchitectureSection["edges"][number];

export type ArchitectureResult = {
  id: string;
  kind: "section" | "symbol" | "call";
  sectionId: string;
  sectionName: string;
  label: string;
  detail: string;
  query: string;
  tab: "symbols" | "calls";
};

export type ArchitectureResultGroup = {
  sectionId: string;
  sectionName: string;
  results: ArchitectureResult[];
};

export type CallSort = {
  column: "caller" | "relation" | "callee" | "confidence";
  direction: "ascending" | "descending";
};

function normalized(value: string): string {
  return value.trim().toLocaleLowerCase();
}

export function nodeNameMap(model: CallflowViewModel): Map<string, string> {
  return new Map(
    model.sections.flatMap((section) =>
      section.nodes.map((node) => [node.id, node.label] as const)
    )
  );
}

export function filterSectionSymbols(
  section: ArchitectureSection,
  query: string
): ArchitectureSection["nodes"] {
  const needle = normalized(query);
  if (!needle) return section.nodes;
  return section.nodes.filter((node) =>
    [node.label, node.kind, node.sourceFile ?? ""]
      .some((value) => value.toLocaleLowerCase().includes(needle))
  );
}

export function filterSectionCalls(
  section: ArchitectureSection,
  names: ReadonlyMap<string, string>,
  query: string
): ArchitectureEdge[] {
  const needle = normalized(query);
  if (!needle) return section.edges;
  return section.edges.filter((edge) =>
    [
      names.get(edge.source) ?? edge.source,
      names.get(edge.target) ?? edge.target,
      edge.relation,
      edge.confidence
    ].some((value) => value.toLocaleLowerCase().includes(needle))
  );
}

export function sortCalls(
  edges: readonly ArchitectureEdge[],
  names: ReadonlyMap<string, string>,
  sort: CallSort
): ArchitectureEdge[] {
  const value = (edge: ArchitectureEdge): string => {
    if (sort.column === "caller") return names.get(edge.source) ?? edge.source;
    if (sort.column === "callee") return names.get(edge.target) ?? edge.target;
    return edge[sort.column];
  };
  const direction = sort.direction === "ascending" ? 1 : -1;
  return [...edges].sort((left, right) =>
    value(left).localeCompare(value(right), undefined, { sensitivity: "base" }) * direction
  );
}

export function searchArchitecture(
  model: CallflowViewModel,
  query: string
): ArchitectureResultGroup[] {
  const needle = normalized(query);
  if (!needle) return [];
  const names = nodeNameMap(model);
  const groups: ArchitectureResultGroup[] = [];

  for (const section of model.sections) {
    if (section.id === "overview") continue;
    const results: ArchitectureResult[] = [];
    if (section.name.toLocaleLowerCase().includes(needle)) {
      results.push({
        id: `section:${section.id}`,
        kind: "section",
        sectionId: section.id,
        sectionName: section.name,
        label: section.name,
        detail: `${section.nodes.length} symbols · ${section.edges.length} calls`,
        query: "",
        tab: "symbols"
      });
    }
    for (const node of filterSectionSymbols(section, query)) {
      results.push({
        id: `symbol:${section.id}:${node.id}`,
        kind: "symbol",
        sectionId: section.id,
        sectionName: section.name,
        label: node.label,
        detail: [node.kind || "symbol", node.sourceFile].filter(Boolean).join(" · "),
        query: node.label,
        tab: "symbols"
      });
    }
    for (const [index, edge] of filterSectionCalls(section, names, query).entries()) {
      const caller = names.get(edge.source) ?? edge.source;
      const callee = names.get(edge.target) ?? edge.target;
      results.push({
        id: `call:${section.id}:${edge.source}:${edge.target}:${index}`,
        kind: "call",
        sectionId: section.id,
        sectionName: section.name,
        label: `${caller} → ${callee}`,
        detail: `${edge.relation} · ${edge.confidence}`,
        query: query,
        tab: "calls"
      });
    }
    if (results.length > 0) {
      const rank = { section: 0, symbol: 1, call: 2 };
      groups.push({
        sectionId: section.id,
        sectionName: section.name,
        results: results.sort((left, right) =>
          rank[left.kind] - rank[right.kind]
          || left.label.localeCompare(right.label, undefined, { sensitivity: "base" })
        )
      });
    }
  }
  return groups;
}
