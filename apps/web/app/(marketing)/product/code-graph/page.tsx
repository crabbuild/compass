import { ArrowRightIcon, CheckCircle2Icon, CircleDotDashedIcon, FileCode2Icon, NetworkIcon } from 'lucide-react';
import Link from 'next/link';

import { EvidenceDiagram, PipelineDiagram } from '@/components/diagrams';
import { FeatureGrid, MarketingPage, PageSection } from '@/components/marketing-page';
import { ProductionGraphExplorer } from '@/components/production-graph-explorer';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { buttonVariants } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import { pageMetadata } from '@/lib/site';

export const metadata = pageMetadata('Code graph', 'Build an inspectable graph of source entities and relationships while preserving identity, direction, multiplicity, and provenance.');

export default function CodeGraphPage() {
  return <MarketingPage eyebrow="Code graph" title="Make the relationships legible." description="Compass extracts structure from source files, resolves cross-file links, and publishes a graph with identity, direction, multiplicity, and provenance intact.">
      <PageSection eyebrow="Graph construction" title="Every stage earns its place in the map." description="The graph is the result of a bounded pipeline—not a decorative layer over a search index.">
        <FeatureGrid items={[{ eyebrow: '01 / discover', title: 'Scope the corpus', description: 'Deterministic discovery, ignore rules, and file classification define what belongs in the build.' }, { eyebrow: '02 / extract', title: 'Emit local facts', description: 'Per-file syntax facts carry source ranges and evidence into the project-level pipeline.' }, { eyebrow: '03 / resolve', title: 'Connect the project', description: 'Cross-file imports, calls, members, aliases, and identities are resolved with ambiguity preserved.' }]} />
      </PageSection>
      <PageSection eyebrow="Try a real graph" title="Explore dotenv before you change it." description="This is a compact snapshot of dotenv, the widely adopted Node.js environment loader. Drag the fixed layout, search for a symbol, click a relationship, or open the exact source line on GitHub.">
        <ProductionGraphExplorer />
      </PageSection>
      <section className="border-y border-border/70 bg-muted/25">
        <div className="mx-auto max-w-7xl px-5 py-20 lg:px-8 lg:py-28">
          <div className="flex flex-col gap-10">
            <PipelineDiagram />
            <div className="grid gap-8 lg:grid-cols-[0.82fr_1.18fr] lg:items-start">
              <div className="flex flex-col gap-6">
              <div>
                <p className="eyebrow">The artifact trail</p>
                <h2 className="mt-4 font-heading text-3xl font-semibold tracking-[-0.05em]">Follow the result back to the boundary that produced it.</h2>
                <p className="mt-4 text-base leading-7 text-muted-foreground">A missing edge is not silently filled in. A limit is not treated as an empty result. The stages make those decisions visible.</p>
              </div>
              </div>
              <div className="flex flex-col gap-3">
                <ArtifactRow title="Manifest" text="what entered the build" />
                <ArtifactRow title="Evidence" text="what a parser observed" />
                <ArtifactRow title="Relationships" text="what resolution could connect" />
                <ArtifactRow title="Snapshot" text="what passed validation" />
              </div>
            </div>
          </div>
        </div>
      </section>
      <PageSection eyebrow="What stays attached" title="The viewer is only the front door." description="Explore the graph visually, then hover or query the same entities and edges to see the details behind the shape.">
        <div className="grid gap-8 lg:grid-cols-[1.08fr_0.92fr] lg:items-center">
          <EvidenceDiagram />
          <Card className="border-border/80 bg-card/70 shadow-none">
            <CardHeader><CardTitle className="font-heading text-2xl tracking-[-0.045em]">Every edge can answer four follow-ups.</CardTitle></CardHeader>
            <CardContent className="flex flex-col gap-4 text-sm leading-7 text-muted-foreground">
              <span className="flex gap-3"><CheckCircle2Icon className="mt-1 size-4 shrink-0 text-primary" />What relation is this?</span>
              <span className="flex gap-3"><CheckCircle2Icon className="mt-1 size-4 shrink-0 text-primary" />Which direction should I traverse?</span>
              <span className="flex gap-3"><CheckCircle2Icon className="mt-1 size-4 shrink-0 text-primary" />Where did the source produce it?</span>
              <span className="flex gap-3"><CheckCircle2Icon className="mt-1 size-4 shrink-0 text-primary" />Is the result direct, inferred, or unresolved?</span>
            </CardContent>
          </Card>
        </div>
      </PageSection>
      <section className="border-y border-border/70 bg-muted/25"><div className="mx-auto flex max-w-7xl flex-col gap-8 px-5 py-20 lg:flex-row lg:items-center lg:justify-between lg:px-8 lg:py-28"><div className="flex max-w-2xl flex-col gap-4"><p className="eyebrow">Inspect the output</p><h2 className="font-heading text-3xl font-semibold tracking-[-0.05em]">Graph JSON, HTML, SVG, and more keep the result portable.</h2><p className="text-base leading-7 text-muted-foreground">Use the viewer for exploration, typed queries for automation, and exports when the graph needs to travel.</p></div><div className="flex flex-wrap gap-3 text-sm font-mono text-muted-foreground"><span className="inline-flex items-center gap-2 rounded-full border border-border bg-card px-3 py-2"><FileCode2Icon className="text-primary" /> graph.json</span><span className="inline-flex items-center gap-2 rounded-full border border-border bg-card px-3 py-2"><NetworkIcon className="text-primary" /> graph.html</span><span className="inline-flex items-center gap-2 rounded-full border border-border bg-card px-3 py-2"><CircleDotDashedIcon className="text-primary" /> graph.svg</span></div></div></section>
    <div className="mx-auto max-w-7xl px-5 py-16 lg:px-8"><Link className={cn(buttonVariants({ variant: 'outline' }), 'gap-2')} href="/docs/concepts/graph-model">Explore the graph model <ArrowRightIcon data-icon="inline-end" /></Link></div>
  </MarketingPage>;
}

function ArtifactRow({ title, text }: { title: string; text: string }) {
  return <div className="flex items-center justify-between gap-4 rounded-xl border border-border/80 bg-card/75 px-4 py-3"><span className="font-mono text-xs uppercase tracking-[0.12em] text-primary">{title}</span><span className="text-right text-sm text-muted-foreground">{text}</span></div>;
}
