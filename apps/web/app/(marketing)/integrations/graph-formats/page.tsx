import { ArrowRightIcon, FileCode2Icon, FileOutputIcon, NetworkIcon } from 'lucide-react';
import Link from 'next/link';

import { ExportSurfaceDiagram } from '@/components/diagrams';
import { FeatureGrid, MarketingPage, PageSection } from '@/components/marketing-page';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { buttonVariants } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import { pageMetadata } from '@/lib/site';

export const metadata = pageMetadata('Graph formats', 'Carry Compass graph snapshots into interactive HTML, SVG, GraphML, Wiki, Obsidian, and JSON workflows.');

export default function GraphFormatsPage() {
  return <MarketingPage eyebrow="Integration / exports" title="Carry the graph without carrying the runtime." description="Compass keeps the validated snapshot portable. Choose an interactive viewer, a static diagram, a graph interchange format, or a team-readable note without changing the identity and evidence underneath.">
    <PageSection eyebrow="Portable outputs" title="Publish once, choose the surface your team needs." description="The export is a projection of the same graph contract, not a second source of truth.">
      <ExportSurfaceDiagram />
    </PageSection>
    <section className="border-y border-border/70 bg-muted/25"><div className="mx-auto max-w-7xl px-5 py-20 lg:px-8 lg:py-28"><FeatureGrid items={[{ eyebrow: 'Interactive', title: 'graph.html', description: 'Open the shared viewer when people need hover evidence, focus, and source-aware exploration.' }, { eyebrow: 'Static', title: 'graph.svg', description: 'Embed a bounded diagram in a design review, issue, or architecture document.' }, { eyebrow: 'Interchange', title: 'GraphML and JSON', description: 'Keep machine-readable artifacts for scripts, archives, and downstream tooling.' }, { eyebrow: 'Knowledge', title: 'Wiki and Obsidian', description: 'Turn the graph into readable notes that stay close to the team’s existing workspace.' }]} /></div></section>
    <PageSection eyebrow="What remains stable" title="Format changes do not erase meaning." description="Every projection retains the fields that make a relationship inspectable, then adds only the presentation the destination needs.">
      <div className="grid gap-6 lg:grid-cols-[1.1fr_0.9fr]">
        <Card className="border-border/80 bg-card/70 shadow-none"><CardHeader><CardTitle className="font-heading text-2xl tracking-[-0.045em]">Portable by contract</CardTitle></CardHeader><CardContent className="grid gap-4 text-sm leading-7 text-muted-foreground sm:grid-cols-2"><span className="flex gap-3"><NetworkIcon className="mt-1 size-4 shrink-0 text-primary" />stable node identity</span><span className="flex gap-3"><FileCode2Icon className="mt-1 size-4 shrink-0 text-primary" />source anchors</span><span className="flex gap-3"><NetworkIcon className="mt-1 size-4 shrink-0 text-primary" />directed relationships</span><span className="flex gap-3"><FileOutputIcon className="mt-1 size-4 shrink-0 text-primary" />explicit provenance</span></CardContent></Card>
        <Card className="overflow-hidden border-border/80 bg-compass-canvas-deep shadow-none"><CardHeader className="border-b border-border/70"><CardTitle className="font-mono text-sm tracking-normal">export manifest</CardTitle></CardHeader><CardContent className="p-6 font-mono text-xs leading-7 text-muted-foreground"><p><span className="text-primary">snapshot</span>: graph.json</p><p><span className="text-primary">interactive</span>: graph.html</p><p><span className="text-primary">static</span>: graph.svg</p><p><span className="text-primary">interchange</span>: graph.graphml</p></CardContent></Card>
      </div>
    </PageSection>
    <section className="border-t border-border/70 bg-primary text-primary-foreground"><div className="mx-auto flex max-w-7xl flex-col gap-6 px-5 py-16 lg:flex-row lg:items-center lg:justify-between lg:px-8"><div><p className="font-heading text-2xl font-semibold tracking-[-0.04em]">Let the graph travel.</p><p className="mt-2 text-sm text-primary-foreground/75">Build locally, then choose the format that makes the next conversation easier.</p></div><Link className={cn(buttonVariants({ variant: 'secondary' }), 'gap-2')} href="/product#code-graph">Explore the graph model <ArrowRightIcon data-icon="inline-end" /></Link></div></section>
  </MarketingPage>;
}
