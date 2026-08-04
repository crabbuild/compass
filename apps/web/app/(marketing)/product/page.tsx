import {
  ArrowRightIcon,
  BracesIcon,
  CheckCircle2Icon,
  CircleAlertIcon,
  CircleDotDashedIcon,
  FileCode2Icon,
  GitBranchIcon,
  GitCommitHorizontalIcon,
  GitCompareArrowsIcon,
  GaugeIcon,
  LockKeyholeIcon,
  NetworkIcon,
  SearchCodeIcon,
  ShieldCheckIcon,
  TerminalSquareIcon,
} from 'lucide-react';
import Link from 'next/link';

import {
  EvidenceDiagram,
  HistoryComparisonDiagram,
  ImpactPathDiagram,
  PipelineDiagram,
} from '@/components/diagrams';
import { MarketingPage, PageSection } from '@/components/marketing-page';
import { ProductionGraphExplorer } from '@/components/production-graph-explorer';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { buttonVariants } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import { pageMetadata } from '@/lib/site';

export const metadata = pageMetadata(
  'Product',
  'Explore Compass in one place: source graphs, CompassQL, versioned history, integrations, and portable evidence.',
);

const surfaces = [
  {
    icon: NetworkIcon,
    eyebrow: 'Visualize',
    title: 'Code graph',
    description: 'Explore entities, relationships, communities, source ranges, and evidence in one local snapshot.',
    href: '#code-graph',
  },
  {
    icon: BracesIcon,
    eyebrow: 'Query',
    title: 'CompassQL',
    description: 'Ask bounded, read-only structural questions with stable ordering and explicit diagnostics.',
    href: '#compassql',
  },
  {
    icon: GitCompareArrowsIcon,
    eyebrow: 'Compare',
    title: 'Versioned history',
    description: 'Compare exact graph realizations across commits without rewriting the historical answer.',
    href: '#history',
  },
];

const graphPrinciples = [
  ['stable identity', 'A node means the same thing across the viewer, query output, and exports.'],
  ['direction + multiplicity', 'CALLS, IMPORTS, USES, and CONTAINS keep their direction and count.'],
  ['source range', 'Every useful relationship can lead back to the file and line that produced it.'],
  ['provenance state', 'Direct, inferred, unresolved, and ambiguous results stay distinguishable.'],
];

export default function ProductPage() {
  return (
    <MarketingPage
      eyebrow="Product"
      title="One local graph. Every useful question."
      description="Compass turns a repository into an inspectable, evidence-backed graph, then gives you the smallest surface for exploring, querying, comparing, and sharing what you found."
    >
      <PageSection
        id="surfaces"
        eyebrow="Product surfaces"
        title="Move from shape to reason without changing tools."
        description="Start with the question in front of you. Each surface uses the same graph model, so the context you discover in one place remains useful in the next."
      >
        <div className="grid gap-5 md:grid-cols-3">
          {surfaces.map((surface) => <SurfaceCard key={surface.title} {...surface} />)}
        </div>
      </PageSection>

      <PageSection
        id="code-graph"
        eyebrow="Code graph"
        title="Explore the relationships before you edit."
        description="The viewer keeps a fixed layout, searchable symbols, relationship types, and source anchors in the same working set. Try a real dotenv snapshot, then open the full source line when a node earns your attention."
      >
        <ProductionGraphExplorer />
        <div className="mt-6 grid gap-4 md:grid-cols-3">
          <ProductSignal icon={SearchCodeIcon} title="Orient" text="Find communities, entry points, and the symbols that hold the system together." />
          <ProductSignal icon={NetworkIcon} title="Inspect" text="Hover a node or edge to keep identity, direction, and evidence in view." />
          <ProductSignal icon={FileCode2Icon} title="Navigate" text="Open the exact source path and line instead of losing the thread in a file tree." />
        </div>
      </PageSection>

      <section className="border-y border-border/70 bg-muted/25">
        <div className="mx-auto max-w-7xl px-5 py-20 lg:px-8 lg:py-28">
          <div className="grid gap-10 lg:grid-cols-[1.1fr_0.9fr] lg:items-center">
            <PipelineDiagram />
            <div className="flex flex-col gap-6">
              <div>
                <p className="eyebrow">From source to snapshot</p>
                <h2 className="mt-4 font-heading text-3xl font-semibold tracking-[-0.05em] sm:text-4xl">The build is a visible sequence, not a black box.</h2>
                <p className="mt-5 text-base leading-7 text-muted-foreground">Discovery, extraction, resolution, and publication each have an owner, an artifact, and a failure mode you can reason about.</p>
              </div>
              <div className="grid gap-3 sm:grid-cols-2">
                <BuildSignal icon={FileCode2Icon} title="per-file facts" text="Evidence starts close to the source." />
                <BuildSignal icon={GitBranchIcon} title="directed links" text="Resolution preserves ambiguity." />
                <BuildSignal icon={ShieldCheckIcon} title="validated output" text="Incomplete artifacts do not publish." />
                <BuildSignal icon={LockKeyholeIcon} title="local boundary" text="Optional network work stays explicit." />
              </div>
            </div>
          </div>
          <div className="mt-10 grid gap-8 lg:grid-cols-[1.1fr_0.9fr] lg:items-center">
            <EvidenceDiagram />
            <div className="flex flex-col gap-5">
              <p className="eyebrow">The evidence contract</p>
              <h2 className="font-heading text-3xl font-semibold tracking-[-0.05em] sm:text-4xl">A relationship is a claim you can follow.</h2>
              <p className="text-base leading-7 text-muted-foreground">Compass keeps the useful details beside the line: what the relationship means, which direction it travels, where it appeared, and how the extractor knows about it.</p>
              <div className="grid gap-3 sm:grid-cols-2">
                {graphPrinciples.map(([label, text]) => <div className="rounded-lg border border-border/80 bg-card/75 p-3" key={label}><span className="font-mono text-[0.65rem] uppercase tracking-[0.12em] text-primary">{label}</span><p className="mt-1 text-sm leading-6 text-muted-foreground">{text}</p></div>)}
              </div>
              <Link className="inline-flex items-center gap-2 text-sm font-medium text-primary" href="/docs/concepts/universal-semantic-evidence">Read the evidence model <ArrowRightIcon data-icon="inline-end" /></Link>
            </div>
          </div>
        </div>
      </section>

      <PageSection
        id="compassql"
        eyebrow="CompassQL"
        title="Ask structural questions with a contract."
        description="CompassQL is a deterministic, bounded, read-only query surface for finding paths, relationships, and impact without inventing meaning."
      >
        <div className="grid gap-6 lg:grid-cols-[1.1fr_0.9fr]">
          <Card className="overflow-hidden border-border/80 bg-compass-canvas-deep shadow-none">
            <CardHeader className="border-b border-border/70"><CardTitle className="flex items-center gap-3 font-mono text-sm tracking-normal"><BracesIcon className="text-compass-amber" /> impact.cql</CardTitle></CardHeader>
            <CardContent className="p-6 font-mono text-sm leading-8">
              <p><span className="text-compass-amber">MATCH</span> (changed)-[:CALLS|IMPORTS_FROM*1..3]-&gt;(affected)</p>
              <p><span className="text-compass-amber">WHERE</span> changed.source_file = <span className="text-primary">&quot;src/payments.rs&quot;</span></p>
              <p><span className="text-compass-amber">RETURN</span> affected.id, affected.source</p>
              <p><span className="text-compass-amber">ORDER BY</span> affected.id</p>
              <p><span className="text-muted-foreground">LIMIT 100</span></p>
            </CardContent>
          </Card>
          <Card className="border-border/80 bg-card/70 shadow-none">
            <CardHeader><CardTitle className="font-heading text-xl tracking-[-0.04em]">Readable input. Stable output.</CardTitle></CardHeader>
            <CardContent className="flex flex-col gap-4 text-sm leading-7 text-muted-foreground">
              <span className="flex gap-2"><CheckCircle2Icon className="mt-1 shrink-0 text-primary" /> deterministic ordering</span>
              <span className="flex gap-2"><CheckCircle2Icon className="mt-1 shrink-0 text-primary" /> bounded path expansion</span>
              <span className="flex gap-2"><CheckCircle2Icon className="mt-1 shrink-0 text-primary" /> explicit unsupported syntax</span>
              <span className="flex gap-2"><CheckCircle2Icon className="mt-1 shrink-0 text-primary" /> JSON and JSONL output</span>
            </CardContent>
          </Card>
        </div>
        <div className="mt-8 grid gap-4 sm:grid-cols-3">
          <QueryClause icon={BracesIcon} label="MATCH" text="choose a relationship and direction" />
          <QueryClause icon={GaugeIcon} label="WHERE" text="narrow the corpus with a predicate" />
          <QueryClause icon={TerminalSquareIcon} label="RETURN" text="select stable fields and anchors" />
        </div>
        <div className="mt-10"><ImpactPathDiagram /></div>
        <div className="mt-6 grid gap-4 md:grid-cols-3">
          <QuerySignal icon={GaugeIcon} title="bounded" text="depth and result limits are part of the contract" />
          <QuerySignal icon={CheckCircle2Icon} title="deterministic" text="stable ordering makes output repeatable" />
          <QuerySignal icon={CircleAlertIcon} title="honest" text="unsupported or ambiguous results stay explicit" />
        </div>
        <div className="mt-8 flex flex-wrap items-center justify-between gap-4 rounded-xl border border-border/80 bg-card/70 p-5">
          <div><span className="eyebrow">Language contract</span><p className="mt-2 text-sm leading-6 text-muted-foreground">Read the supported syntax, bounds, and diagnostics before you automate it.</p></div>
          <Link className={cn(buttonVariants({ variant: 'outline' }), 'gap-2')} href="/docs/COMPASSQL">Read CompassQL docs <ArrowRightIcon data-icon="inline-end" /></Link>
        </div>
      </PageSection>

      <PageSection
        id="history"
        eyebrow="Versioned history"
        title="Compare two code graphs, not just two source diffs."
        description="Choose exact Git revisions, identify the immutable graph behind each one, and see the nodes, relationships, and evidence that changed."
      >
        <HistoryComparisonDiagram />
        <div className="mt-6 grid gap-4 md:grid-cols-3">
          <HistorySignal icon={GitCommitHorizontalIcon} title="commit" text="Bind the question to an exact revision." />
          <HistorySignal icon={GitCompareArrowsIcon} title="compare" text="Diff graph structure and source evidence." />
          <HistorySignal icon={ShieldCheckIcon} title="preserve" text="Keep published realizations immutable." />
        </div>
        <div className="mt-8 grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
          <HistoryStep title="Select" text="Choose exact parent and current revisions." />
          <HistoryStep title="Materialize" text="Build or reuse each compatible realization." />
          <HistoryStep title="Compare" text="Classify topology and evidence changes." />
          <HistoryStep title="Inspect" text="Jump from a changed edge to its source anchor." />
        </div>
      </PageSection>

      <section className="border-y border-border/70 bg-muted/25" id="integrations">
        <div className="mx-auto max-w-7xl px-5 py-20 lg:px-8 lg:py-28">
          <div className="max-w-2xl">
            <p className="eyebrow">Surfaces that fit the work</p>
            <h2 className="mt-4 font-heading text-3xl font-semibold tracking-[-0.05em] sm:text-4xl">Use the graph where the next decision happens.</h2>
            <p className="mt-5 text-base leading-7 text-muted-foreground">Keep exploration in the editor, automation in CI, focused context in MCP, and artifacts portable when the graph needs to travel.</p>
          </div>
          <div className="mt-10 grid gap-4 md:grid-cols-2 lg:grid-cols-4">
            <SurfaceLink icon={FileCode2Icon} label="VS Code" text="Inspect source and graph together." href="/integrations#editor" />
            <SurfaceLink icon={NetworkIcon} label="MCP + skills" text="Return focused context to tools." href="/integrations#assistants" />
            <SurfaceLink icon={GitBranchIcon} label="CLI + CI" text="Run bounded checks in automation." href="/integrations#automation" />
            <SurfaceLink icon={CircleDotDashedIcon} label="Graph formats" text="Export JSON, HTML, SVG, and GraphML." href="/integrations#exports" />
          </div>
        </div>
      </section>

      <section className="border-t border-border/70 bg-primary text-primary-foreground">
        <div className="mx-auto flex max-w-7xl flex-col gap-7 px-5 py-16 lg:flex-row lg:items-center lg:justify-between lg:px-8 lg:py-20">
          <div className="max-w-2xl"><p className="eyebrow text-primary-foreground/70">A practical first move</p><h2 className="mt-3 font-heading text-3xl font-semibold tracking-[-0.05em] sm:text-4xl">Give the codebase a bearing.</h2><p className="mt-4 text-base leading-7 text-primary-foreground/75">Install Compass locally, build one graph, and answer a question you could not answer from a file tree alone.</p></div>
          <div className="flex flex-col gap-3 sm:flex-row"><Link className={cn(buttonVariants({ variant: 'secondary', size: 'lg' }), 'gap-2')} href="/install">Install Compass <ArrowRightIcon data-icon="inline-end" /></Link><Link className={cn(buttonVariants({ variant: 'outline', size: 'lg' }), 'border-primary-foreground/30 bg-transparent text-primary-foreground hover:bg-primary-foreground/10 hover:text-primary-foreground')} href="/docs">Read the docs</Link></div>
        </div>
      </section>
    </MarketingPage>
  );
}

function SurfaceCard({ icon: Icon, eyebrow, title, description, href }: { icon: typeof NetworkIcon; eyebrow: string; title: string; description: string; href: string }) {
  return <Link className="group flex min-h-52 flex-col gap-6 rounded-2xl border border-border/80 bg-card/70 p-6 shadow-none transition-transform duration-300 hover:-translate-y-1 hover:border-primary/30" href={href}><span className="grid size-11 place-items-center rounded-xl border border-primary/15 bg-primary/[0.07] text-primary"><Icon aria-hidden="true" className="size-5" strokeWidth={1.8} /></span><span className="flex flex-col gap-2"><span className="eyebrow">{eyebrow}</span><span className="font-heading text-xl font-semibold tracking-[-0.04em]">{title}</span><span className="text-sm leading-6 text-muted-foreground">{description}</span></span><ArrowRightIcon className="mt-auto size-4 text-primary transition-transform group-hover:translate-x-1" /> </Link>;
}

function ProductSignal({ icon: Icon, title, text }: { icon: typeof NetworkIcon; title: string; text: string }) {
  return <Card className="border-border/80 bg-card/70 shadow-none"><CardContent className="flex min-h-36 flex-col justify-between gap-5 p-5"><Icon className="size-5 text-primary" /><div><p className="font-heading text-lg font-semibold tracking-[-0.035em]">{title}</p><p className="mt-1 text-sm leading-6 text-muted-foreground">{text}</p></div></CardContent></Card>;
}

function BuildSignal({ icon: Icon, title, text }: { icon: typeof FileCode2Icon; title: string; text: string }) {
  return <div className="flex items-start gap-3 rounded-xl border border-border/80 bg-card/70 p-4"><Icon className="mt-0.5 size-4 shrink-0 text-primary" /><div><span className="font-mono text-[0.65rem] uppercase tracking-[0.12em] text-primary">{title}</span><p className="mt-1 text-sm leading-6 text-muted-foreground">{text}</p></div></div>;
}

function QueryClause({ icon: Icon, label, text }: { icon: typeof BracesIcon; label: string; text: string }) {
  return <div className="flex min-h-32 flex-col gap-4 rounded-xl border border-border/80 bg-card/75 p-5"><Icon className="size-5 text-primary" /><span className="font-mono text-xs uppercase tracking-[0.12em] text-primary">{label}</span><span className="text-sm leading-6 text-muted-foreground">{text}</span></div>;
}

function QuerySignal({ icon: Icon, title, text }: { icon: typeof GaugeIcon; title: string; text: string }) {
  return <div className="flex items-start gap-3 rounded-xl border border-border/80 bg-card/70 p-5"><Icon className="mt-1 size-4 shrink-0 text-primary" /><div className="flex flex-col gap-1"><span className="font-mono text-xs uppercase tracking-[0.12em] text-primary">{title}</span><span className="text-sm leading-6 text-muted-foreground">{text}</span></div></div>;
}

function HistorySignal({ icon: Icon, title, text }: { icon: typeof GitCommitHorizontalIcon; title: string; text: string }) {
  return <div className="flex items-start gap-4 rounded-xl border border-border/80 bg-card/70 p-5"><Icon className="mt-1 size-4 shrink-0 text-primary" /><div className="flex flex-col gap-1"><span className="font-mono text-sm text-primary">{title}</span><span className="text-sm leading-6 text-muted-foreground">{text}</span></div></div>;
}

function HistoryStep({ title, text }: { title: string; text: string }) {
  return <Card className="border-border/80 bg-card/70 shadow-none"><CardContent className="flex min-h-32 flex-col justify-between gap-5 p-5"><span className="font-mono text-xs text-primary">{title.toUpperCase()}</span><span className="text-sm leading-6 text-muted-foreground">{text}</span></CardContent></Card>;
}

function SurfaceLink({ icon: Icon, label, text, href }: { icon: typeof NetworkIcon; label: string; text: string; href: string }) {
  return <Link className="group flex min-h-36 flex-col justify-between gap-5 rounded-xl border border-border/80 bg-card/75 p-5 transition-colors hover:border-primary/30" href={href}><span className="flex items-center justify-between gap-3"><Icon className="size-5 text-primary" /><ArrowRightIcon className="size-4 text-muted-foreground transition-transform group-hover:translate-x-1 group-hover:text-primary" /></span><span><span className="font-heading text-lg font-semibold tracking-[-0.035em]">{label}</span><span className="mt-1 block text-sm leading-6 text-muted-foreground">{text}</span></span></Link>;
}
