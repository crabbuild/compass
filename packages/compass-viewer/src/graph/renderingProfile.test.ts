import { describe, expect, it } from "vitest";
import type { GraphNode, GraphViewModel } from "../contracts/graph";
import {
  centerCommunityOverviewPositions,
  relaxCommunityOverviewPositions,
  AGGREGATED_EDGE_RENDER_LIMIT,
  graphRenderingProfile,
  seedCommunityOverviewPositions,
  seedGraphLayoutPositions,
  seedStaticGraphPositions,
  STATIC_LAYOUT_EDGE_THRESHOLD,
  STATIC_LAYOUT_NODE_THRESHOLD,
  communitySeedPositions,
  stableBearing,
  visibleGraphEdges
} from "./renderingProfile";
import {
  communityNodeSize,
  communityOverviewLabelText,
  communityOverviewLabelledIds
} from "./communityOverview";

function model(nodes: number, edges: number): GraphViewModel {
  return {
    schema: "compass.viewer.graph/1",
    title: "Fixture",
    stats: { nodes, edges, communities: 1, aggregated: false },
    nodes: Array.from({ length: nodes }, (_, index) => ({
      id: `n-${index}`,
      label: `Node ${index}`,
      community: 0
    })),
    edges: Array.from({ length: edges }, (_, index) => ({
      id: `e-${index}`,
      source: "n-0",
      target: "n-1",
      relation: "calls"
    })),
    communities: [{ id: 0, label: "Core", color: "#4e79a7", hidden: false }],
    hyperedges: []
  };
}

describe("graphRenderingProfile", () => {
  it("keeps small and sparse graphs interactive", () => {
    expect(graphRenderingProfile(model(
      STATIC_LAYOUT_NODE_THRESHOLD - 1,
      STATIC_LAYOUT_EDGE_THRESHOLD - 1
    ))).toBe("interactive");
  });

  it("selects static rendering for either a large or dense graph", () => {
    expect(graphRenderingProfile(model(STATIC_LAYOUT_NODE_THRESHOLD, 0))).toBe("static");
    expect(graphRenderingProfile(model(2, STATIC_LAYOUT_EDGE_THRESHOLD))).toBe("static");
  });
});

describe("seedStaticGraphPositions", () => {
  it("produces stable positions independent of input order", () => {
    const nodes: GraphNode[] = [
      { id: "beta", label: "Beta", community: 2 },
      { id: "alpha", label: "Alpha", community: 1 },
      { id: "gamma", label: "Gamma", community: 1 }
    ];

    expect([...seedStaticGraphPositions(nodes)]).toEqual([
      ...seedStaticGraphPositions([...nodes].reverse())
    ]);
  });

  it("assigns every node a distinct position", () => {
    const nodes = model(1_500, 0).nodes;
    const positions = seedStaticGraphPositions(nodes);
    const coordinates = new Set(
      [...positions.values()].map(({ x, y }) => `${x.toFixed(6)},${y.toFixed(6)}`)
    );

    expect(positions.size).toBe(nodes.length);
    expect(coordinates.size).toBe(nodes.length);
  });

  it("centres large communities and scatters the tail outside", () => {
    const nodes: GraphNode[] = Array.from({ length: 120 }, (_, index) => ({
      id: `community-${index}`,
      label: `Community ${index}`,
      community: index,
      degree: index % 17,
      memberCount: 120 - index,
      size: 8 + 10 * Math.sqrt((120 - index) / 120)
    }));
    const positions = seedStaticGraphPositions(nodes, true);
    expect([...positions]).toEqual([
      ...seedStaticGraphPositions([...nodes].reverse(), true)
    ]);
    expect(positions.size).toBe(nodes.length);

    const coordinates = [...positions.values()];
    const left = Math.min(...coordinates.map(({ x }) => x));
    const right = Math.max(...coordinates.map(({ x }) => x));
    const top = Math.min(...coordinates.map(({ y }) => y));
    const bottom = Math.max(...coordinates.map(({ y }) => y));
    // The map widens toward the canvas aspect instead of leaving the side
    // margins of a square grid empty.
    expect((right - left) / (bottom - top)).toBeGreaterThan(1.15);

    // Importance reads as centrality: the first half of the ranking holds the
    // middle, the second half is scattered further out.
    const centroid = {
      x: (left + right) / 2,
      y: (top + bottom) / 2
    };
    const meanRadius = (entries: readonly GraphNode[]) => entries.reduce(
      (sum, entry) => {
        const position = positions.get(entry.id)!;
        return sum + Math.hypot(position.x - centroid.x, position.y - centroid.y);
      },
      0
    ) / entries.length;
    expect(meanRadius(nodes.slice(0, nodes.length / 2)))
      .toBeLessThan(meanRadius(nodes.slice(nodes.length / 2)));

    // Rendered bubbles never overlap, so the overview stays readable.
    for (let left_index = 0; left_index < nodes.length; left_index += 1) {
      for (let right_index = left_index + 1; right_index < nodes.length; right_index += 1) {
        const first = nodes[left_index]!;
        const second = nodes[right_index]!;
        const firstPosition = positions.get(first.id)!;
        const secondPosition = positions.get(second.id)!;
        const distance = Math.hypot(
          firstPosition.x - secondPosition.x,
          firstPosition.y - secondPosition.y
        );
        expect(distance).toBeGreaterThanOrEqual(
          (first.size ?? 0) + (second.size ?? 0) - 0.001
        );
      }
    }
  });
});

describe("centerCommunityOverviewPositions", () => {
  it("moves each rank toward the radius its importance earns", () => {
    const nodes: GraphNode[] = Array.from({ length: 9 }, (_, index) => ({
      id: `community:${index}`,
      label: `module_${index}.ts`,
      community: index,
      memberCount: 90 - index * 10,
      degree: 0,
      size: 10
    }));
    // A settled square whose order has nothing to do with importance.
    const settled = new Map(nodes.map((node, index) => [
      node.id,
      { x: (index % 3) * 120, y: Math.floor(index / 3) * 120 }
    ]));
    const centered = centerCommunityOverviewPositions(nodes, settled, undefined, 1);
    const centroid = { x: 120, y: 120 };
    const radius = (id: string) => {
      const position = centered.get(id)!;
      return Math.hypot(position.x - centroid.x, position.y - centroid.y);
    };

    expect(radius("community:0")).toBeLessThan(radius("community:4"));
    expect(radius("community:4")).toBeLessThan(radius("community:8"));
    // The angular reading from the arrangement survives the radial move.
    const before = settled.get("community:8")!;
    const after = centered.get("community:8")!;
    expect(Math.sign(after.x - centroid.x)).toBe(Math.sign(before.x - centroid.x));
    expect([...centerCommunityOverviewPositions(nodes, settled, undefined, 1)])
      .toEqual([...centered]);
  });
});

describe("relaxCommunityOverviewPositions", () => {
  it("separates overlapping bubbles while keeping the settled arrangement", () => {
    const nodes: GraphNode[] = Array.from({ length: 12 }, (_, index) => ({
      id: `community:${index}`,
      label: `module_${index}.ts`,
      community: index,
      memberCount: 40 - index,
      degree: index % 5,
      size: 8 + (index % 3) * 4
    }));
    // Every bubble starts on top of the others, as a collapsed simulation would.
    const collapsed = new Map(nodes.map((node) => [node.id, { x: 0, y: 0 }]));
    const relaxed = relaxCommunityOverviewPositions(nodes, collapsed);
    const positions = [...relaxed.values()];

    expect(relaxed.size).toBe(nodes.length);
    expect(positions.some(({ x, y }) => x !== 0 || y !== 0)).toBe(true);
    // Footprints keep at least their own width apart, which is what keeps the
    // label under each bubble readable.
    for (let left = 0; left < nodes.length; left += 1) {
      for (let right = left + 1; right < nodes.length; right += 1) {
        const first = relaxed.get(nodes[left]!.id)!;
        const second = relaxed.get(nodes[right]!.id)!;
        const separated = Math.abs(first.x - second.x) >= 2 * (nodes[left]!.size ?? 0)
          || Math.abs(first.y - second.y) >= 2 * (nodes[right]!.size ?? 0);
        expect(separated).toBe(true);
      }
    }
  });

  it("is deterministic and leaves a non-overlapping layout untouched", () => {
    const nodes: GraphNode[] = Array.from({ length: 6 }, (_, index) => ({
      id: `community:${index}`,
      label: `module_${index}.ts`,
      community: index,
      memberCount: 10,
      degree: 1,
      size: 10
    }));
    const spaced = new Map(nodes.map((node, index) => [
      node.id,
      { x: index * 400, y: 0 }
    ]));
    const first = relaxCommunityOverviewPositions(nodes, spaced);
    const second = relaxCommunityOverviewPositions([...nodes].reverse(), spaced);
    expect([...first]).toEqual([...second]);
    for (const [id, position] of spaced) {
      expect(first.get(id)).toEqual(position);
    }
  });
});

describe("seedCommunityOverviewPositions", () => {
  it("seeds an untouched group's bearing from its identity, not its rank", () => {
    const base: GraphNode[] = [
      { id: "h0-aaaaaaaaaaaaaaaa", label: "A", community: 0, memberCount: 40, size: 30 },
      { id: "h0-bbbbbbbbbbbbbbbb", label: "B", community: 1, memberCount: 20, size: 20 }
    ];
    const before = communitySeedPositions(base, base.map(() => ({
      id: "", halfWidth: 12, halfHeight: 12
    })));
    const grown: GraphNode[] = [
      ...base,
      { id: "h0-cccccccccccccccc", label: "C", community: 2, memberCount: 3_000, size: 60 }
    ];
    const after = communitySeedPositions(grown, grown.map(() => ({
      id: "", halfWidth: 12, halfHeight: 12
    })));
    const bearing = (position: { x: number; y: number }) => Math.atan2(position.y, position.x);
    for (const id of ["h0-aaaaaaaaaaaaaaaa", "h0-bbbbbbbbbbbbbbbb"]) {
      const earlier = before.get(id);
      const later = after.get(id);
      expect(earlier).toBeDefined();
      expect(later).toBeDefined();
      if (earlier && later && (earlier.x !== 0 || earlier.y !== 0)) {
        expect(bearing(later)).toBeCloseTo(bearing(earlier), 10);
        // The radius may grow when a larger group takes the centre.
        expect(Math.hypot(later.x, later.y)).toBeGreaterThanOrEqual(
          Math.hypot(earlier.x, earlier.y)
        );
      }
    }
  });

  it("derives bearings deterministically from identity alone", () => {
    expect(stableBearing("h0-0123456789abcdef"))
      .toBe(stableBearing("h0-0123456789abcdef"));
    expect(stableBearing("h0-0123456789abcdef"))
      .not.toBe(stableBearing("h0-fedcba9876543210"));
    for (const id of ["h0-1", "h0-abcdef0123456789", "level-2-group"]) {
      const bearing = stableBearing(id);
      expect(bearing).toBeGreaterThanOrEqual(0);
      expect(bearing).toBeLessThan(Math.PI * 2);
    }
  });

  it("keeps bubble labels clear of every neighbouring bubble", () => {
    const labels = [
      "test_basic.py",
      "setupmethod",
      "test_blueprints.py",
      "Flask (flask/app.py:L110)",
      "TestGenericHandlers",
      "__init__.py (flask/__init__.py:L1)"
    ];
    const nodes: GraphNode[] = Array.from({ length: 174 }, (_, index) => ({
      id: `community:${index}`,
      label: labels[index % labels.length]!,
      community: index,
      degree: index % 6,
      memberCount: Math.max(5, 375 - index * 2)
    })).map((node, _, all) => ({
      ...node,
      size: communityNodeSize(
        node.memberCount ?? 1,
        Math.max(...all.map((entry) => entry.memberCount ?? 1))
      )
    }));
    const positions = seedCommunityOverviewPositions(nodes);
    const labelled = communityOverviewLabelledIds(nodes);
    // Average glyph width of the rendered 13px label font.
    const glyphWidth = 7.2;

    expect([...positions]).toEqual([
      ...seedCommunityOverviewPositions([...nodes].reverse())
    ]);
    for (const node of nodes) {
      if (!labelled.has(node.id)) continue;
      const position = positions.get(node.id)!;
      const radius = node.size ?? 12;
      const halfText = communityOverviewLabelText(node.label).length * glyphWidth / 2;
      const label = {
        left: position.x - halfText,
        right: position.x + halfText,
        top: position.y + radius,
        bottom: position.y + radius + 28
      };
      for (const other of nodes) {
        if (other.id === node.id) continue;
        const otherPosition = positions.get(other.id)!;
        const otherRadius = other.size ?? 12;
        const horizontal = Math.max(
          label.left - otherPosition.x,
          0,
          otherPosition.x - label.right
        );
        const vertical = Math.max(
          label.top - otherPosition.y,
          0,
          otherPosition.y - label.bottom
        );
        expect(Math.hypot(horizontal, vertical) - otherRadius).toBeGreaterThan(0);
      }
    }
  });
});

describe("seedGraphLayoutPositions", () => {
  const nodes: GraphNode[] = [
    { id: "gamma", label: "Gamma", community: 2 },
    { id: "alpha", label: "Alpha", community: 1 },
    { id: "beta", label: "Beta", community: 1 }
  ];

  it.each(["circle", "concentric", "spiral", "grid", "hierarchical"] as const)(
    "produces a deterministic %s layout",
    (style) => {
      expect([...seedGraphLayoutPositions(nodes, style)]).toEqual([
        ...seedGraphLayoutPositions([...nodes].reverse(), style)
      ]);
    }
  );

  it("places lower hierarchical depths above higher depths", () => {
    const layered: GraphNode[] = [
      { id: "root", label: "Root", community: 0, depth: 0, root: true },
      { id: "one", label: "One", community: 0, depth: 1 },
      { id: "two", label: "Two", community: 0, depth: 2 }
    ];
    const positions = seedGraphLayoutPositions(layered, "hierarchical");
    expect(positions.get("root")!.y).toBeLessThan(positions.get("one")!.y);
    expect(positions.get("one")!.y).toBeLessThan(positions.get("two")!.y);
  });

  it("centers the highest-degree node and expands through concentric rings", () => {
    const hubs: GraphNode[] = Array.from({ length: 24 }, (_, index) => ({
      id: `hub-${index.toString().padStart(2, "0")}`,
      label: `Hub ${index}`,
      community: index % 3,
      degree: index
    }));
    const positions = seedGraphLayoutPositions(hubs, "concentric");
    expect(positions.get("hub-23")).toEqual({ x: 0, y: 0 });
    const radii = [...positions.values()].map(({ x, y }) => Math.hypot(x, y));
    expect(new Set(radii.map((radius) => Math.round(radius)))).toEqual(
      new Set([0, 72, 144])
    );
  });

  it("places spiral nodes at monotonically increasing radii", () => {
    const positions = seedGraphLayoutPositions(nodes, "spiral");
    const radii = [...positions.values()].map(({ x, y }) => Math.hypot(x, y));
    expect(radii).toEqual([...radii].sort((left, right) => left - right));
  });

  it("places grid nodes at distinct coordinates", () => {
    const positions = seedGraphLayoutPositions(model(5_200, 0).nodes, "grid");
    const coordinates = new Set(
      [...positions.values()].map(({ x, y }) => `${x},${y}`)
    );
    expect(positions.size).toBe(5_200);
    expect(coordinates.size).toBe(5_200);
  });

  it("aligns natural community blocks in outer rows and columns", () => {
    const groupedNodes: GraphNode[] = Array.from({ length: 16 }, (_, index) => ({
      id: `grouped-${index}`,
      label: `Grouped ${index}`,
      community: Math.floor(index / 4)
    }));
    const positions = seedGraphLayoutPositions(groupedNodes, "grid");
    const first = positions.get("grouped-0")!;
    const nextColumn = positions.get("grouped-4")!;
    const nextRow = positions.get("grouped-8")!;

    expect(first.y).toBe(nextColumn.y);
    expect(first.x).toBe(nextRow.x);
    expect(nextColumn.x - first.x).toBeGreaterThan(112);
    expect(nextRow.y - first.y).toBeGreaterThan(112);
  });

  it("batches aggregated communities into aligned four-by-four blocks", () => {
    const communities: GraphNode[] = Array.from({ length: 40 }, (_, index) => ({
      id: `community-${index}`,
      label: `Community ${index}`,
      community: index
    }));
    const positions = seedGraphLayoutPositions(communities, "grid", true);
    const blockCenter = (start: number, end: number) => {
      const block = Array.from({ length: end - start }, (_, offset) =>
        positions.get(`community-${start + offset}`)!);
      return {
        x: (Math.min(...block.map(({ x }) => x)) + Math.max(...block.map(({ x }) => x))) / 2,
        y: (Math.min(...block.map(({ y }) => y)) + Math.max(...block.map(({ y }) => y))) / 2
      };
    };
    const first = blockCenter(0, 16);
    const nextColumn = blockCenter(16, 32);
    const nextRow = blockCenter(32, 40);

    expect(first.y).toBe(nextColumn.y);
    expect(first.x).toBe(nextRow.x);
    expect(nextColumn.x - first.x).toBeGreaterThan(4 * 56);
    expect(nextRow.y - first.y).toBeGreaterThan(4 * 56);
  });
});

describe("visibleGraphEdges", () => {
  it("keeps a deterministic bounded backbone for dense community overviews", () => {
    const dense = model(100, 5_000);
    dense.stats.aggregated = true;
    dense.edges = dense.edges.map((edge, index) => ({
      ...edge,
      source: `n-${index % 100}`,
      target: `n-${(index * 17 + 1) % 100}`,
      weight: index + 1,
      relation: `${index + 1} cross-community edges`
    }));
    const selected = visibleGraphEdges(dense);
    expect(selected).toHaveLength(AGGREGATED_EDGE_RENDER_LIMIT);
    expect(selected[0]?.weight).toBe(5_000);
    expect(selected.map((edge) => edge.id)).toEqual(
      visibleGraphEdges({ ...dense, edges: [...dense.edges].reverse() })
        .map((edge) => edge.id)
    );
  });

  it("preserves every edge outside an aggregated overview", () => {
    const exact = model(100, 5_000);
    expect(visibleGraphEdges(exact)).toBe(exact.edges);
  });
});
