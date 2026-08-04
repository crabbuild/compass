import Link from 'next/link';
import {
  ArrowRightIcon,
  BoxesIcon,
  BracesIcon,
  CheckCircle2Icon,
  CpuIcon,
  FileDiffIcon,
  GaugeIcon,
  GithubIcon,
  HardDriveIcon,
  LockKeyholeIcon,
  NetworkIcon,
  RouteIcon,
  SearchIcon,
  TerminalSquareIcon,
} from 'lucide-react';

import { HeroGraph } from '@/components/hero-graph';
import { ExportGallery } from '@/components/export-gallery';
import { InstallCommand } from '@/components/install-command';
import { PipelineDiagram } from '@/components/diagrams';
import { ProductionGraphExplorer } from '@/components/production-graph-explorer';
import { SectionHeading } from '@/components/section-heading';
import { Badge } from '@/components/ui/badge';
import { Button, buttonVariants } from '@/components/ui/button';
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card';
import { cn } from '@/lib/utils';

const evidence = [
  { icon: CpuIcon, value: 'native Rust', label: 'one executable, linked parsers' },
  { icon: HardDriveIcon, value: 'local first', label: 'no model or database required' },
  { icon: RouteIcon, value: 'traceable', label: 'source ranges and provenance' },
  { icon: GaugeIcon, value: 'bounded', label: 'explicit limits on graph work' },
];

const featureCards = [
  {
    icon: NetworkIcon,
    eyebrow: 'Map',
    title: 'See the shape of a system',
    description: 'Communities, architecture reports, and directed relationships turn an unfamiliar repository into a navigable surface.',
    href: '/product#code-graph',
    link: 'Explore code graphs',
  },
  {
    icon: FileDiffIcon,
    eyebrow: 'Compare',
    title: 'Know the blast radius',
    description: 'Follow reverse dependencies and compare graph realizations across exact Git commits before a change ships.',
    href: '/product#history',
    link: 'Trace historical change',
  },
  {
    icon: BracesIcon,
    eyebrow: 'Query',
    title: 'Ask exact questions',
    description: 'CompassQL gives scripts and assistants a deterministic, read-only way to ask structural questions with evidence.',
    href: '/product#compassql',
    link: 'Read about CompassQL',
  },
];

export default function HomePage() {
  return (
    <>
      <section className="relative isolate overflow-hidden border-b border-border/70">
        <div className="site-grid pointer-events-none absolute inset-0 -z-10 opacity-70" aria-hidden="true" />
        <div className="mx-auto grid max-w-7xl grid-cols-[minmax(0,1fr)] items-center gap-12 px-5 pb-20 pt-16 lg:grid-cols-[minmax(0,0.9fr)_minmax(0,1.1fr)] lg:gap-16 lg:px-8 lg:pb-28 lg:pt-24">
          <div className="flex min-w-0 w-full flex-col items-start">
            <Badge className="gap-2 rounded-full px-3 py-1 font-mono text-[0.68rem] uppercase tracking-[0.14em]" variant="outline">
              <span className="size-1.5 rounded-full bg-compass-amber" />
              Native code intelligence
            </Badge>
            <h1 className="mt-7 max-w-2xl font-heading text-[clamp(4rem,8vw,7.2rem)] font-semibold leading-[0.88] tracking-[-0.08em] text-foreground">
              <span className="block">See</span>
              <span className="block text-primary">what connects.</span>
            </h1>
            <p className="mt-7 max-w-xl text-lg leading-8 text-muted-foreground sm:text-xl">
              Compass turns a repository into a local, evidence-backed graph—so every change starts with context.
            </p>
            <div className="mt-9 flex flex-col gap-3 sm:flex-row sm:items-center">
              <Link className={cn(buttonVariants({ size: 'lg' }), 'gap-2 px-5')} href="/install">
                Install Compass
                <ArrowRightIcon data-icon="inline-end" />
              </Link>
              <Link className={cn(buttonVariants({ size: 'lg', variant: 'outline' }), 'px-5')} href="/docs">
                Read the docs
              </Link>
            </div>
            <div className="mt-7 flex w-full flex-col gap-2">
              <span className="font-mono text-[0.68rem] uppercase tracking-[0.14em] text-muted-foreground">Start at the terminal</span>
              <InstallCommand />
            </div>
          </div>

          <div className="hero-graph-shell min-w-0 w-full relative h-[29rem] overflow-hidden rounded-[1.5rem] border border-border/80 bg-card/75 shadow-[0_24px_80px_-40px_color-mix(in_oklch,var(--primary)_45%,transparent)] backdrop-blur-sm lg:h-[35rem]">
            <HeroGraph />
          </div>
        </div>
      </section>

      <section className="border-b border-border/70 bg-card/45">
        <div className="mx-auto grid max-w-7xl sm:grid-cols-2 lg:grid-cols-4">
          {evidence.map(({ icon: Icon, value, label }, index) => (
            <div
              className={cn(
                'flex min-h-28 items-center gap-4 px-5 py-5',
                index > 0 && 'border-t border-border/70',
                index === 1 && 'sm:border-t-0 sm:border-l',
                index === 3 && 'sm:border-l',
                'lg:border-t-0 lg:px-8',
                index > 0 && 'lg:border-l',
              )}
              key={value}
            >
              <span className="grid size-10 shrink-0 place-items-center rounded-xl border border-primary/15 bg-primary/[0.07] text-primary">
                <Icon aria-hidden="true" className="size-5" strokeWidth={1.8} />
              </span>
              <span className="flex min-w-0 flex-col gap-1">
                <span className="font-heading text-lg font-semibold tracking-[-0.03em]">{value}</span>
                <span className="text-sm leading-5 text-muted-foreground">{label}</span>
              </span>
            </div>
          ))}
        </div>
      </section>

      <ExportGallery />

      <section className="border-b border-border/70 bg-background" aria-labelledby="home-graph-title">
        <div className="mx-auto max-w-7xl px-5 py-24 lg:px-8 lg:py-32">
          <div className="flex flex-col gap-8 lg:flex-row lg:items-end lg:justify-between">
            <div className="max-w-2xl">
              <p className="eyebrow">Try a real codebase</p>
              <h2 id="home-graph-title" className="mt-4 max-w-2xl font-heading text-3xl font-semibold tracking-[-0.055em] sm:text-4xl">
                Explore the relationships before they become a change.
              </h2>
              <p className="mt-5 max-w-xl text-base leading-7 text-muted-foreground">
                Drag, search, and pin a symbol in a real <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-[0.85em] text-foreground">dotenv@17.4.2</code> snapshot. Every node and edge keeps its source path, line range, and relationship type attached.
              </p>
            </div>
            <div className="flex flex-col items-start gap-3 lg:items-end">
              <span className="inline-flex items-center gap-2 rounded-full border border-border/80 bg-card px-3 py-2 font-mono text-[0.65rem] uppercase tracking-[0.12em] text-muted-foreground">
                <NetworkIcon className="size-3.5 text-primary" aria-hidden="true" />
                25 nodes · 52 edges · local snapshot
              </span>
              <Link className="inline-flex items-center gap-2 text-sm font-medium text-primary" href="/product#code-graph">
                Open the full graph workspace <ArrowRightIcon data-icon="inline-end" />
              </Link>
            </div>
          </div>

          <div className="mt-12">
            <ProductionGraphExplorer />
          </div>
        </div>
      </section>

      <section className="mx-auto max-w-7xl px-5 py-24 lg:px-8 lg:py-32">
        <SectionHeading
          eyebrow="A smaller surface for a large system"
          title="The graph is useful because the evidence stays attached."
          description="Compass does more than draw nodes. It preserves direction, source anchors, multiplicity, and provenance while it builds a queryable snapshot."
        />
        <div className="mt-14 grid gap-5 lg:grid-cols-3">
          {featureCards.map((feature) => {
            const Icon = feature.icon;
            return (
              <Card className="group min-h-72 border-border/80 bg-card/70 shadow-none transition-transform duration-300 hover:-translate-y-1" key={feature.title}>
                <CardHeader className="gap-5">
                  <div className="flex size-11 items-center justify-center rounded-xl border border-border bg-muted/55 text-primary">
                    <Icon />
                  </div>
                  <div className="flex flex-col gap-2">
                    <span className="eyebrow">{feature.eyebrow}</span>
                    <CardTitle className="font-heading text-xl tracking-[-0.035em]">{feature.title}</CardTitle>
                  </div>
                </CardHeader>
                <CardContent className="flex h-full flex-col justify-between gap-7">
                  <CardDescription className="text-[0.95rem] leading-7">{feature.description}</CardDescription>
                  <Link className="inline-flex items-center gap-2 text-sm font-medium text-primary transition-[gap] group-hover:gap-3" href={feature.href}>
                    {feature.link}
                    <ArrowRightIcon data-icon="inline-end" />
                  </Link>
                </CardContent>
              </Card>
            );
          })}
        </div>
      </section>

      <section className="border-y border-border/70 bg-muted/25">
        <div className="mx-auto grid max-w-7xl gap-12 px-5 py-24 lg:grid-cols-[0.82fr_1.18fr] lg:items-center lg:px-8 lg:py-32">
          <div>
            <p className="eyebrow">How it works</p>
            <h2 className="mt-4 max-w-lg font-heading text-3xl font-semibold tracking-[-0.05em] sm:text-4xl">A source-driven pipeline you can inspect.</h2>
            <p className="mt-5 max-w-lg text-base leading-7 text-muted-foreground">
              Discovery, extraction, resolution, and analysis each have a clear boundary. The result is one coherent snapshot—not a black-box answer.
            </p>
            <Link className="mt-8 inline-flex items-center gap-2 text-sm font-medium text-primary" href="/docs/concepts/how-it-works">
              Read the pipeline guide <ArrowRightIcon data-icon="inline-end" />
            </Link>
          </div>
          <div className="grid overflow-hidden rounded-2xl border border-border/80 bg-card shadow-sm sm:grid-cols-2">
            <PipelineStep index="01" icon={SearchIcon} title="Discover" text="classify files + scope" detail="ignore rules define the corpus" artifact="manifest" />
            <PipelineStep index="02" icon={BoxesIcon} title="Extract" text="emit per-file facts" detail="syntax facts keep their source anchors" artifact="evidence" />
            <PipelineStep index="03" icon={NetworkIcon} title="Resolve" text="connect cross-file links" detail="ambiguity stays visible at the boundary" artifact="relationships" />
            <PipelineStep index="04" icon={CheckCircle2Icon} title="Publish" text="validate one snapshot" detail="complete artifacts land atomically" artifact="graph.json" />
          </div>
          <div className="lg:col-span-2">
            <PipelineDiagram />
          </div>
        </div>
      </section>

      <section className="mx-auto max-w-7xl px-5 py-24 lg:px-8 lg:py-32">
        <div className="grid gap-6 lg:grid-cols-[1.1fr_0.9fr]">
          <Card className="overflow-hidden border-border/80 bg-compass-canvas-deep text-foreground shadow-none">
            <CardHeader className="border-b border-border/70 px-6 py-6 sm:px-8">
              <div className="flex items-center justify-between gap-4">
                <div className="flex items-center gap-3">
                  <TerminalSquareIcon className="text-compass-amber" />
                  <CardTitle className="font-mono text-sm tracking-normal">CompassQL / exact path</CardTitle>
                </div>
                <Badge variant="outline">read-only</Badge>
              </div>
            </CardHeader>
            <CardContent className="grid gap-7 px-6 py-7 sm:px-8 sm:py-8 lg:grid-cols-[1fr_0.9fr]">
              <div className="flex flex-col gap-4 font-mono text-sm leading-7">
                <p className="text-muted-foreground"><span className="text-compass-amber">MATCH</span> (entry)-[:CALLS*1..4]-&gt;(target)</p>
                <p className="text-muted-foreground"><span className="text-compass-amber">WHERE</span> entry.name = <span className="text-foreground">&quot;CheckoutHandler&quot;</span></p>
                <p className="text-muted-foreground"><span className="text-compass-amber">RETURN</span> target.id, target.path</p>
                <p className="mt-2 text-xs text-muted-foreground">limit: 100 · path expansion: bounded · provenance: retained</p>
              </div>
              <div className="rounded-xl border border-border/80 bg-background/45 p-4 font-mono text-xs leading-6">
                <div className="mb-3 flex items-center justify-between border-b border-border/60 pb-3 text-muted-foreground"><span>RESULTS</span><span>3 rows</span></div>
                <p><span className="text-primary">01</span> PaymentGateway</p>
                <p><span className="text-primary">02</span> Inventory.reserve</p>
                <p><span className="text-primary">03</span> Session.authorize</p>
              </div>
            </CardContent>
          </Card>
          <Card className="border-border/80 bg-card/70 shadow-none">
            <CardHeader className="gap-4">
              <div className="flex size-11 items-center justify-center rounded-xl border border-border bg-muted/55 text-primary"><LockKeyholeIcon /></div>
              <div className="flex flex-col gap-2">
                <span className="eyebrow">Boundaries by default</span>
                <CardTitle className="font-heading text-2xl tracking-[-0.045em]">Local work stays local.</CardTitle>
              </div>
            </CardHeader>
            <CardContent className="flex flex-col gap-6">
              <p className="text-[0.95rem] leading-7 text-muted-foreground">Structural extraction and graph queries do not require Python, embeddings, model credentials, or runtime grammar downloads.</p>
              <div className="flex flex-col gap-3 border-t border-border/70 pt-5 font-mono text-xs text-muted-foreground">
                <span className="flex items-center gap-2"><CheckCircle2Icon className="text-primary" /> local source processing</span>
                <span className="flex items-center gap-2"><CheckCircle2Icon className="text-primary" /> explicit optional providers</span>
                <span className="flex items-center gap-2"><CheckCircle2Icon className="text-primary" /> bounded graph operations</span>
              </div>
              <Link className="inline-flex items-center gap-2 text-sm font-medium text-primary" href="/security">Read the security boundary <ArrowRightIcon data-icon="inline-end" /></Link>
            </CardContent>
          </Card>
        </div>
      </section>

      <section className="border-t border-border/70 bg-primary text-primary-foreground">
        <div className="mx-auto flex max-w-7xl flex-col gap-8 px-5 py-20 lg:flex-row lg:items-end lg:justify-between lg:px-8 lg:py-24">
          <div className="max-w-2xl">
            <p className="eyebrow text-primary-foreground/70">Make the next change with context</p>
            <h2 className="mt-4 max-w-3xl font-heading text-4xl font-semibold tracking-[-0.06em] sm:text-5xl">Ship the next change with context.</h2>
            <p className="mt-5 max-w-xl text-base leading-7 text-primary-foreground/75">Install Compass locally, build your first graph, and ask one question you could not answer from a file tree alone.</p>
          </div>
          <div className="flex flex-col gap-3 sm:flex-row">
            <Link className={cn(buttonVariants({ variant: 'secondary', size: 'lg' }), 'px-5')} href="/install">Install Compass</Link>
            <Link className={cn(buttonVariants({ variant: 'outline', size: 'lg' }), 'border-primary-foreground/30 bg-transparent text-primary-foreground hover:bg-primary-foreground/10 hover:text-primary-foreground')} href="https://github.com/crabbuild/compass" target="_blank" rel="noreferrer">
              <GithubIcon data-icon="inline-start" /> View on GitHub
            </Link>
          </div>
        </div>
      </section>
    </>
  );
}

function PipelineStep({ index, icon: Icon, title, text, detail, artifact }: { index: string; icon: typeof SearchIcon; title: string; text: string; detail: string; artifact: string }) {
  return (
    <div className="pipeline-step group flex min-h-44 flex-col justify-between p-6 sm:min-h-52 sm:p-7 lg:min-h-56">
      <div className="flex items-center justify-between font-mono text-xs text-muted-foreground"><span>{index}</span><Icon className="text-primary" /></div>
      <div className="flex flex-col gap-2">
        <span className="font-heading text-xl font-semibold tracking-[-0.04em]">{title}</span>
        <span className="font-mono text-xs text-muted-foreground">{text}</span>
        <span className="max-w-[18rem] text-sm leading-6 text-muted-foreground/90">{detail}</span>
        <span className="mt-1 inline-flex w-fit rounded-full border border-border/80 bg-muted/60 px-2.5 py-1 font-mono text-[0.62rem] uppercase tracking-[0.12em] text-primary">{artifact}</span>
      </div>
    </div>
  );
}
