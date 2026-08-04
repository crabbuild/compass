import { BracesIcon, FileOutputIcon, GitPullRequestArrowIcon, MonitorDotIcon, Share2Icon } from 'lucide-react';
import Link from 'next/link';

import { McpSurfaceDiagram } from '@/components/diagrams';
import { FeatureGrid, MarketingPage, PageSection } from '@/components/marketing-page';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { pageMetadata } from '@/lib/site';

export const metadata = pageMetadata('Integrations', 'Bring Compass into editors, assistants, CI, and portable graph workflows without leaving the local workspace.');

const surfaces = [
  { eyebrow: 'Editor', title: 'VS Code extension', description: 'Explore current graphs, cursor-rooted call graphs, architecture flow, queries, and exact Git evolution beside the code.', href: '/integrations/vscode' },
  { eyebrow: 'Assistant', title: 'MCP and native skills', description: 'Expose focused query and graph resources to tools that need structured context without a repository-sized prompt.', href: '/integrations/mcp' },
  { eyebrow: 'Automation', title: 'CLI and CI', description: 'Run deterministic CompassQL checks with bounded output, explicit status, and machine-readable results.', href: '/integrations/cli' },
  { eyebrow: 'Delivery', title: 'CI review surface', description: 'Carry graph artifacts and query results through pull-request checks without changing the source of truth.', href: '/integrations/ci' },
  { eyebrow: 'Export', title: 'Portable graph formats', description: 'Project one validated graph into HTML, SVG, GraphML, Wiki, Obsidian, and JSON artifacts.', href: '/integrations/graph-formats' },
];

export default function IntegrationsPage() {
  return <MarketingPage eyebrow="Integration surfaces" title="Meet the workflow where it already lives." description="Compass is a native CLI first, then a set of focused surfaces for editors, assistants, automation, and portable graph formats. Every surface consumes the same local graph model and keeps the boundary explicit.">
    <PageSection eyebrow="Choose a surface" title="One local graph, several useful views." description="The integration is not a second graph. It is a different way to inspect, query, or carry the same evidence-backed snapshot.">
      <FeatureGrid items={surfaces} />
    </PageSection>
    <section className="border-y border-border/70 bg-muted/25">
      <div className="mx-auto grid max-w-7xl gap-8 px-5 py-20 lg:grid-cols-[1.08fr_0.92fr] lg:items-center lg:px-8 lg:py-28">
        <McpSurfaceDiagram />
        <div className="flex flex-col gap-6">
          <div>
            <p className="eyebrow">One contract across tools</p>
            <h2 className="mt-4 font-heading text-3xl font-semibold tracking-[-0.05em] sm:text-4xl">Keep the answer small without making it vague.</h2>
            <p className="mt-5 text-base leading-7 text-muted-foreground">The viewer, CLI, MCP server, and exporters all work from stable node identity, directed edges, source anchors, and provenance. A tool can ask a smaller question without losing the reason behind the result.</p>
          </div>
          <div className="grid gap-3 sm:grid-cols-2">
            <IntegrationFact label="local" text="source and graph stay on the machine" />
            <IntegrationFact label="typed" text="versioned JSON contracts" />
            <IntegrationFact label="bounded" text="limits travel with the query" />
            <IntegrationFact label="portable" text="exports keep the graph useful" />
          </div>
        </div>
      </div>
    </section>
    <PageSection eyebrow="Surface map" title="Pick the place where the next decision happens." description="Each integration is designed around a different hand-off, so the page explains what enters, what Compass does, and what leaves.">
      <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-5">
        <SurfaceCard icon={MonitorDotIcon} title="Editor" text="inspect beside source" href="/integrations/vscode" />
        <SurfaceCard icon={Share2Icon} title="MCP" text="serve focused context" href="/integrations/mcp" />
        <SurfaceCard icon={BracesIcon} title="CLI" text="script exact queries" href="/integrations/cli" />
        <SurfaceCard icon={GitPullRequestArrowIcon} title="CI" text="review repeatable output" href="/integrations/ci" />
        <SurfaceCard icon={FileOutputIcon} title="Export" text="carry the snapshot" href="/integrations/graph-formats" />
      </div>
    </PageSection>
  </MarketingPage>;
}

function IntegrationFact({ label, text }: { label: string; text: string }) {
  return <div className="rounded-xl border border-border/80 bg-card/70 px-4 py-3"><span className="font-mono text-xs uppercase tracking-[0.12em] text-primary">{label}</span><p className="mt-1 text-sm text-muted-foreground">{text}</p></div>;
}

function SurfaceCard({ icon: Icon, title, text, href }: { icon: typeof MonitorDotIcon; title: string; text: string; href: string }) {
  return <Card className="group relative border-border/80 bg-card/70 shadow-none transition-transform duration-300 hover:-translate-y-1"><CardHeader className="gap-4"><Icon className="text-primary" /><CardTitle className="font-heading text-lg tracking-[-0.035em]"><Link className="after:absolute after:inset-0" href={href}>{title}</Link></CardTitle></CardHeader><CardContent><p className="text-sm leading-6 text-muted-foreground">{text}</p></CardContent></Card>;
}
