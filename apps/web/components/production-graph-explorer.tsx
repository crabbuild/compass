'use client';

import { useCallback, useMemo, useRef, useState } from 'react';
import {
  ArrowRightIcon,
  ExternalLinkIcon,
  FileCode2Icon,
  GaugeIcon,
  NetworkIcon,
  RotateCcwIcon,
  SearchIcon,
  TagIcon,
} from 'lucide-react';

import {
  EdgeHoverCard,
  NodeHoverCard,
  VisNetworkCanvas,
  graphNodeActivation,
} from '@compass/viewer';
import type {
  GraphCanvasHandle,
  GraphCanvasPosition,
} from '@compass/viewer';
import type {
  GraphEdgeHover,
  GraphHover,
  GraphNode,
  GraphViewModel,
} from '@compass/viewer';

const EMPTY_COMMUNITIES = new Set<number>();
const EMPTY_CHANGES = new Set<'added' | 'removed' | 'changed' | 'unchanged'>();

const communities = [
  { id: 0, label: 'Public API', color: '#5865F2', hidden: false },
  { id: 1, label: 'Vault + crypto', color: '#7B84FF', hidden: false },
  { id: 2, label: 'Runtime helpers', color: '#9AA1FF', hidden: false },
  { id: 3, label: 'CLI wiring', color: '#C3C8FF', hidden: false },
] satisfies GraphViewModel['communities'];

const dotenvNodes: GraphViewModel['nodes'] = [
  node('config-file', 'config.js', 'file', 3, 'config.js', 1, 10, 18),
  node('main-file', 'main.js', 'file', 0, 'lib/main.js', 1, 424, 34),
  node('env-file', 'env-options.js', 'file', 3, 'lib/env-options.js', 1, 29, 15),
  node('cli-file', 'cli-options.js', 'file', 3, 'lib/cli-options.js', 1, 18, 15),
  node('env-options', 'options', 'variable', 3, 'lib/env-options.js', 2, 2, 18),
  node('main-config', 'config()', 'function', 0, 'lib/main.js', 320, 336, 28, 'config(options)'),
  node('main-config-dotenv', 'configDotenv()', 'function', 0, 'lib/main.js', 240, 317, 28, 'configDotenv(options)'),
  node('main-config-vault', '_configVault()', 'function', 1, 'lib/main.js', 220, 238, 24, '_configVault(options)'),
  node('main-parse-vault', '_parseVault()', 'function', 1, 'lib/main.js', 79, 120, 25, '_parseVault(options)'),
  node('main-vault-path', '_vaultPath()', 'function', 1, 'lib/main.js', 192, 214, 22, '_vaultPath(options)'),
  node('main-dotenv-key', '_dotenvKey()', 'function', 1, 'lib/main.js', 134, 147, 20, '_dotenvKey(options)'),
  node('main-instructions', '_instructions()', 'function', 1, 'lib/main.js', 149, 190, 21, '_instructions(result, dotenvKey)'),
  node('main-parse', 'parse()', 'function', 0, 'lib/main.js', 41, 77, 24, 'parse(src)'),
  node('main-populate', 'populate()', 'function', 0, 'lib/main.js', 370, 403, 24, 'populate(processEnv, parsed, options)'),
  node('main-decrypt', 'decrypt()', 'function', 1, 'lib/main.js', 338, 367, 22, 'decrypt(encrypted, keyStr)'),
  node('main-resolve-home', '_resolveHome()', 'function', 2, 'lib/main.js', 216, 218, 18, '_resolveHome(envPath)'),
  node('main-parse-boolean', 'parseBoolean()', 'function', 2, 'lib/main.js', 23, 28, 20, 'parseBoolean(value)'),
  node('main-dim', 'dim()', 'function', 2, 'lib/main.js', 34, 36, 17, 'dim(text)'),
  node('main-debug', '_debug()', 'function', 2, 'lib/main.js', 126, 128, 17, '_debug(message)'),
  node('main-log', '_log()', 'function', 2, 'lib/main.js', 130, 132, 17, '_log(message)'),
  node('main-module', 'DotenvModule', 'object', 0, 'lib/main.js', 405, 415, 23),
  node('dep-fs', 'fs', 'import', 2, 'lib/main.js', 1, 1, 16, "require('fs')"),
  node('dep-path', 'path', 'import', 2, 'lib/main.js', 2, 2, 16, "require('path')"),
  node('dep-os', 'os', 'import', 2, 'lib/main.js', 3, 3, 16, "require('os')"),
  node('dep-crypto', 'crypto', 'import', 1, 'lib/main.js', 4, 4, 18, "require('crypto')"),
];

const dotenvEdges: GraphViewModel['edges'] = [
  edge('config-imports-main', 'config-file', 'main-file', 'IMPORTS', 'config.js', 2),
  edge('config-imports-env', 'config-file', 'env-file', 'IMPORTS', 'config.js', 4),
  edge('config-imports-cli', 'config-file', 'cli-file', 'IMPORTS', 'config.js', 5),
  edge('config-calls-api', 'config-file', 'main-config', 'CALLS', 'config.js', 2),
  edge('env-contains-options', 'env-file', 'env-options', 'CONTAINS', 'lib/env-options.js', 2),
  edge('env-supports-config', 'env-options', 'main-config', 'OPTIONS_FOR', 'config.js', 3),
  edge('cli-supports-config', 'cli-file', 'main-config', 'OPTIONS_FOR', 'config.js', 5),
  edge('main-contains-config', 'main-file', 'main-config', 'CONTAINS', 'lib/main.js', 320),
  edge('main-contains-config-dotenv', 'main-file', 'main-config-dotenv', 'CONTAINS', 'lib/main.js', 240),
  edge('main-contains-config-vault', 'main-file', 'main-config-vault', 'CONTAINS', 'lib/main.js', 220),
  edge('main-contains-parse-vault', 'main-file', 'main-parse-vault', 'CONTAINS', 'lib/main.js', 79),
  edge('main-contains-vault-path', 'main-file', 'main-vault-path', 'CONTAINS', 'lib/main.js', 192),
  edge('main-contains-dotenv-key', 'main-file', 'main-dotenv-key', 'CONTAINS', 'lib/main.js', 134),
  edge('main-contains-instructions', 'main-file', 'main-instructions', 'CONTAINS', 'lib/main.js', 149),
  edge('main-contains-parse', 'main-file', 'main-parse', 'CONTAINS', 'lib/main.js', 41),
  edge('main-contains-populate', 'main-file', 'main-populate', 'CONTAINS', 'lib/main.js', 370),
  edge('main-contains-decrypt', 'main-file', 'main-decrypt', 'CONTAINS', 'lib/main.js', 338),
  edge('main-contains-resolve-home', 'main-file', 'main-resolve-home', 'CONTAINS', 'lib/main.js', 216),
  edge('main-contains-parse-boolean', 'main-file', 'main-parse-boolean', 'CONTAINS', 'lib/main.js', 23),
  edge('main-contains-dim', 'main-file', 'main-dim', 'CONTAINS', 'lib/main.js', 34),
  edge('main-contains-debug', 'main-file', 'main-debug', 'CONTAINS', 'lib/main.js', 126),
  edge('main-contains-log', 'main-file', 'main-log', 'CONTAINS', 'lib/main.js', 130),
  edge('main-contains-module', 'main-file', 'main-module', 'CONTAINS', 'lib/main.js', 405),
  edge('main-imports-fs', 'main-file', 'dep-fs', 'IMPORTS', 'lib/main.js', 1),
  edge('main-imports-path', 'main-file', 'dep-path', 'IMPORTS', 'lib/main.js', 2),
  edge('main-imports-os', 'main-file', 'dep-os', 'IMPORTS', 'lib/main.js', 3),
  edge('main-imports-crypto', 'main-file', 'dep-crypto', 'IMPORTS', 'lib/main.js', 4),
  edge('config-calls-dotenv-key', 'main-config', 'main-dotenv-key', 'CALLS', 'lib/main.js', 322),
  edge('config-calls-vault-path', 'main-config', 'main-vault-path', 'CALLS', 'lib/main.js', 326),
  edge('config-calls-config-dotenv', 'main-config', 'main-config-dotenv', 'CALLS', 'lib/main.js', 330),
  edge('config-calls-config-vault', 'main-config', 'main-config-vault', 'CALLS', 'lib/main.js', 335),
  edge('config-dotenv-calls-boolean', 'main-config-dotenv', 'main-parse-boolean', 'CALLS', 'lib/main.js', 247),
  edge('config-dotenv-calls-home', 'main-config-dotenv', 'main-resolve-home', 'CALLS', 'lib/main.js', 261),
  edge('config-dotenv-calls-parse', 'main-config-dotenv', 'main-parse', 'CALLS', 'lib/main.js', 277),
  edge('config-dotenv-calls-populate', 'main-config-dotenv', 'main-populate', 'CALLS', 'lib/main.js', 279),
  edge('config-dotenv-calls-debug', 'main-config-dotenv', 'main-debug', 'CALLS', 'lib/main.js', 254),
  edge('config-dotenv-calls-log', 'main-config-dotenv', 'main-log', 'CALLS', 'lib/main.js', 309),
  edge('config-dotenv-calls-dim', 'main-config-dotenv', 'main-dim', 'CALLS', 'lib/main.js', 309),
  edge('config-dotenv-reads-fs', 'main-config-dotenv', 'dep-fs', 'READS', 'lib/main.js', 277),
  edge('config-dotenv-uses-path', 'main-config-dotenv', 'dep-path', 'USES', 'lib/main.js', 241),
  edge('config-vault-calls-boolean', 'main-config-vault', 'main-parse-boolean', 'CALLS', 'lib/main.js', 221),
  edge('config-vault-calls-log', 'main-config-vault', 'main-log', 'CALLS', 'lib/main.js', 225),
  edge('config-vault-calls-parse-vault', 'main-config-vault', 'main-parse-vault', 'CALLS', 'lib/main.js', 228),
  edge('config-vault-calls-populate', 'main-config-vault', 'main-populate', 'CALLS', 'lib/main.js', 235),
  edge('parse-vault-calls-vault-path', 'main-parse-vault', 'main-vault-path', 'CALLS', 'lib/main.js', 82),
  edge('parse-vault-calls-dotenv-key', 'main-parse-vault', 'main-dotenv-key', 'CALLS', 'lib/main.js', 93),
  edge('parse-vault-calls-instructions', 'main-parse-vault', 'main-instructions', 'CALLS', 'lib/main.js', 103),
  edge('parse-vault-calls-decrypt', 'main-parse-vault', 'main-decrypt', 'CALLS', 'lib/main.js', 106),
  edge('parse-vault-calls-parse', 'main-parse-vault', 'main-parse', 'CALLS', 'lib/main.js', 119),
  edge('parse-vault-calls-populate', 'main-parse-vault', 'main-populate', 'CALLS', 'lib/main.js', 113),
  edge('decrypt-uses-crypto', 'main-decrypt', 'dep-crypto', 'USES', 'lib/main.js', 347),
  edge('populate-calls-debug', 'main-populate', 'main-debug', 'CALLS', 'lib/main.js', 391),
];

const degreeById = new Map<string, number>();
for (const connection of dotenvEdges) {
  degreeById.set(connection.source, (degreeById.get(connection.source) ?? 0) + 1);
  degreeById.set(connection.target, (degreeById.get(connection.target) ?? 0) + 1);
}

const dotenvGraphModel: GraphViewModel = {
  schema: 'compass.viewer.graph/1',
  title: 'dotenv / source graph',
  stats: {
    nodes: dotenvNodes.length,
    edges: dotenvEdges.length,
    communities: communities.length,
    aggregated: false,
  },
  communities,
  nodes: dotenvNodes.map((entry) => ({
    ...entry,
    communityName: communities.find((community) => community.id === entry.community)?.label,
    degree: degreeById.get(entry.id) ?? 0,
  })),
  edges: dotenvEdges,
  hyperedges: [],
};

const initialPositions: ReadonlyMap<string, GraphCanvasPosition> = new Map([
  ['config-file', { x: -520, y: -130 }],
  ['env-file', { x: -520, y: 120 }],
  ['cli-file', { x: -520, y: 360 }],
  ['env-options', { x: -340, y: 120 }],
  ['main-file', { x: -120, y: 30 }],
  ['main-config', { x: 100, y: -250 }],
  ['main-config-dotenv', { x: 140, y: -70 }],
  ['main-config-vault', { x: 155, y: 120 }],
  ['main-parse-vault', { x: 365, y: 100 }],
  ['main-vault-path', { x: 560, y: 20 }],
  ['main-dotenv-key', { x: 560, y: 170 }],
  ['main-instructions', { x: 730, y: 260 }],
  ['main-decrypt', { x: 730, y: 420 }],
  ['main-parse', { x: 390, y: -190 }],
  ['main-populate', { x: 390, y: -360 }],
  ['main-resolve-home', { x: 370, y: -500 }],
  ['main-parse-boolean', { x: -50, y: -450 }],
  ['main-dim', { x: 110, y: -520 }],
  ['main-debug', { x: -180, y: -440 }],
  ['main-log', { x: -310, y: -450 }],
  ['main-module', { x: -130, y: 280 }],
  ['dep-fs', { x: -310, y: -210 }],
  ['dep-path', { x: -180, y: -250 }],
  ['dep-os', { x: -320, y: -310 }],
  ['dep-crypto', { x: 560, y: 430 }],
]);

export function ProductionGraphExplorer() {
  const [focusedNodeId, setFocusedNodeId] = useState<string | null>('main-config');
  const [hover, setHover] = useState<GraphHover | null>(null);
  const [edgeHover, setEdgeHover] = useState<GraphEdgeHover | null>(null);
  const [query, setQuery] = useState('');
  const [forceLabels, setForceLabels] = useState(true);
  const [hiddenCommunities, setHiddenCommunities] = useState<ReadonlySet<number>>(EMPTY_COMMUNITIES);
  const canvasRef = useRef<GraphCanvasHandle>(null);

  const nodeById = useMemo(
    () => new Map(dotenvGraphModel.nodes.map((entry) => [entry.id, entry])),
    [],
  );
  const edgeById = useMemo(
    () => new Map(dotenvGraphModel.edges.map((entry) => [entry.id, entry])),
    [],
  );
  const selected = focusedNodeId ? nodeById.get(focusedNodeId) : undefined;
  const hoveredNode = hover ? nodeById.get(hover.nodeId) : undefined;
  const hoveredActivation = hoveredNode ? graphNodeActivation(dotenvGraphModel, hoveredNode) : undefined;
  const hoveredEdge = edgeHover ? edgeById.get(edgeHover.edgeId) : undefined;
  const hoveredEdgeSource = hoveredEdge ? nodeById.get(hoveredEdge.source) : undefined;
  const hoveredEdgeTarget = hoveredEdge ? nodeById.get(hoveredEdge.target) : undefined;
  const connectedEdges = selected
    ? dotenvGraphModel.edges.filter((entry) => entry.source === selected.id || entry.target === selected.id)
    : [];
  const matches = useMemo(() => {
    const normalizedQuery = query.trim().toLocaleLowerCase();
    if (!normalizedQuery) return [];
    return dotenvGraphModel.nodes
      .filter((entry) => entry.label.toLocaleLowerCase().includes(normalizedQuery)
        || entry.source?.file.toLocaleLowerCase().includes(normalizedQuery)
        || entry.kind?.toLocaleLowerCase().includes(normalizedQuery))
      .slice(0, 8);
  }, [query]);

  const focus = useCallback((nodeId: string) => {
    setFocusedNodeId(nodeId);
    setHover(null);
    setEdgeHover(null);
  }, []);
  const clear = useCallback(() => {
    setFocusedNodeId(null);
    setHover(null);
    setEdgeHover(null);
  }, []);
  const toggleCommunity = useCallback((communityId: number) => {
    setHiddenCommunities((current) => {
      const next = new Set(current);
      if (next.has(communityId)) next.delete(communityId);
      else next.add(communityId);
      return next;
    });
  }, []);
  const openSource = useCallback((source: GraphNode['source']) => {
    if (!source?.file) return;
    const line = source.startLine ? `#L${source.startLine}` : '';
    window.open(`https://github.com/motdotla/dotenv/blob/v17.4.2/${source.file}${line}`, '_blank', 'noopener,noreferrer');
  }, []);

  const activateNode = useCallback((nodeId: string) => {
    const node = nodeById.get(nodeId);
    if (!node) return;
    focus(nodeId);
    openSource(node.source);
  }, [focus, nodeById, openSource]);
  const activateEdge = useCallback((edgeId: string) => {
    const edge = edgeById.get(edgeId);
    if (!edge) return;
    setHover(null);
    setEdgeHover(null);
    openSource(edge.relationshipSite);
  }, [edgeById, openSource]);

  return (
    <div className="code-graph-explorer hero-code-graph">
      <div className="code-graph-explorer-stage">
        <VisNetworkCanvas
          ref={canvasRef}
          model={dotenvGraphModel}
          focusedNodeId={focusedNodeId}
          initialPositions={initialPositions}
          physicsRunning={false}
          forceLabels={forceLabels}
          hiddenCommunities={hiddenCommunities}
          hiddenChanges={EMPTY_CHANGES}
          onFocus={focus}
          onOpenSource={activateNode}
          onOpenRelationshipSource={activateEdge}
          onHover={setHover}
          onHoverEdge={setEdgeHover}
          onClear={clear}
          onStabilized={() => undefined}
        />

        <div className="code-graph-explorer-toolbar" role="toolbar" aria-label="Graph controls">
          <span className="code-graph-explorer-status">
            <i aria-hidden="true" />
            {selected ? `Inspecting ${selected.label}` : 'Fixed layout · drag to explore'}
          </span>
          <div className="code-graph-explorer-actions">
            <button type="button" aria-pressed={forceLabels} onClick={() => setForceLabels((visible) => !visible)}>
              <TagIcon aria-hidden="true" /> Labels
            </button>
            <button type="button" onClick={() => canvasRef.current?.fit()}>
              <GaugeIcon aria-hidden="true" /> Fit
            </button>
            <button type="button" onClick={() => { clear(); canvasRef.current?.reset(); }}>
              <RotateCcwIcon aria-hidden="true" /> Reset
            </button>
          </div>
        </div>

        {hover && hoveredNode && hoveredActivation && (
          <NodeHoverCard node={hoveredNode} hover={hover} activation={hoveredActivation} />
        )}
        {edgeHover && hoveredEdge && hoveredEdgeSource && hoveredEdgeTarget && (
          <EdgeHoverCard
            edge={hoveredEdge}
            sourceNode={hoveredEdgeSource}
            targetNode={hoveredEdgeTarget}
            hover={edgeHover}
          />
        )}

        <footer className="code-graph-explorer-footer">
          <span><i aria-hidden="true" /> dotenv@17.4.2 · source graph</span>
          <span>{dotenvGraphModel.stats.nodes} nodes · {dotenvGraphModel.stats.edges} edges</span>
        </footer>
      </div>

      <aside className="code-graph-explorer-inspector" aria-label="Graph inspector">
        <header className="code-graph-inspector-header">
          <span className="code-graph-inspector-mark"><NetworkIcon aria-hidden="true" /></span>
          <div>
            <strong>Compass</strong>
            <span>dotenv / source graph</span>
          </div>
        </header>

        <div className="code-graph-search" role="search">
          <SearchIcon aria-hidden="true" />
          <input
            type="search"
            aria-label="Search dotenv graph"
            placeholder="Search nodes and files"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
          />
          {matches.length > 0 && (
            <div className="code-graph-search-results" role="listbox" aria-label="Matching graph nodes">
              {matches.map((entry) => (
                <button
                  key={entry.id}
                  type="button"
                  role="option"
                  aria-selected={entry.id === focusedNodeId}
                  onClick={() => { focus(entry.id); setQuery(''); }}
                >
                  <strong>{entry.label}</strong>
                  <span>{entry.source?.file}</span>
                </button>
              ))}
            </div>
          )}
        </div>

        <section className="code-graph-inspector-section" aria-labelledby="code-graph-node-heading">
          <div className="code-graph-section-label">
            <h2 id="code-graph-node-heading">Inspector</h2>
            <span>{selected ? 'Pinned' : 'Node details'}</span>
          </div>
          {selected ? (
            <div className="code-graph-node-details">
              <div className="code-graph-node-title">
                <span style={{ background: communities.find((community) => community.id === selected.community)?.color }} />
                <div>
                  <strong>{selected.label}</strong>
                  <span>{(selected.kind ?? 'symbol').toUpperCase()}</span>
                </div>
              </div>
              <div className="code-graph-node-stats">
                <span><small>LANGUAGE</small><strong>{selected.language ?? 'JavaScript'}</strong></span>
                <span><small>DEGREE</small><strong>{selected.degree ?? 0}</strong></span>
                <span><small>COMMUNITY</small><strong>{selected.communityName ?? 'Source graph'}</strong></span>
                <span><small>LINES</small><strong>{lineRange(selected)}</strong></span>
              </div>
              <button className="code-graph-source-link" type="button" onClick={() => openSource(selected.source)}>
                <FileCode2Icon aria-hidden="true" />
                <span><strong>{selected.source?.file}</strong><small>{lineRange(selected)}</small></span>
                <ExternalLinkIcon aria-hidden="true" />
              </button>
              <p className="code-graph-inspector-hint"><strong>Double-click</strong> a node or edge to open its GitHub source.</p>
            </div>
          ) : (
            <p className="code-graph-empty-state">Select a symbol to keep its source location and relationships in view.</p>
          )}
        </section>

        <section className="code-graph-inspector-section" aria-labelledby="code-graph-relationships-heading">
          <div className="code-graph-section-label">
            <h2 id="code-graph-relationships-heading">Relationships</h2>
            <span>{connectedEdges.length}</span>
          </div>
          {selected && connectedEdges.length > 0 ? (
            <div className="code-graph-relationship-list">
              {connectedEdges.slice(0, 7).map((connection) => {
                const targetId = connection.source === selected.id ? connection.target : connection.source;
                const target = nodeById.get(targetId);
                if (!target) return null;
                return (
                  <button key={connection.id} type="button" onClick={() => focus(target.id)}>
                    <span className="code-graph-relationship-type">{connection.relation}</span>
                    <span className="code-graph-relationship-target">{target.label}</span>
                    <ArrowRightIcon aria-hidden="true" />
                  </button>
                );
              })}
            </div>
          ) : (
            <p className="code-graph-empty-state">No connected relationships are pinned.</p>
          )}
        </section>

        <section className="code-graph-inspector-section code-graph-community-section" aria-labelledby="code-graph-communities-heading">
          <div className="code-graph-section-label">
            <h2 id="code-graph-communities-heading">Communities</h2>
            <span>{communities.length}</span>
          </div>
          <div className="code-graph-community-list">
            {communities.map((community) => {
              const visible = !hiddenCommunities.has(community.id);
              const count = dotenvGraphModel.nodes.filter((entry) => entry.community === community.id).length;
              return (
                <button
                  key={community.id}
                  type="button"
                  aria-pressed={visible}
                  data-visible={visible}
                  onClick={() => toggleCommunity(community.id)}
                >
                  <span className="code-graph-community-dot" style={{ background: community.color }} />
                  <span>{community.label}</span>
                  <small>{count}</small>
                </button>
              );
            })}
          </div>
        </section>
      </aside>
    </div>
  );
}

function node(
  id: string,
  label: string,
  kind: string,
  community: number,
  file: string,
  startLine: number,
  endLine: number,
  size: number,
  signature?: string,
): GraphViewModel['nodes'][number] {
  return {
    id,
    label,
    kind,
    community,
    language: 'JavaScript',
    signature,
    size,
    source: { file, startLine, endLine },
  };
}

function edge(
  id: string,
  source: string,
  target: string,
  relation: string,
  file: string,
  startLine: number,
): GraphViewModel['edges'][number] {
  return {
    id,
    source,
    target,
    relation,
    confidence: 'extracted',
    relationshipSite: { file, startLine },
  };
}

function lineRange(node: GraphNode): string {
  const start = node.source?.startLine;
  const end = node.source?.endLine;
  if (start === undefined) return '—';
  return end !== undefined && end !== start ? `${start}–${end}` : String(start);
}
