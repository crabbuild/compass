import type { ReactNode } from 'react';

function DiagramShell({
  eyebrow,
  title,
  children,
  className = '',
}: {
  eyebrow: string;
  title: string;
  children: ReactNode;
  className?: string;
}) {
  return (
    <div className={`overflow-hidden rounded-2xl border border-border/80 bg-card/70 shadow-sm ${className}`}>
      <div className="flex items-center justify-between gap-4 border-b border-border/70 px-5 py-4 sm:px-6">
        <div>
          <p className="eyebrow">{eyebrow}</p>
          <p className="mt-1 font-heading text-sm font-semibold tracking-[-0.02em]">{title}</p>
        </div>
        <span className="font-mono text-[0.62rem] uppercase tracking-[0.14em] text-muted-foreground">Compass / visual contract</span>
      </div>
      <div className="p-4 sm:p-6">{children}</div>
    </div>
  );
}

export function PipelineDiagram() {
  return (
    <DiagramShell eyebrow="Artifact lineage" title="Four boundaries, one publishable snapshot.">
      <svg
        aria-labelledby="pipeline-diagram-title pipeline-diagram-description"
        className="h-auto w-full"
        role="img"
        viewBox="0 0 1120 360"
      >
        <title id="pipeline-diagram-title">Compass source-driven pipeline</title>
        <desc id="pipeline-diagram-description">Compass files move from scoped discovery to per-file evidence, resolved relationships, and one validated graph JSON snapshot. Each boundary names its owner and artifact.</desc>
        <defs>
          <marker id="pipeline-arrow" markerHeight="8" markerWidth="8" orient="auto-start-reverse" refX="7" refY="4">
            <path d="M0 0L8 4L0 8Z" fill="var(--compass-blue)" />
          </marker>
        </defs>
        <path d="M254 150H294M524 150H564M794 150H834" fill="none" markerEnd="url(#pipeline-arrow)" stroke="var(--compass-blue)" strokeWidth="2.5" />
        <path d="M84 256V284H1036V256" fill="none" stroke="var(--border)" strokeDasharray="4 6" strokeWidth="1" />
        <g>
          <rect fill="var(--background)" height="154" rx="16" stroke="var(--border)" width="230" x="24" y="72" />
          <circle cx="58" cy="108" fill="var(--compass-indigo)" r="15" />
          <text fill="var(--primary-foreground)" fontFamily="var(--font-plex-mono)" fontSize="10" textAnchor="middle" x="58" y="112">01</text>
          <text fill="var(--foreground)" fontFamily="var(--font-space-grotesk)" fontSize="20" fontWeight="600" x="88" y="114">Discover</text>
          <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="11" x="44" y="151">scope + classify files</text>
          <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="10" x="44" y="171">owner · compass-files</text>
          <rect fill="var(--compass-canvas-deep)" height="27" rx="8" stroke="var(--border)" width="142" x="44" y="184" />
          <text fill="var(--compass-blue)" fontFamily="var(--font-plex-mono)" fontSize="10" x="57" y="201">manifest.json</text>
        </g>
        <g>
          <rect fill="var(--background)" height="154" rx="16" stroke="var(--border)" width="230" x="294" y="72" />
          <circle cx="328" cy="108" fill="var(--compass-blue)" r="15" />
          <text fill="var(--primary-foreground)" fontFamily="var(--font-plex-mono)" fontSize="10" textAnchor="middle" x="328" y="112">02</text>
          <text fill="var(--foreground)" fontFamily="var(--font-space-grotesk)" fontSize="20" fontWeight="600" x="358" y="114">Extract</text>
          <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="11" x="314" y="151">syntax facts + anchors</text>
          <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="10" x="314" y="171">owner · compass-languages</text>
          <rect fill="var(--compass-canvas-deep)" height="27" rx="8" stroke="var(--border)" width="142" x="314" y="184" />
          <text fill="var(--compass-blue)" fontFamily="var(--font-plex-mono)" fontSize="10" x="327" y="201">per-file facts</text>
        </g>
        <g>
          <rect fill="var(--background)" height="154" rx="16" stroke="var(--border)" width="230" x="564" y="72" />
          <circle cx="598" cy="108" fill="var(--compass-blue)" r="15" />
          <text fill="var(--primary-foreground)" fontFamily="var(--font-plex-mono)" fontSize="10" textAnchor="middle" x="598" y="112">03</text>
          <text fill="var(--foreground)" fontFamily="var(--font-space-grotesk)" fontSize="20" fontWeight="600" x="628" y="114">Resolve</text>
          <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="11" x="584" y="151">connect cross-file links</text>
          <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="10" x="584" y="171">owner · compass-resolve</text>
          <rect fill="var(--compass-canvas-deep)" height="27" rx="8" stroke="var(--border)" width="142" x="584" y="184" />
          <text fill="var(--compass-blue)" fontFamily="var(--font-plex-mono)" fontSize="10" x="597" y="201">directed edges</text>
        </g>
        <g>
          <rect fill="var(--background)" height="154" rx="16" stroke="var(--border)" width="230" x="834" y="72" />
          <circle cx="868" cy="108" fill="var(--compass-indigo)" r="15" />
          <text fill="var(--primary-foreground)" fontFamily="var(--font-plex-mono)" fontSize="10" textAnchor="middle" x="868" y="112">04</text>
          <text fill="var(--foreground)" fontFamily="var(--font-space-grotesk)" fontSize="20" fontWeight="600" x="898" y="114">Publish</text>
          <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="11" x="854" y="151">validate + materialize</text>
          <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="10" x="854" y="171">owner · compass-graph</text>
          <rect fill="var(--compass-canvas-deep)" height="27" rx="8" stroke="var(--border)" width="142" x="854" y="184" />
          <text fill="var(--compass-blue)" fontFamily="var(--font-plex-mono)" fontSize="10" x="867" y="201">graph.json</text>
        </g>
        <g fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="11" textAnchor="middle">
          <text x="84" y="313">repository scope</text>
          <text x="354" y="313">source evidence</text>
          <text x="624" y="313">relationship set</text>
          <text x="994" y="313">atomic snapshot</text>
        </g>
      </svg>
    </DiagramShell>
  );
}

export function EvidenceDiagram() {
  return (
    <DiagramShell eyebrow="Edge anatomy" title="A relationship is useful when it can answer follow-up questions.">
      <svg
        aria-labelledby="evidence-diagram-title evidence-diagram-description"
        className="h-auto w-full"
        role="img"
        viewBox="0 0 760 300"
      >
        <title id="evidence-diagram-title">A Compass relationship with evidence</title>
        <desc id="evidence-diagram-description">A directed calls edge connects two symbols and retains its relation, confidence, source range, and provenance.</desc>
        <defs>
          <marker id="evidence-arrow" markerHeight="8" markerWidth="8" orient="auto" refX="7" refY="4">
            <path d="M0 0L8 4L0 8Z" fill="var(--compass-blue)" />
          </marker>
        </defs>
        <path d="M191 126H568" fill="none" markerEnd="url(#evidence-arrow)" stroke="var(--compass-blue)" strokeWidth="3" />
        <circle cx="160" cy="126" fill="var(--compass-indigo)" r="31" />
        <circle cx="600" cy="126" fill="var(--compass-blue)" r="31" />
        <text fill="var(--primary-foreground)" fontFamily="var(--font-space-grotesk)" fontSize="13" fontWeight="600" textAnchor="middle" x="160" y="130">caller</text>
        <text fill="var(--primary-foreground)" fontFamily="var(--font-space-grotesk)" fontSize="13" fontWeight="600" textAnchor="middle" x="600" y="130">target</text>
        <g fill="var(--foreground)" fontFamily="var(--font-plex-mono)" fontSize="11" textAnchor="middle">
          <rect fill="var(--compass-canvas-deep)" height="30" rx="15" stroke="var(--border)" width="110" x="325" y="109" />
          <text x="380" y="128">CALLS · direct</text>
        </g>
        <g fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="11">
          <text x="45" y="49">source anchor</text>
          <text x="45" y="69">src/checkout.rs:42</text>
          <path d="M104 79L139 102" fill="none" stroke="var(--border)" />
          <text x="492" y="220">provenance</text>
          <text x="492" y="240">extracted · parser evidence</text>
          <path d="M548 202L591 157" fill="none" stroke="var(--border)" />
          <text x="270" y="210">relationship site</text>
          <text x="270" y="230">line 42 → line 88</text>
        </g>
      </svg>
    </DiagramShell>
  );
}

export function ImpactPathDiagram() {
  return (
    <DiagramShell eyebrow="Bounded traversal" title="Start at the changed symbol; keep the path visible.">
      <svg
        aria-labelledby="impact-diagram-title impact-diagram-description"
        className="h-auto w-full"
        role="img"
        viewBox="0 0 1040 320"
      >
        <title id="impact-diagram-title">Compass impact path</title>
        <desc id="impact-diagram-description">A changed payment function is traversed through three directly connected symbols. Each node keeps its source path and the traversal stays within explicit depth and result limits.</desc>
        <defs>
          <marker id="impact-arrow" markerHeight="8" markerWidth="8" orient="auto" refX="7" refY="4">
            <path d="M0 0L8 4L0 8Z" fill="var(--compass-blue)" />
          </marker>
        </defs>
        <path d="M200 164H280M460 164H540M720 164H800" fill="none" markerEnd="url(#impact-arrow)" stroke="var(--compass-blue)" strokeDasharray="8 6" strokeWidth="2.5" />
        <g fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="11" textAnchor="middle">
          <text x="240" y="112">CALLS</text>
          <text x="500" y="112">CALLS</text>
          <text x="760" y="112">CALLS</text>
        </g>
        <g>
          <rect fill="var(--compass-indigo)" height="92" rx="16" width="180" x="20" y="118" />
          <text fill="var(--primary-foreground)" fontFamily="var(--font-plex-mono)" fontSize="10" x="40" y="143">CHANGED SYMBOL</text>
          <text fill="var(--primary-foreground)" fontFamily="var(--font-space-grotesk)" fontSize="17" fontWeight="600" x="40" y="168">charge()</text>
          <text fill="var(--primary-foreground)" fontFamily="var(--font-plex-mono)" fontSize="10" opacity="0.78" x="40" y="190">src/payments.rs:42</text>
          <rect fill="var(--background)" height="92" rx="16" stroke="var(--border)" width="180" x="280" y="118" />
          <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="10" x="300" y="143">TARGET 01</text>
          <text fill="var(--foreground)" fontFamily="var(--font-space-grotesk)" fontSize="17" fontWeight="600" x="300" y="168">authorize()</text>
          <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="10" x="300" y="190">src/gateway.rs:88</text>
          <rect fill="var(--background)" height="92" rx="16" stroke="var(--border)" width="180" x="540" y="118" />
          <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="10" x="560" y="143">TARGET 02</text>
          <text fill="var(--foreground)" fontFamily="var(--font-space-grotesk)" fontSize="17" fontWeight="600" x="560" y="168">CheckoutHandler</text>
          <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="10" x="560" y="190">src/checkout.rs:17</text>
          <rect fill="var(--background)" height="92" rx="16" stroke="var(--border)" width="180" x="800" y="118" />
          <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="10" x="820" y="143">TARGET 03</text>
          <text fill="var(--foreground)" fontFamily="var(--font-space-grotesk)" fontSize="17" fontWeight="600" x="820" y="168">write_receipt()</text>
          <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="10" x="820" y="190">src/receipt.rs:61</text>
        </g>
        <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="11" textAnchor="middle" x="520" y="270">direction: changed → affected · depth 1..3 · max 100 results · anchors retained</text>
      </svg>
    </DiagramShell>
  );
}

export function HistoryComparisonDiagram() {
  return (
    <DiagramShell eyebrow="Immutable comparison" title="Compare realizations without rewriting either one.">
      <svg
        aria-labelledby="history-diagram-title history-diagram-description"
        className="h-auto w-full"
        role="img"
        viewBox="0 0 820 310"
      >
        <title id="history-diagram-title">Compass graph history comparison</title>
        <desc id="history-diagram-description">Two graph realizations bound to separate commits show stable nodes, an added edge, and a changed node.</desc>
        <path d="M410 47V266" stroke="var(--border)" strokeDasharray="4 6" />
        <g fontFamily="var(--font-plex-mono)" fontSize="11">
          <rect fill="var(--compass-canvas-deep)" height="32" rx="16" stroke="var(--border)" width="180" x="99" y="20" />
          <text fill="var(--compass-ink-soft)" x="124" y="41">parent · a83f2c</text>
          <rect fill="var(--compass-canvas-deep)" height="32" rx="16" stroke="var(--border)" width="180" x="541" y="20" />
          <text fill="var(--compass-ink-soft)" x="566" y="41">current · f0b219</text>
        </g>
        <g fill="none" stroke="var(--compass-ink-soft)" strokeWidth="2">
          <path d="M157 148L273 101L321 205L184 231Z" />
          <path d="M499 148L615 101L663 205L526 231Z" />
        </g>
        <g>
          <circle cx="157" cy="148" fill="var(--compass-blue)" r="11" />
          <circle cx="273" cy="101" fill="var(--compass-blue)" r="11" />
          <circle cx="321" cy="205" fill="var(--compass-blue)" r="11" />
          <circle cx="184" cy="231" fill="var(--compass-blue)" r="11" />
          <circle cx="499" cy="148" fill="var(--compass-blue)" r="11" />
          <circle cx="615" cy="101" fill="var(--compass-blue)" r="11" />
          <circle cx="663" cy="205" fill="var(--compass-indigo)" r="14" />
          <circle cx="526" cy="231" fill="var(--compass-blue)" r="11" />
          <path d="M615 101L663 205" fill="none" stroke="var(--compass-indigo)" strokeDasharray="5 4" strokeWidth="3" />
        </g>
        <g fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="11" textAnchor="middle">
          <text x="410" y="292">same identity · changed topology · evidence-gated diff</text>
        </g>
      </svg>
    </DiagramShell>
  );
}

export function AssistantContextDiagram() {
  return (
    <DiagramShell eyebrow="Focused context" title="One question in, one bounded answer out.">
      <svg
        aria-labelledby="assistant-diagram-title assistant-diagram-description"
        className="h-auto w-full"
        role="img"
        viewBox="0 0 820 260"
      >
        <title id="assistant-diagram-title">Compass assistant context flow</title>
        <desc id="assistant-diagram-description">A local repository is queried through CompassQL and returns a compact answer with identity, path, and provenance.</desc>
        <defs>
          <marker id="assistant-arrow" markerHeight="8" markerWidth="8" orient="auto" refX="7" refY="4">
            <path d="M0 0L8 4L0 8Z" fill="var(--compass-blue)" />
          </marker>
        </defs>
        <path d="M208 130H319M501 130H612" fill="none" markerEnd="url(#assistant-arrow)" stroke="var(--compass-blue)" strokeWidth="2" />
        <g fontFamily="var(--font-space-grotesk)" fontSize="16" fontWeight="600" textAnchor="middle">
          <rect fill="var(--background)" height="84" rx="14" stroke="var(--border)" width="170" x="38" y="88" />
          <text fill="var(--foreground)" x="123" y="124">Repository</text>
          <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="10" fontWeight="400" x="123" y="148">local files + graph</text>
          <rect fill="var(--compass-canvas-deep)" height="84" rx="14" stroke="var(--compass-blue)" width="170" x="325" y="88" />
          <text fill="var(--foreground)" x="410" y="124">CompassQL</text>
          <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="10" fontWeight="400" x="410" y="148">bounded read-only query</text>
          <rect fill="var(--background)" height="84" rx="14" stroke="var(--border)" width="170" x="612" y="88" />
          <text fill="var(--foreground)" x="697" y="124">Answer</text>
          <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="10" fontWeight="400" x="697" y="148">path + evidence</text>
        </g>
        <g fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="11" textAnchor="middle">
          <text x="262" y="63">no context dump</text>
          <text x="553" y="63">machine-readable</text>
          <text x="410" y="235">identity · direction · bounds · provenance</text>
        </g>
      </svg>
    </DiagramShell>
  );
}

export function EditorSurfaceDiagram() {
  return (
    <DiagramShell eyebrow="Editor surface" title="Keep source, graph, and inspection in one working set.">
      <svg
        aria-labelledby="editor-surface-title editor-surface-description"
        className="h-auto w-full"
        role="img"
        viewBox="0 0 980 360"
      >
        <title id="editor-surface-title">Compass VS Code editor surface</title>
        <desc id="editor-surface-description">A source editor points to a shared Compass graph. Selecting a node reveals a source-aware inspector without leaving the editor.</desc>
        <defs>
          <marker id="editor-surface-arrow" markerHeight="8" markerWidth="8" orient="auto" refX="7" refY="4">
            <path d="M0 0L8 4L0 8Z" fill="var(--compass-blue)" />
          </marker>
        </defs>
        <rect fill="var(--compass-canvas-deep)" height="278" rx="14" stroke="var(--border)" width="320" x="18" y="42" />
        <rect fill="var(--background)" height="34" rx="14" stroke="var(--border)" width="320" x="18" y="42" />
        <path d="M18 76H338" stroke="var(--border)" />
        <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="10" x="36" y="64">src / checkout.rs</text>
        <circle cx="309" cy="59" fill="var(--compass-amber)" r="4" />
        <g fontFamily="var(--font-plex-mono)" fontSize="11">
          <text fill="var(--compass-ink-soft)" x="40" y="111">38</text>
          <text fill="var(--compass-ink-soft)" x="40" y="137">39</text>
          <text fill="var(--compass-ink-soft)" x="40" y="163">40</text>
          <text fill="var(--compass-ink-soft)" x="40" y="189">41</text>
          <text fill="var(--compass-ink-soft)" x="40" y="215">42</text>
          <text fill="var(--compass-ink-soft)" x="40" y="241">43</text>
          <text fill="var(--compass-ink-soft)" x="66" y="111"><tspan fill="var(--compass-blue)">fn</tspan> handle_checkout(</text>
          <text fill="var(--compass-ink-soft)" x="66" y="137">  request: Request,</text>
          <text fill="var(--compass-ink-soft)" x="66" y="163">) -&gt; Result&lt;Receipt&gt; {'{'}</text>
          <rect fill="color-mix(in srgb, var(--compass-indigo) 22%, transparent)" height="22" rx="5" width="210" x="62" y="199" />
          <text fill="var(--foreground)" x="66" y="215">  payment.charge(request)</text>
          <text fill="var(--compass-ink-soft)" x="66" y="241">{'}'}</text>
        </g>
        <path d="M338 181H397" fill="none" markerEnd="url(#editor-surface-arrow)" stroke="var(--compass-blue)" strokeWidth="2" />
        <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="10" textAnchor="middle" x="367" y="162">source anchor</text>
        <rect fill="var(--background)" height="278" rx="14" stroke="var(--border)" width="548" x="414" y="42" />
        <rect fill="var(--compass-canvas-deep)" height="34" rx="14" stroke="var(--border)" width="548" x="414" y="42" />
        <path d="M414 76H962" stroke="var(--border)" />
        <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="10" x="434" y="64">GRAPH / CURRENT SNAPSHOT</text>
        <text fill="var(--compass-blue)" fontFamily="var(--font-plex-mono)" fontSize="10" textAnchor="end" x="940" y="64">12 nodes · 12 edges</text>
        <path d="M502 198L616 132L734 214L842 146M616 132L734 214" fill="none" stroke="var(--compass-blue)" strokeOpacity="0.68" strokeWidth="2" />
        <circle cx="502" cy="198" fill="var(--compass-blue)" r="15" />
        <circle cx="616" cy="132" fill="var(--compass-indigo)" r="20" />
        <circle cx="734" cy="214" fill="var(--compass-blue)" r="17" />
        <circle cx="842" cy="146" fill="var(--compass-blue)" r="14" />
        <g fill="var(--foreground)" fontFamily="var(--font-space-grotesk)" fontSize="13" fontWeight="600" textAnchor="middle">
          <text x="502" y="232">charge()</text>
          <text x="616" y="170">CheckoutHandler</text>
          <text x="734" y="248">authorize()</text>
          <text x="842" y="180">Receipt</text>
        </g>
        <rect fill="var(--card)" height="105" rx="10" stroke="var(--compass-blue)" width="198" x="734" y="88" />
        <text fill="var(--compass-blue)" fontFamily="var(--font-plex-mono)" fontSize="9" x="750" y="108">SELECTED NODE</text>
        <text fill="var(--foreground)" fontFamily="var(--font-space-grotesk)" fontSize="14" fontWeight="600" x="750" y="132">CheckoutHandler</text>
        <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="9" x="750" y="151">src/checkout.rs:17</text>
        <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="9" x="750" y="168">kind · function · direct edges</text>
        <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="10" textAnchor="middle" x="688" y="300">click to focus · double-click to open source · hover for edge evidence</text>
      </svg>
    </DiagramShell>
  );
}

export function McpSurfaceDiagram() {
  return (
    <DiagramShell eyebrow="Tool boundary" title="A focused request crosses the boundary; the repository stays local.">
      <svg
        aria-labelledby="mcp-surface-title mcp-surface-description"
        className="h-auto w-full"
        role="img"
        viewBox="0 0 980 310"
      >
        <title id="mcp-surface-title">Compass MCP integration</title>
        <desc id="mcp-surface-description">A tool sends a structured query to the local Compass MCP server, which returns a bounded graph answer with source evidence.</desc>
        <defs>
          <marker id="mcp-surface-arrow" markerHeight="8" markerWidth="8" orient="auto" refX="7" refY="4">
            <path d="M0 0L8 4L0 8Z" fill="var(--compass-blue)" />
          </marker>
        </defs>
        <path d="M278 146H374M606 146H702" fill="none" markerEnd="url(#mcp-surface-arrow)" stroke="var(--compass-blue)" strokeWidth="2.5" />
        <g fontFamily="var(--font-space-grotesk)" fontSize="18" fontWeight="600" textAnchor="middle">
          <rect fill="var(--background)" height="132" rx="16" stroke="var(--border)" width="240" x="38" y="80" />
          <text fill="var(--foreground)" x="158" y="124">Tool request</text>
          <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="11" fontWeight="400" x="158" y="150">query_cql / graph resource</text>
          <text fill="var(--compass-blue)" fontFamily="var(--font-plex-mono)" fontSize="10" fontWeight="400" x="158" y="177">local process boundary</text>
          <rect fill="var(--compass-canvas-deep)" height="132" rx="16" stroke="var(--compass-blue)" width="240" x="374" y="80" />
          <text fill="var(--foreground)" x="494" y="124">Compass MCP</text>
          <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="11" fontWeight="400" x="494" y="150">load · validate · bound</text>
          <text fill="var(--compass-blue)" fontFamily="var(--font-plex-mono)" fontSize="10" fontWeight="400" x="494" y="177">compass:// resources</text>
          <rect fill="var(--background)" height="132" rx="16" stroke="var(--border)" width="240" x="702" y="80" />
          <text fill="var(--foreground)" x="822" y="124">Typed answer</text>
          <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="11" fontWeight="400" x="822" y="150">nodes · edges · diagnostics</text>
          <text fill="var(--compass-blue)" fontFamily="var(--font-plex-mono)" fontSize="10" fontWeight="400" x="822" y="177">path + source anchors</text>
        </g>
        <g fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="10" textAnchor="middle">
          <text x="326" y="112">structured input</text>
          <text x="654" y="112">machine-readable output</text>
          <text x="494" y="252">no hosted index · explicit limits · same graph model as VS Code</text>
        </g>
      </svg>
    </DiagramShell>
  );
}

export function AutomationSurfaceDiagram() {
  return (
    <DiagramShell eyebrow="Automation surface" title="Make structural checks repeatable in the same pipeline as your code.">
      <svg
        aria-labelledby="automation-surface-title automation-surface-description"
        className="h-auto w-full"
        role="img"
        viewBox="0 0 980 320"
      >
        <title id="automation-surface-title">Compass CLI and CI integration</title>
        <desc id="automation-surface-description">A repository revision is built locally or in CI, queried with CompassQL, and published as bounded JSON output with an explicit status.</desc>
        <defs>
          <marker id="automation-surface-arrow" markerHeight="8" markerWidth="8" orient="auto" refX="7" refY="4">
            <path d="M0 0L8 4L0 8Z" fill="var(--compass-blue)" />
          </marker>
        </defs>
        <path d="M188 150H274M438 150H524M688 150H774" fill="none" markerEnd="url(#automation-surface-arrow)" stroke="var(--compass-blue)" strokeWidth="2.5" />
        <g fontFamily="var(--font-space-grotesk)" fontSize="17" fontWeight="600" textAnchor="middle">
          <rect fill="var(--background)" height="108" rx="14" stroke="var(--border)" width="150" x="38" y="96" />
          <text fill="var(--foreground)" x="113" y="137">Revision</text>
          <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="10" fontWeight="400" x="113" y="162">commit / workspace</text>
          <rect fill="var(--compass-canvas-deep)" height="108" rx="14" stroke="var(--compass-blue)" width="150" x="274" y="96" />
          <text fill="var(--foreground)" x="349" y="137">compass build</text>
          <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="10" fontWeight="400" x="349" y="162">graph.json + manifest</text>
          <rect fill="var(--compass-canvas-deep)" height="108" rx="14" stroke="var(--compass-blue)" width="150" x="524" y="96" />
          <text fill="var(--foreground)" x="599" y="137">CompassQL</text>
          <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="10" fontWeight="400" x="599" y="162">--cql · bounded</text>
          <rect fill="var(--background)" height="108" rx="14" stroke="var(--border)" width="150" x="774" y="96" />
          <text fill="var(--foreground)" x="849" y="137">Result</text>
          <text fill="var(--compass-blue)" fontFamily="var(--font-plex-mono)" fontSize="10" fontWeight="400" x="849" y="162">json / jsonl · status</text>
        </g>
        <g fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="10" textAnchor="middle">
          <text x="231" y="126">scope</text>
          <text x="481" y="126">snapshot</text>
          <text x="731" y="126">contract</text>
          <text x="494" y="260">same command locally or in CI · no hidden network fallback</text>
        </g>
      </svg>
    </DiagramShell>
  );
}

export function ExportSurfaceDiagram() {
  return (
    <DiagramShell eyebrow="Portable outputs" title="Publish once, choose the surface your team needs.">
      <svg
        aria-labelledby="export-surface-title export-surface-description"
        className="h-auto w-full"
        role="img"
        viewBox="0 0 980 330"
      >
        <title id="export-surface-title">Compass portable graph outputs</title>
        <desc id="export-surface-description">A validated graph JSON snapshot feeds the interactive viewer, SVG and GraphML exports, and documentation-oriented formats.</desc>
        <defs>
          <marker id="export-surface-arrow" markerHeight="8" markerWidth="8" orient="auto" refX="7" refY="4">
            <path d="M0 0L8 4L0 8Z" fill="var(--compass-blue)" />
          </marker>
        </defs>
        <rect fill="var(--compass-indigo)" height="108" rx="16" width="210" x="385" y="104" />
        <text fill="var(--primary-foreground)" fontFamily="var(--font-space-grotesk)" fontSize="20" fontWeight="600" textAnchor="middle" x="490" y="146">graph.json</text>
        <text fill="var(--primary-foreground)" fontFamily="var(--font-plex-mono)" fontSize="10" opacity="0.8" textAnchor="middle" x="490" y="170">validated snapshot</text>
        <path d="M385 152H248M595 152H732M385 184L248 250M595 184L732 250" fill="none" markerEnd="url(#export-surface-arrow)" stroke="var(--compass-blue)" strokeWidth="2" />
        <g fontFamily="var(--font-space-grotesk)" fontSize="16" fontWeight="600" textAnchor="middle">
          <rect fill="var(--background)" height="72" rx="12" stroke="var(--border)" width="190" x="38" y="116" />
          <text fill="var(--foreground)" x="133" y="147">graph.html</text>
          <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="10" fontWeight="400" x="133" y="167">interactive viewer</text>
          <rect fill="var(--background)" height="72" rx="12" stroke="var(--border)" width="190" x="752" y="116" />
          <text fill="var(--foreground)" x="847" y="147">graph.svg</text>
          <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="10" fontWeight="400" x="847" y="167">static diagram</text>
          <rect fill="var(--background)" height="72" rx="12" stroke="var(--border)" width="190" x="38" y="238" />
          <text fill="var(--foreground)" x="133" y="269">GraphML</text>
          <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="10" fontWeight="400" x="133" y="289">portable interchange</text>
          <rect fill="var(--background)" height="72" rx="12" stroke="var(--border)" width="190" x="752" y="238" />
          <text fill="var(--foreground)" x="847" y="269">Wiki / Obsidian</text>
          <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="10" fontWeight="400" x="847" y="289">team-readable notes</text>
        </g>
        <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="10" textAnchor="middle" x="490" y="55">identity · direction · evidence preserved at every projection</text>
      </svg>
    </DiagramShell>
  );
}
