import type { ReactNode } from 'react';

function BlogDiagramShell({
  eyebrow,
  title,
  children,
}: {
  eyebrow: string;
  title: string;
  children: ReactNode;
}) {
  return (
    <figure className="not-prose my-10 overflow-hidden rounded-2xl border border-border/80 bg-card/70 shadow-sm">
      <figcaption className="flex flex-wrap items-center justify-between gap-3 border-b border-border/70 px-5 py-4 sm:px-6">
        <div>
          <p className="eyebrow">{eyebrow}</p>
          <p className="mt-1 font-heading text-sm font-semibold tracking-[-0.02em]">{title}</p>
        </div>
        <span className="font-mono text-[0.62rem] uppercase tracking-[0.14em] text-muted-foreground">Compass / field note</span>
      </figcaption>
      <div className="p-4 sm:p-6">{children}</div>
    </figure>
  );
}

export function LaunchMapDiagram() {
  return (
    <BlogDiagramShell eyebrow="A first snapshot" title="One local build, several ways to ask a question.">
      <svg
        aria-labelledby="launch-map-title launch-map-description"
        className="h-auto w-full"
        role="img"
        viewBox="0 0 960 300"
      >
        <title id="launch-map-title">From a repository to Compass surfaces</title>
        <desc id="launch-map-description">
          A repository is scoped and parsed into a manifest and source facts, resolved into graph JSON, and then opened in the viewer, CompassQL, or an automation surface.
        </desc>
        <defs>
          <marker id="blog-launch-arrow" markerHeight="8" markerWidth="8" orient="auto" refX="7" refY="4">
            <path d="M0 0L8 4L0 8Z" fill="var(--compass-blue)" />
          </marker>
        </defs>
        <g fill="none" markerEnd="url(#blog-launch-arrow)" stroke="var(--compass-blue)" strokeWidth="2.5">
          <path d="M205 132H255" />
          <path d="M445 132H495" />
          <path d="M685 132H735" />
        </g>
        <g>
          <rect fill="var(--background)" height="132" rx="16" stroke="var(--border)" width="181" x="24" y="66" />
          <circle cx="54" cy="96" fill="var(--compass-indigo)" r="14" />
          <text fill="var(--primary-foreground)" fontFamily="var(--font-plex-mono)" fontSize="9" textAnchor="middle" x="54" y="100">01</text>
          <text fill="var(--foreground)" fontFamily="var(--font-space-grotesk)" fontSize="18" fontWeight="600" x="78" y="101">Repository</text>
          <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="10" x="42" y="127">scope + ignore rules</text>
          <rect fill="var(--compass-canvas-deep)" height="25" rx="7" stroke="var(--border)" width="132" x="42" y="150" />
          <text fill="var(--compass-blue)" fontFamily="var(--font-plex-mono)" fontSize="10" x="54" y="166">src/ · Cargo.toml</text>
        </g>
        <g>
          <rect fill="var(--background)" height="132" rx="16" stroke="var(--border)" width="181" x="264" y="66" />
          <circle cx="294" cy="96" fill="var(--compass-blue)" r="14" />
          <text fill="var(--primary-foreground)" fontFamily="var(--font-plex-mono)" fontSize="9" textAnchor="middle" x="294" y="100">02</text>
          <text fill="var(--foreground)" fontFamily="var(--font-space-grotesk)" fontSize="18" fontWeight="600" x="318" y="101">Build</text>
          <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="10" x="282" y="127">facts + anchors</text>
          <rect fill="var(--compass-canvas-deep)" height="25" rx="7" stroke="var(--border)" width="132" x="282" y="150" />
          <text fill="var(--compass-blue)" fontFamily="var(--font-plex-mono)" fontSize="10" x="294" y="166">manifest.json</text>
        </g>
        <g>
          <rect fill="var(--background)" height="132" rx="16" stroke="var(--border)" width="181" x="504" y="66" />
          <circle cx="534" cy="96" fill="var(--compass-blue)" r="14" />
          <text fill="var(--primary-foreground)" fontFamily="var(--font-plex-mono)" fontSize="9" textAnchor="middle" x="534" y="100">03</text>
          <text fill="var(--foreground)" fontFamily="var(--font-space-grotesk)" fontSize="18" fontWeight="600" x="558" y="101">Snapshot</text>
          <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="10" x="522" y="127">directed graph</text>
          <rect fill="var(--compass-canvas-deep)" height="25" rx="7" stroke="var(--border)" width="132" x="522" y="150" />
          <text fill="var(--compass-blue)" fontFamily="var(--font-plex-mono)" fontSize="10" x="534" y="166">graph.json</text>
        </g>
        <g>
          <rect fill="var(--compass-canvas-deep)" height="132" rx="16" stroke="var(--compass-blue)" width="181" x="744" y="66" />
          <circle cx="774" cy="96" fill="var(--compass-indigo)" r="14" />
          <text fill="var(--primary-foreground)" fontFamily="var(--font-plex-mono)" fontSize="9" textAnchor="middle" x="774" y="100">04</text>
          <text fill="var(--foreground)" fontFamily="var(--font-space-grotesk)" fontSize="18" fontWeight="600" x="798" y="101">Ask</text>
          <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="10" x="762" y="127">same model, new view</text>
          <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="10" x="762" y="157">viewer · CompassQL</text>
          <text fill="var(--compass-blue)" fontFamily="var(--font-plex-mono)" fontSize="10" x="762" y="175">CLI · MCP · CI</text>
        </g>
        <g fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="10" textAnchor="middle">
          <text x="114" y="246">local files</text>
          <text x="354" y="246">source evidence</text>
          <text x="594" y="246">validated artifact</text>
          <text x="834" y="246">bounded answer</text>
        </g>
        <path d="M114 258H834" fill="none" stroke="var(--border)" strokeDasharray="4 6" />
      </svg>
    </BlogDiagramShell>
  );
}

export function EvidenceAnatomyDiagram() {
  return (
    <BlogDiagramShell eyebrow="Edge anatomy" title="A relationship is a claim with a route back to source.">
      <svg
        aria-labelledby="blog-evidence-title blog-evidence-description"
        className="h-auto w-full"
        role="img"
        viewBox="0 0 900 360"
      >
        <title id="blog-evidence-title">An evidence-backed CALLS relationship</title>
        <desc id="blog-evidence-description">
          A caller points to a target with a directed CALLS relationship. The edge keeps its relationship site, source anchors, provenance, and confidence.
        </desc>
        <defs>
          <marker id="blog-evidence-arrow" markerHeight="8" markerWidth="8" orient="auto" refX="7" refY="4">
            <path d="M0 0L8 4L0 8Z" fill="var(--compass-blue)" />
          </marker>
        </defs>
        <path d="M246 90H653" fill="none" markerEnd="url(#blog-evidence-arrow)" stroke="var(--compass-blue)" strokeWidth="3" />
        <rect fill="var(--background)" height="88" rx="16" stroke="var(--border)" width="202" x="28" y="46" />
        <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="10" x="50" y="72">CALLER</text>
        <text fill="var(--foreground)" fontFamily="var(--font-space-grotesk)" fontSize="18" fontWeight="600" x="50" y="101">CheckoutHandler</text>
        <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="10" x="50" y="120">src/checkout.rs:17</text>
        <rect fill="var(--compass-canvas-deep)" height="32" rx="16" stroke="var(--border)" width="142" x="354" y="74" />
        <text fill="var(--foreground)" fontFamily="var(--font-plex-mono)" fontSize="10" textAnchor="middle" x="425" y="95">CALLS · extracted</text>
        <rect fill="var(--background)" height="88" rx="16" stroke="var(--border)" width="202" x="670" y="46" />
        <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="10" x="692" y="72">TARGET</text>
        <text fill="var(--foreground)" fontFamily="var(--font-space-grotesk)" fontSize="18" fontWeight="600" x="692" y="101">charge()</text>
        <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="10" x="692" y="120">src/payments.rs:42</text>
        <g>
          <rect fill="var(--background)" height="122" rx="14" stroke="var(--border)" width="258" x="28" y="190" />
          <text fill="var(--compass-blue)" fontFamily="var(--font-plex-mono)" fontSize="10" x="50" y="217">RELATIONSHIP SITE</text>
          <text fill="var(--foreground)" fontFamily="var(--font-space-grotesk)" fontSize="15" fontWeight="600" x="50" y="245">call expression</text>
          <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="10" x="50" y="268">src/checkout.rs:42</text>
          <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="10" x="50" y="287">the edge starts here</text>
        </g>
        <g>
          <rect fill="var(--background)" height="122" rx="14" stroke="var(--border)" width="258" x="321" y="190" />
          <text fill="var(--compass-blue)" fontFamily="var(--font-plex-mono)" fontSize="10" x="343" y="217">SOURCE ANCHOR</text>
          <text fill="var(--foreground)" fontFamily="var(--font-space-grotesk)" fontSize="15" fontWeight="600" x="343" y="245">target definition</text>
          <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="10" x="343" y="268">src/payments.rs:42</text>
          <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="10" x="343" y="287">open the exact symbol</text>
        </g>
        <g>
          <rect fill="var(--background)" height="122" rx="14" stroke="var(--border)" width="258" x="614" y="190" />
          <text fill="var(--compass-blue)" fontFamily="var(--font-plex-mono)" fontSize="10" x="636" y="217">PROVENANCE</text>
          <text fill="var(--foreground)" fontFamily="var(--font-space-grotesk)" fontSize="15" fontWeight="600" x="636" y="245">parser evidence</text>
          <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="10" x="636" y="268">confidence: extracted</text>
          <text fill="var(--compass-ink-soft)" fontFamily="var(--font-plex-mono)" fontSize="10" x="636" y="287">not an invented link</text>
        </g>
        <path d="M129 134L129 190M772 134L772 190" fill="none" stroke="var(--border)" strokeDasharray="4 6" />
      </svg>
    </BlogDiagramShell>
  );
}

export type GuideStep = {
  index: string;
  title: string;
  text: string;
};

export function GuideSteps({ items }: { items: readonly GuideStep[] }) {
  return (
    <div className="not-prose my-8 grid gap-3 sm:grid-cols-2">
      {items.map((item) => (
        <div className="rounded-xl border border-border/80 bg-card/65 p-5" key={item.index}>
          <p className="font-mono text-xs text-primary">{item.index}</p>
          <h3 className="mt-4 font-heading text-base font-semibold tracking-[-0.025em]">{item.title}</h3>
          <p className="mt-2 text-sm leading-6 text-muted-foreground">{item.text}</p>
        </div>
      ))}
    </div>
  );
}

export function EvidenceStates() {
  const states = [
    ['extracted', 'The parser saw the relationship directly in source.'],
    ['inferred', 'Resolution connected compatible facts across files.'],
    ['ambiguous', 'Several targets remain possible; Compass keeps them visible.'],
    ['unresolved', 'The source did not support a safe target yet.'],
  ];

  return (
    <div className="not-prose my-8 grid gap-3 sm:grid-cols-2">
      {states.map(([label, text]) => (
        <div className="flex gap-3 rounded-xl border border-border/80 bg-card/65 p-4" key={label}>
          <span className="mt-1 h-2.5 w-2.5 shrink-0 rounded-full bg-primary" aria-hidden="true" />
          <div>
            <p className="font-mono text-xs uppercase tracking-[0.12em] text-primary">{label}</p>
            <p className="mt-1 text-sm leading-6 text-muted-foreground">{text}</p>
          </div>
        </div>
      ))}
    </div>
  );
}
