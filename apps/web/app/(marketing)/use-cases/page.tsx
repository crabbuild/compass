import {
  ArrowRightIcon,
  BotIcon,
  CheckCircle2Icon,
  CircleAlertIcon,
  CompassIcon,
  FileCode2Icon,
  GitBranchIcon,
  GitCompareArrowsIcon,
  NetworkIcon,
  SearchCodeIcon,
  ShieldCheckIcon,
} from 'lucide-react';
import Link from 'next/link';

import {
  AssistantContextDiagram,
  EvidenceDiagram,
  HistoryComparisonDiagram,
  ImpactPathDiagram,
} from '@/components/diagrams';
import { MarketingPage, PageSection } from '@/components/marketing-page';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { buttonVariants } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import { pageMetadata } from '@/lib/site';

export const metadata = pageMetadata(
  'Use cases',
  'Use Compass to explore unfamiliar codebases, review change impact, focus assistant context, and understand system evolution.',
);

const workflows = [
  {
    icon: CompassIcon,
    eyebrow: 'Explore',
    title: 'Read an unfamiliar codebase',
    description: 'Find communities, entry points, and surprising connections before you start editing.',
    href: '#exploration',
  },
  {
    icon: CircleAlertIcon,
    eyebrow: 'Impact',
    title: 'Estimate change blast radius',
    description: 'Traverse incoming relationships from a changed symbol and keep the path evidence attached.',
    href: '#impact',
  },
  {
    icon: BotIcon,
    eyebrow: 'Assist',
    title: 'Give tools focused context',
    description: 'Return compact graph answers to assistants, editors, and automation instead of a context dump.',
    href: '#assistants',
  },
  {
    icon: GitCompareArrowsIcon,
    eyebrow: 'History',
    title: 'Understand evolution',
    description: 'Compare graph realizations across exact revisions when a source diff hides the topology.',
    href: '#history',
  },
];

export default function UseCasesPage() {
  return (
    <MarketingPage
      eyebrow="Use cases"
      title="Turn uncertainty into a smaller question."
      description="Compass gives each kind of repository uncertainty a practical surface: orient in the graph, trace impact, focus a tool, or compare how the system evolved."
    >
      <PageSection
        id="workflows"
        eyebrow="Developer workflows"
        title="Start from the work, not the feature list."
        description="Choose the question that is blocking the next decision. The workflow pages are now one connected guide, so you can move between them without resetting context."
      >
        <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-4">
          {workflows.map((workflow) => <WorkflowCard key={workflow.title} {...workflow} />)}
        </div>
      </PageSection>

      <PageSection
        id="exploration"
        eyebrow="Explore"
        title="Build a mental model before you build a plan."
        description="Exploration is not about reading every file. It is about choosing a small, defensible path through the system and knowing where that path came from."
      >
        <div className="grid gap-8 lg:grid-cols-[1.08fr_0.92fr] lg:items-center">
          <EvidenceDiagram />
          <Card className="border-border/80 bg-card/70 shadow-none">
            <CardHeader><CardTitle className="font-heading text-2xl tracking-[-0.045em]">Leave with more than a picture.</CardTitle></CardHeader>
            <CardContent className="flex flex-col gap-4 text-sm leading-7 text-muted-foreground">
              <Outcome icon={NetworkIcon} text="A shortlist of meaningful entry points." />
              <Outcome icon={SearchCodeIcon} text="A path you can explain to a teammate." />
              <Outcome icon={FileCode2Icon} text="Source anchors for the next edit." />
              <Outcome icon={CheckCircle2Icon} text="A repeatable question for the next session." />
            </CardContent>
          </Card>
        </div>
        <div className="mt-8 grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
          <WorkflowStep title="Open" text="Load the local graph and scan communities." />
          <WorkflowStep title="Hover" text="Inspect node identity, path, and kind." />
          <WorkflowStep title="Trace" text="Follow one directed edge into the code." />
          <WorkflowStep title="Record" text="Save the query and source anchors." />
        </div>
      </PageSection>

      <section id="impact" className="scroll-mt-24 border-y border-border/70 bg-muted/25">
        <div className="mx-auto max-w-7xl px-5 py-20 lg:px-8 lg:py-28">
          <div className="max-w-2xl">
            <p className="eyebrow">Impact</p>
            <h2 className="mt-4 font-heading text-3xl font-semibold tracking-[-0.05em] sm:text-4xl">See what a change can reach before it reaches production.</h2>
            <p className="mt-5 text-base leading-7 text-muted-foreground">Start at a symbol, walk directed incoming relationships, and keep the evidence for every affected target in view. A useful blast radius is bounded and explainable.</p>
          </div>
          <div className="mt-10"><ImpactPathDiagram /></div>
          <div className="mt-6 grid gap-4 md:grid-cols-2 lg:grid-cols-4">
            <ImpactCheck title="Start" text="Exact symbol, path, or node id." />
            <ImpactCheck title="Direction" text="Incoming relationships for reachability." />
            <ImpactCheck title="Bounds" text="Depth, fan-out, and result limits." />
            <ImpactCheck title="Evidence" text="Edge type, anchor, and provenance." />
          </div>
          <div className="mt-8 grid gap-8 lg:grid-cols-[0.9fr_1.1fr] lg:items-center">
            <div>
              <p className="eyebrow">A reviewable blast radius</p>
              <h3 className="mt-4 font-heading text-2xl font-semibold tracking-[-0.045em] sm:text-3xl">Separate reachable from merely possible.</h3>
              <p className="mt-4 text-base leading-7 text-muted-foreground">Keep the graph result next to the diff. Reviewers can inspect the edge, jump to the source range, and decide whether the path is actionable.</p>
            </div>
            <Card className="overflow-hidden border-border/80 bg-compass-canvas-deep shadow-none"><CardHeader className="border-b border-border/70"><CardTitle className="font-mono text-sm tracking-normal">impact.json</CardTitle></CardHeader><CardContent className="p-6 font-mono text-xs leading-7 text-muted-foreground"><p><span className="text-primary">path</span>: payments.rs → Gateway → Checkout</p><p><span className="text-primary">edge</span>: CALLS · direct · line 42</p><p><span className="text-primary">bound</span>: depth 1..3 · limit 100</p><p><span className="text-primary">status</span>: reviewable · provenance retained</p></CardContent></Card>
          </div>
        </div>
      </section>

      <PageSection
        id="assistants"
        eyebrow="Assist"
        title="Give assistants a map, not a context dump."
        description="Expose focused graph queries through native skills, hooks, MCP, and editor workflows so tools can ask smaller, better-scoped questions."
      >
        <AssistantContextDiagram />
        <div className="mt-8 grid gap-5 lg:grid-cols-3">
          <AssistantStep title="Ask" text="Resolve the user’s question into a path, impact, or relationship query." />
          <AssistantStep title="Bound" text="Apply explicit depth, limit, and provider boundaries before execution." />
          <AssistantStep title="Return" text="Keep identities, anchors, diagnostics, and provenance in the answer." />
        </div>
        <div className="mt-8 grid gap-8 lg:grid-cols-[1.08fr_0.92fr] lg:items-center">
          <Card className="overflow-hidden border-border/80 bg-compass-canvas-deep shadow-none"><CardHeader className="border-b border-border/70"><CardTitle className="font-mono text-sm tracking-normal">assistant-context.json</CardTitle></CardHeader><CardContent className="p-6 font-mono text-xs leading-7 text-muted-foreground"><p><span className="text-primary">question</span>: what calls CheckoutHandler?</p><p><span className="text-primary">paths</span>: 3 · depth: 1..4</p><p><span className="text-primary">answer</span>: PaymentGateway, Inventory.reserve</p><p><span className="text-primary">evidence</span>: source anchors + direct edges</p></CardContent></Card>
          <div><p className="eyebrow">Integration choices</p><h3 className="mt-4 font-heading text-2xl font-semibold tracking-[-0.045em] sm:text-3xl">Fit the boundary to the tool.</h3><p className="mt-4 text-base leading-7 text-muted-foreground">Use the CLI in scripts, MCP for a local tool boundary, native skills for repeatable workflows, or the shared viewer when a human needs to inspect the answer.</p><div className="mt-6 flex flex-wrap gap-2 font-mono text-xs text-muted-foreground"><span className="rounded-full border border-border bg-card px-3 py-2">CLI</span><span className="rounded-full border border-border bg-card px-3 py-2">MCP</span><span className="rounded-full border border-border bg-card px-3 py-2">VS Code</span><span className="rounded-full border border-border bg-card px-3 py-2">JSONL</span></div></div>
        </div>
      </PageSection>

      <section id="history" className="scroll-mt-24 border-y border-border/70 bg-muted/25">
        <div className="mx-auto max-w-7xl px-5 py-20 lg:px-8 lg:py-28">
          <div className="max-w-2xl"><p className="eyebrow">History</p><h2 className="mt-4 font-heading text-3xl font-semibold tracking-[-0.05em] sm:text-4xl">Understand how the system evolved, not just what the diff touched.</h2><p className="mt-5 text-base leading-7 text-muted-foreground">When a source diff is too narrow, compare the realizations that produced the system before and after the change. Published snapshots stay immutable.</p></div>
          <div className="mt-10"><HistoryComparisonDiagram /></div>
          <div className="mt-6 grid gap-4 md:grid-cols-3"><HistoryOutcome icon={GitBranchIcon} title="Commit" text="Bind the question to an exact revision." /><HistoryOutcome icon={GitCompareArrowsIcon} title="Compare" text="Diff graph structure and source evidence." /><HistoryOutcome icon={ShieldCheckIcon} title="Preserve" text="Keep historical answers stable and reviewable." /></div>
        </div>
      </section>

      <section className="border-t border-border/70 bg-primary text-primary-foreground">
        <div className="mx-auto flex max-w-7xl flex-col gap-7 px-5 py-16 lg:flex-row lg:items-center lg:justify-between lg:px-8 lg:py-20">
          <div className="max-w-2xl"><p className="eyebrow text-primary-foreground/70">Start with the uncertainty</p><h2 className="mt-3 font-heading text-3xl font-semibold tracking-[-0.05em] sm:text-4xl">Give the codebase a bearing.</h2><p className="mt-4 text-base leading-7 text-primary-foreground/75">Install Compass locally, choose one question, and keep the path from answer back to source.</p></div>
          <div className="flex flex-col gap-3 sm:flex-row"><Link className={cn(buttonVariants({ variant: 'secondary', size: 'lg' }), 'gap-2')} href="/install">Install Compass <ArrowRightIcon data-icon="inline-end" /></Link><Link className={cn(buttonVariants({ variant: 'outline', size: 'lg' }), 'border-primary-foreground/30 bg-transparent text-primary-foreground hover:bg-primary-foreground/10 hover:text-primary-foreground')} href="/product">See the product</Link></div>
        </div>
      </section>
    </MarketingPage>
  );
}

function WorkflowCard({ icon: Icon, eyebrow, title, description, href }: { icon: typeof CompassIcon; eyebrow: string; title: string; description: string; href: string }) {
  return <Link className="group flex min-h-56 flex-col gap-5 rounded-2xl border border-border/80 bg-card/70 p-5 transition-transform duration-300 hover:-translate-y-1 hover:border-primary/30" href={href}><span className="grid size-11 place-items-center rounded-xl border border-primary/15 bg-primary/[0.07] text-primary"><Icon aria-hidden="true" className="size-5" strokeWidth={1.8} /></span><span className="flex flex-col gap-2"><span className="eyebrow">{eyebrow}</span><span className="font-heading text-xl font-semibold tracking-[-0.04em]">{title}</span><span className="text-sm leading-6 text-muted-foreground">{description}</span></span><ArrowRightIcon className="mt-auto size-4 text-primary transition-transform group-hover:translate-x-1" /></Link>;
}

function Outcome({ icon: Icon, text }: { icon: typeof NetworkIcon; text: string }) {
  return <span className="flex items-start gap-3"><Icon className="mt-1 size-4 shrink-0 text-primary" /><span>{text}</span></span>;
}

function WorkflowStep({ title, text }: { title: string; text: string }) {
  return <Card className="border-border/80 bg-card/70 shadow-none"><CardContent className="flex min-h-32 flex-col justify-between gap-5 p-5"><span className="font-mono text-xs uppercase tracking-[0.12em] text-primary">{title}</span><span className="text-sm leading-6 text-muted-foreground">{text}</span></CardContent></Card>;
}

function ImpactCheck({ title, text }: { title: string; text: string }) {
  return <Card className="border-border/80 bg-card/70 shadow-none"><CardHeader className="gap-2"><span className="eyebrow">{title}</span><CardTitle className="font-heading text-lg tracking-[-0.035em]">{text}</CardTitle></CardHeader><CardContent><p className="flex items-center gap-2 text-sm text-muted-foreground"><CheckCircle2Icon className="size-4 text-primary" /> Kept explicit in the query result.</p></CardContent></Card>;
}

function AssistantStep({ title, text }: { title: string; text: string }) {
  return <Card className="border-border/80 bg-card/70 shadow-none"><CardContent className="flex min-h-40 flex-col justify-between gap-6 p-6"><span className="font-mono text-xs uppercase tracking-[0.12em] text-primary">handoff</span><div><p className="font-heading text-xl font-semibold tracking-[-0.04em]">{title}</p><p className="mt-2 text-sm leading-6 text-muted-foreground">{text}</p></div></CardContent></Card>;
}

function HistoryOutcome({ icon: Icon, title, text }: { icon: typeof GitBranchIcon; title: string; text: string }) {
  return <div className="flex items-start gap-4 rounded-xl border border-border/80 bg-card/70 p-5"><Icon className="mt-1 size-4 shrink-0 text-primary" /><div className="flex flex-col gap-1"><span className="font-mono text-sm text-primary">{title}</span><span className="text-sm leading-6 text-muted-foreground">{text}</span></div></div>;
}
