import { ArrowRightIcon, CheckCircle2Icon, FileCode2Icon, GitBranchIcon, GitCommitHorizontalIcon, LockKeyholeIcon, NetworkIcon, SearchCodeIcon } from 'lucide-react';

import { EvidenceDiagram, PipelineDiagram } from '@/components/diagrams';
import { FeatureGrid, MarketingPage, PageSection } from '@/components/marketing-page';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { cn } from '@/lib/utils';
import { buttonVariants } from '@/components/ui/button';
import { pageMetadata } from '@/lib/site';
import Link from 'next/link';

export const metadata = pageMetadata('Product', 'Explore Compass code graphs, CompassQL, immutable history, integrations, atomic builds, and portable outputs.');

const features = [
  { eyebrow: 'Map', title: 'Code graphs with provenance', description: 'Find entities, relationships, communities, source ranges, and evidence without leaving your local workspace.', href: '/product/code-graph' },
  { eyebrow: 'Query', title: 'CompassQL for exact questions', description: 'Use a bounded, read-only openCypher subset for repeatable structural checks and automation.', href: '/product/compassql' },
  { eyebrow: 'History', title: 'Immutable graph realizations', description: 'Compare exact commits without rewriting the historical snapshot you are trying to understand.', href: '/product/history' },
  { eyebrow: 'Assistants', title: 'Focused context for tools', description: 'Serve compact graph answers through native skills, hooks, MCP, and editor workflows.', href: '/integrations' },
  { eyebrow: 'Operations', title: 'Atomic and incremental', description: 'Reuse compatible extraction work and publish complete artifact sets with explicit failure boundaries.' },
  { eyebrow: 'Exports', title: 'Portable outputs', description: 'Move between JSON, HTML, SVG, GraphML, Wiki, Obsidian, and other graph representations.' },
];

const principles = [
  [NetworkIcon, 'Relationships stay directional', 'CALLS, IMPORTS_FROM, USES, and CONTAINS retain their direction and multiplicity.'],
  [SearchCodeIcon, 'Answers point back to source', 'Source anchors and provenance make an edge something you can inspect—not just something you trust.'],
  [LockKeyholeIcon, 'Optional boundaries stay explicit', 'Local structural work never silently becomes a network or credential operation.'],
];

export default function ProductPage() {
  return (
    <MarketingPage eyebrow="Product surface" title="A map you can interrogate, not a screenshot you admire." description="Compass builds a native graph of your codebase and project artifacts, then gives you practical ways to explore, query, compare, and share it.">
      <PageSection eyebrow="One product, several ways in" title="Start with the question in front of you." description="Compass is organized around the work developers actually do when a repository is too large to hold in their head.">
        <FeatureGrid items={features} />
      </PageSection>
      <section className="border-y border-border/70 bg-muted/25">
        <div className="mx-auto grid max-w-7xl gap-8 px-5 py-20 lg:grid-cols-[1.08fr_0.92fr] lg:items-center lg:px-8 lg:py-28">
          <EvidenceDiagram />
          <div className="flex flex-col gap-7">
            <div>
              <p className="eyebrow">The evidence contract</p>
              <h2 className="mt-4 font-heading text-3xl font-semibold tracking-[-0.05em] sm:text-4xl">A relationship is a claim you can follow.</h2>
              <p className="mt-5 max-w-xl text-base leading-7 text-muted-foreground">Compass keeps the useful details beside the line: what the relationship means, which direction it travels, where it appeared, and how the extractor knows about it.</p>
            </div>
            <div className="grid gap-3 sm:grid-cols-2">
              {['stable identity', 'direction + multiplicity', 'source range', 'provenance state'].map((item) => (
                <div className="flex items-center gap-2 rounded-lg border border-border/80 bg-card/75 px-3 py-3 font-mono text-xs text-muted-foreground" key={item}>
                  <CheckCircle2Icon className="size-4 text-primary" />
                  {item}
                </div>
              ))}
            </div>
            <Link className="inline-flex items-center gap-2 text-sm font-medium text-primary" href="/docs/concepts/universal-semantic-evidence">Read the evidence model <ArrowRightIcon data-icon="inline-end" /></Link>
          </div>
        </div>
      </section>
      <PageSection eyebrow="From source to snapshot" title="The build is a visible sequence, not a black box." description="Each boundary has an owner, an artifact, and a failure mode you can reason about. That makes the graph easier to trust and easier to debug.">
        <PipelineDiagram />
        <div className="mt-6 grid gap-4 md:grid-cols-3">
          <SessionCard index="01" title="Scope" text="Discovery applies ignore rules and records the repository manifest." />
          <SessionCard index="02" title="Resolve" text="Per-file facts become directed cross-file relationships with ambiguity intact." />
          <SessionCard index="03" title="Publish" text="Validation materializes one complete graph.json snapshot for every surface." />
        </div>
      </PageSection>
      <section className="border-y border-border/70 bg-muted/25">
        <div className="mx-auto max-w-7xl px-5 py-20 lg:px-8 lg:py-28">
          <div className="grid gap-5 lg:grid-cols-3">
            {principles.map(([Icon, title, text]) => {
              const PrincipleIcon = Icon as typeof NetworkIcon;
              return <Card className="border-border/80 bg-card/70 shadow-none" key={title as string}><CardHeader className="gap-4"><PrincipleIcon className="text-primary" /><CardTitle className="font-heading text-xl tracking-[-0.04em]">{title as string}</CardTitle></CardHeader><CardContent><p className="text-sm leading-7 text-muted-foreground">{text as string}</p></CardContent></Card>;
            })}
          </div>
        </div>
      </section>
      <PageSection eyebrow="Built to be inspected" title="The shape of the system is part of the answer." description="Compass exposes the boundaries between discovery, extraction, resolution, analysis, and publication so you can reason about the result. ">
        <div className="grid gap-5 lg:grid-cols-3">
          <MetricCard icon={FileCode2Icon} value="per-file" label="facts before project-wide resolution" />
          <MetricCard icon={GitBranchIcon} value="directed" label="relationships with source anchors" />
          <MetricCard icon={GitCommitHorizontalIcon} value="immutable" label="historical realizations for exact commits" />
        </div>
      </PageSection>
      <section className="border-t border-border/70 bg-card/45">
        <div className="mx-auto grid max-w-7xl gap-8 px-5 py-20 lg:grid-cols-[0.8fr_1.2fr] lg:items-center lg:px-8 lg:py-28">
          <div>
            <p className="eyebrow">A practical starting point</p>
            <h2 className="mt-4 font-heading text-3xl font-semibold tracking-[-0.05em] sm:text-4xl">Choose the smallest surface that answers the question.</h2>
            <p className="mt-5 text-base leading-7 text-muted-foreground">Use the viewer when you need orientation, CompassQL when you need a repeatable question, and history when the change itself is the unknown.</p>
          </div>
          <div className="grid gap-4 sm:grid-cols-3">
            <QuickLink title="Explore" text="Visual graph + hover evidence" href="/product/code-graph" />
            <QuickLink title="Query" text="Bounded structural contract" href="/product/compassql" />
            <QuickLink title="Compare" text="Exact revision realizations" href="/product/history" />
          </div>
        </div>
      </section>
    </MarketingPage>
  );
}

function SessionCard({ index, title, text }: { index: string; title: string; text: string }) {
  return <Card className="border-border/80 bg-card/70 shadow-none"><CardContent className="flex gap-4 p-5"><span className="font-mono text-xs text-primary">{index}</span><div className="flex flex-col gap-1"><span className="font-heading text-lg font-semibold tracking-[-0.035em]">{title}</span><span className="text-sm leading-6 text-muted-foreground">{text}</span></div></CardContent></Card>;
}

function QuickLink({ title, text, href }: { title: string; text: string; href: string }) {
  return <Link className={cn(buttonVariants({ variant: 'outline' }), 'h-auto min-h-28 items-start justify-between gap-4 p-4 text-left')} href={href}><span className="flex flex-col gap-2"><span className="font-heading text-base font-semibold tracking-[-0.03em]">{title}</span><span className="text-xs leading-5 text-muted-foreground">{text}</span></span><ArrowRightIcon className="mt-0.5 size-4 shrink-0 text-primary" /></Link>;
}

function MetricCard({ icon: Icon, value, label }: { icon: typeof FileCode2Icon; value: string; label: string }) {
  return <Card className="border-border/80 bg-card/70 shadow-none"><CardContent className="flex min-h-44 flex-col justify-between gap-8 p-6"><span className="grid size-11 place-items-center rounded-xl border border-primary/15 bg-primary/[0.07] text-primary"><Icon aria-hidden="true" className="size-5" strokeWidth={1.8} /></span><div className="flex flex-col gap-2"><span className="font-heading text-3xl font-semibold tracking-[-0.06em]">{value}</span><span className="max-w-[15rem] text-sm leading-6 text-muted-foreground">{label}</span></div></CardContent></Card>;
}
