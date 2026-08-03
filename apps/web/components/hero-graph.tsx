'use client';

import { useCallback, useMemo, useState } from 'react';
import {
  EdgeHoverCard,
  NodeHoverCard,
  VisNetworkCanvas,
  graphNodeActivation,
} from '@compass/viewer';
import type { GraphCanvasPosition, GraphEdgeHover, GraphHover, GraphViewModel } from '@compass/viewer';

const EMPTY_COMMUNITIES = new Set<number>();
const EMPTY_CHANGES = new Set<'added' | 'removed' | 'changed' | 'unchanged'>();
const HERO_POSITIONS: ReadonlyMap<string, GraphCanvasPosition> = new Map([
  ['compass', { x: -220, y: 0 }],
  ['build', { x: -120, y: 110 }],
  ['watch', { x: -120, y: -110 }],
  ['discover', { x: 0, y: 0 }],
  ['extract', { x: 60, y: 160 }],
  ['resolve', { x: 60, y: -160 }],
  ['publish', { x: 150, y: 0 }],
  ['graph', { x: 240, y: 0 }],
  ['query', { x: 330, y: 0 }],
  ['history', { x: 280, y: 230 }],
  ['impact', { x: 400, y: 140 }],
  ['output', { x: 400, y: -140 }],
]);

// This is intentionally a small, representative snapshot rather than a
// decorative SVG. It uses the same GraphViewModel contract and vis-network
// renderer that powers the VS Code graph view.
const heroModel: GraphViewModel = {
  schema: 'compass.viewer.graph/1',
  title: 'Compass architecture overview',
  stats: {
    nodes: 12,
    edges: 12,
    communities: 3,
    aggregated: false,
  },
  communities: [
    { id: 0, label: 'CLI', color: '#5865F2', hidden: false },
    { id: 1, label: 'Graph pipeline', color: '#7B84FF', hidden: false },
    { id: 2, label: 'Query + output', color: '#B8BEFF', hidden: false },
  ],
  nodes: [
    node('compass', 'compass', 'binary', 0, 4, 'crates/compass-cli/src/bin/compass.rs', 22),
    node('build', 'build', 'command', 0, 3, 'crates/compass-core/src/build.rs', 17),
    node('watch', 'watch', 'command', 0, 2, 'crates/compass-core/src/watch.rs', 14),
    node('discover', 'discover', 'stage', 1, 3, 'crates/compass-files/src/discovery.rs', 17),
    node('extract', 'extract', 'stage', 1, 3, 'crates/compass-languages/src/lib.rs', 17),
    node('resolve', 'resolve', 'stage', 1, 4, 'crates/compass-resolve/src/lib.rs', 21),
    node('publish', 'publish', 'stage', 1, 3, 'crates/compass-graph/src/publish.rs', 17),
    node('graph', 'Graph', 'model', 1, 4, 'crates/compass-model/src/graph.rs', 21),
    node('query', 'CompassQL', 'query', 2, 3, 'crates/compass-query/src/lib.rs', 18),
    node('impact', 'impact', 'query', 2, 2, 'crates/compass-query/src/impact.rs', 14),
    node('history', 'history', 'store', 2, 2, 'crates/compass-history/src/lib.rs', 14),
    node('output', 'output', 'renderer', 2, 3, 'crates/compass-output/src/lib.rs', 17),
  ],
  edges: [
    edge('compass-build', 'compass', 'build', 'CALLS'),
    edge('compass-watch', 'compass', 'watch', 'CALLS'),
    edge('build-discover', 'build', 'discover', 'BUILDS'),
    edge('watch-discover', 'watch', 'discover', 'WATCHES'),
    edge('discover-extract', 'discover', 'extract', 'EMITS'),
    edge('extract-resolve', 'extract', 'resolve', 'RESOLVES'),
    edge('resolve-publish', 'resolve', 'publish', 'PUBLISHES'),
    edge('publish-graph', 'publish', 'graph', 'WRITES'),
    edge('graph-query', 'graph', 'query', 'SERVES'),
    edge('query-impact', 'query', 'impact', 'TRAVERSES'),
    edge('graph-history', 'graph', 'history', 'STORES'),
    edge('query-output', 'query', 'output', 'RETURNS'),
  ],
  hyperedges: [],
};

export function HeroGraph() {
  const [focusedNodeId, setFocusedNodeId] = useState<string | null>(null);
  const [hover, setHover] = useState<GraphHover | null>(null);
  const [edgeHover, setEdgeHover] = useState<GraphEdgeHover | null>(null);
  const handleClear = useCallback(() => {
    setFocusedNodeId(null);
    setHover(null);
    setEdgeHover(null);
  }, []);
  const handleNoop = useCallback(() => undefined, []);
  const nodeById = useMemo(() => new Map(heroModel.nodes.map((node) => [node.id, node])), []);
  const edgeById = useMemo(() => new Map(heroModel.edges.map((edge) => [edge.id, edge])), []);
  const focusedNode = useMemo(
    () => heroModel.nodes.find((node) => node.id === focusedNodeId),
    [focusedNodeId],
  );
  const hoveredNode = hover ? nodeById.get(hover.nodeId) : undefined;
  const hoveredActivation = hoveredNode ? graphNodeActivation(heroModel, hoveredNode) : undefined;
  const hoveredEdge = edgeHover ? edgeById.get(edgeHover.edgeId) : undefined;
  const hoveredEdgeSource = hoveredEdge ? nodeById.get(hoveredEdge.source) : undefined;
  const hoveredEdgeTarget = hoveredEdge ? nodeById.get(hoveredEdge.target) : undefined;

  return (
    <div className="hero-code-graph" aria-label="Compass code graph preview">
      <div className="hero-code-graph-chrome" aria-hidden="true">
        <span>graph / overview</span>
        <span className="hero-code-graph-local"><i /> local snapshot</span>
      </div>
      <div className="hero-code-graph-canvas">
        <VisNetworkCanvas
          model={heroModel}
          focusedNodeId={focusedNodeId}
          initialPositions={HERO_POSITIONS}
          physicsRunning={false}
          forceLabels
          hiddenCommunities={EMPTY_COMMUNITIES}
          hiddenChanges={EMPTY_CHANGES}
          onFocus={setFocusedNodeId}
          onOpenSource={handleNoop}
          onOpenRelationshipSource={handleNoop}
          onHover={setHover}
          onHoverEdge={setEdgeHover}
          onClear={handleClear}
          onStabilized={handleNoop}
        />
      </div>
      {hover && hoveredNode && hoveredActivation && (
        <NodeHoverCard node={hoveredNode} hover={hover} activation={hoveredActivation} />
      )}
      {edgeHover && hoveredEdge && hoveredEdgeSource && hoveredEdgeTarget && (
        <EdgeHoverCard edge={hoveredEdge} sourceNode={hoveredEdgeSource} targetNode={hoveredEdgeTarget} hover={edgeHover} />
      )}
      <div className="hero-code-graph-footer">
        <span className="hero-code-graph-status">
          <i aria-hidden="true" />
          {hoveredNode ? `hover / ${hoveredNode.label}` : focusedNode ? `focused / ${focusedNode.label}` : 'hover a node or edge for source context'}
        </span>
        <span>{heroModel.stats.nodes} nodes · {heroModel.stats.edges} edges</span>
      </div>
    </div>
  );
}

function node(
  id: string,
  label: string,
  kind: string,
  community: number,
  degree: number,
  file: string,
  size: number,
): GraphViewModel['nodes'][number] {
  return {
    id,
    label,
    kind,
    community,
    degree,
    size,
    language: 'Rust',
    source: { file, startLine: 24 },
  };
}

function edge(
  id: string,
  source: string,
  target: string,
  relation: string,
): GraphViewModel['edges'][number] {
  return {
    id,
    source,
    target,
    relation,
    confidence: 'extracted',
    relationshipSite: {
      file: sourceFile(source),
      startLine: 28 + relation.length,
    },
  };
}

function sourceFile(nodeId: string): string {
  const files: Record<string, string> = {
    compass: 'crates/compass-cli/src/bin/compass.rs',
    build: 'crates/compass-core/src/build.rs',
    watch: 'crates/compass-core/src/watch.rs',
    discover: 'crates/compass-files/src/discovery.rs',
    extract: 'crates/compass-languages/src/lib.rs',
    resolve: 'crates/compass-resolve/src/lib.rs',
    publish: 'crates/compass-graph/src/publish.rs',
    graph: 'crates/compass-model/src/graph.rs',
    query: 'crates/compass-query/src/lib.rs',
  };
  return files[nodeId] ?? 'crates/compass/src/lib.rs';
}
