import type { LucideIcon } from 'lucide-react';
import { ArrowRightIcon, BotIcon, CheckCircle2Icon, FileCode2Icon, FileOutputIcon, GitPullRequestArrowIcon, LockKeyholeIcon, TerminalSquareIcon } from 'lucide-react';
import Link from 'next/link';

import { IntegrationMapDiagram } from '@/components/diagrams';
import { MarketingPage, PageSection } from '@/components/marketing-page';
import { Card, CardContent } from '@/components/ui/card';
import { buttonVariants } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import { pageMetadata } from '@/lib/site';

export const metadata = pageMetadata('Integrations', 'Bring Compass into editors, assistants, automation, and portable graph workflows without leaving the local workspace.', { path: '/integrations' });

const surfaces = [
  {
    id: 'editor',
    eyebrow: 'Editor',
    title: 'VS Code',
    description: 'Inspect a symbol, trace its neighborhood, and jump from an edge to the exact source anchor without leaving the workspace.',
    detail: 'For source-level exploration',
    actionLabel: 'Read the VS Code guide',
    href: '/docs/guides/vscode',
    icon: FileCode2Icon,
  },
  {
    id: 'assistants',
    eyebrow: 'Assistant',
    title: 'MCP + native skills',
    description: 'Give coding tools a focused graph answer through a local process boundary instead of pasting the whole repository into context.',
    detail: 'For focused machine context',
    actionLabel: 'Set up assistant access',
    href: '/docs/guides/assistant-setup',
    icon: BotIcon,
  },
  {
    id: 'automation',
    eyebrow: 'Automation',
    title: 'CLI + CI',
    description: 'Build a snapshot, run a bounded CompassQL query, and publish explicit JSON or JSONL output in a script or pull request.',
    detail: 'For repeatable structural checks',
    actionLabel: 'Read the CI cookbook',
    href: '/docs/cookbook/ci-and-automation',
    icon: TerminalSquareIcon,
  },
  {
    id: 'exports',
    eyebrow: 'Portable output',
    title: 'HTML, JSON, SVG + GraphML',
    description: 'Carry one validated snapshot into an interactive viewer, a static diagram, a machine-readable archive, or team notes.',
    detail: 'For sharing and downstream tools',
    actionLabel: 'Browse output formats',
    href: '/docs/reference/outputs',
    icon: FileOutputIcon,
  },
];

const guarantees = [
  { label: 'local', text: 'source and graph stay on the machine', icon: LockKeyholeIcon },
  { label: 'evidence', text: 'source anchors and provenance travel with edges', icon: CheckCircle2Icon },
  { label: 'bounded', text: 'limits stay explicit in every query and export', icon: GitPullRequestArrowIcon },
  { label: 'deterministic', text: 'stable ordering keeps automation reviewable', icon: ArrowRightIcon },
];

export default function IntegrationsPage() {
  return <MarketingPage eyebrow="Integrations" title="One graph. Every handoff." description="Meet Compass where the work already happens: in the editor, beside an assistant, inside automation, or in the artifact your team shares. Every surface starts from the same local, evidence-backed snapshot.">
    <PageSection eyebrow="The integration map" title="Build once. Route the answer." description="Compass does not create a second graph for each tool. It publishes one validated snapshot, then gives each workflow the view it can use.">
      <IntegrationMapDiagram />
    </PageSection>
    <section className="scroll-mt-24 border-y border-border/70 bg-muted/25" id="surfaces">
      <div className="mx-auto max-w-7xl px-5 py-20 lg:px-8 lg:py-28">
        <div className="max-w-2xl">
          <p className="eyebrow">Choose a surface</p>
          <h2 className="mt-4 font-heading text-3xl font-semibold tracking-[-0.05em] sm:text-4xl">Four ways to pick up the same answer.</h2>
          <p className="mt-5 text-base leading-7 text-muted-foreground">Start with the hand-off that matches the decision in front of you. The contract stays stable when the destination changes.</p>
        </div>
        <div className="mt-10 grid gap-4 md:grid-cols-2">
          {surfaces.map((surface) => <IntegrationSurfaceCard key={surface.id} {...surface} />)}
        </div>
      </div>
    </section>
    <PageSection eyebrow="The stable contract" title="Different surfaces. Same evidence." description="Identity, direction, source anchors, provenance, and explicit limits remain intact as the graph moves through the workflow.">
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
        {guarantees.map(({ icon: Icon, label, text }) => <Card className="border-border/80 bg-card/70 shadow-none" key={label}><CardContent className="flex min-h-40 flex-col justify-between gap-6 p-5"><Icon className="size-5 text-primary" /><div><p className="font-mono text-xs uppercase tracking-[0.12em] text-primary">{label}</p><p className="mt-2 text-sm leading-6 text-muted-foreground">{text}</p></div></CardContent></Card>)}
      </div>
      <div className="mt-10 flex flex-col gap-5 rounded-2xl border border-border/80 bg-card/70 p-6 sm:flex-row sm:items-center sm:justify-between sm:p-7">
        <div>
          <p className="font-heading text-xl font-semibold tracking-[-0.04em]">Ready to give the codebase a bearing?</p>
          <p className="mt-2 text-sm leading-6 text-muted-foreground">Install Compass locally, build one graph, and choose the surface that fits the next question.</p>
        </div>
        <Link className={cn(buttonVariants({ variant: 'outline' }), 'w-full gap-2 sm:w-auto')} href="/docs">Read the docs <ArrowRightIcon data-icon="inline-end" /></Link>
      </div>
    </PageSection>
  </MarketingPage>;
}

function IntegrationSurfaceCard({ icon: Icon, eyebrow, title, description, detail, actionLabel, href, id }: { icon: LucideIcon; eyebrow: string; title: string; description: string; detail: string; actionLabel: string; href: string; id: string }) {
  return <Card className="group relative flex min-h-64 scroll-mt-24 flex-col border-border/80 bg-card/70 shadow-none transition-transform duration-300 hover:-translate-y-1 hover:border-primary/30" id={id}>
    <CardContent className="flex h-full flex-col gap-7 p-6 sm:p-7">
      <div className="flex items-start justify-between gap-4">
        <span className="grid size-11 place-items-center rounded-xl border border-primary/15 bg-primary/[0.07] text-primary"><Icon aria-hidden="true" className="size-5" strokeWidth={1.8} /></span>
        <span className="eyebrow text-right">{eyebrow}</span>
      </div>
      <div>
        <h3 className="font-heading text-2xl font-semibold tracking-[-0.045em]">{title}</h3>
        <p className="mt-3 text-[0.95rem] leading-7 text-muted-foreground">{description}</p>
      </div>
      <div className="mt-auto flex flex-col gap-4 border-t border-border/70 pt-5 sm:flex-row sm:items-center sm:justify-between">
        <span className="font-mono text-[0.65rem] uppercase tracking-[0.1em] text-muted-foreground">{detail}</span>
        <Link className="inline-flex items-center gap-2 text-sm font-medium text-primary" href={href}>{actionLabel}<ArrowRightIcon className="size-4 transition-transform group-hover:translate-x-1" /></Link>
      </div>
    </CardContent>
  </Card>;
}
