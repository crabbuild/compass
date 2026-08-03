import { BotIcon, CircleAlertIcon, CompassIcon, GitBranchIcon } from 'lucide-react';
import Link from 'next/link';

import { AssistantContextDiagram, ImpactPathDiagram } from '@/components/diagrams';
import { FeatureGrid, MarketingPage, PageSection } from '@/components/marketing-page';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { pageMetadata } from '@/lib/site';

export const metadata = pageMetadata('Use cases', 'Explore how Compass helps developers understand unfamiliar codebases, estimate impact, assist tools, and compare history.');

export default function UseCasesPage() {
  return <MarketingPage eyebrow="Use cases" title="When the file tree stops being enough." description="Choose the kind of uncertainty in front of you. Compass gives each question a smaller surface and a traceable answer.">
    <PageSection eyebrow="Developer workflows" title="Start from the work, not the feature list.">
      <FeatureGrid items={[{ eyebrow: 'Explore', title: 'Read an unfamiliar codebase', description: 'Map communities, entry points, and surprising connections before you start editing.', href: '/use-cases/codebase-exploration' }, { eyebrow: 'Impact', title: 'Estimate change blast radius', description: 'Traverse incoming relationships from a changed symbol and retain the path evidence.', href: '/use-cases/impact-analysis' }, { eyebrow: 'Assist', title: 'Give tools focused context', description: 'Serve compact graph answers to assistants, editors, and automation without dumping a repository.', href: '/use-cases/assistants' }, { eyebrow: 'History', title: 'Understand evolution', description: 'Compare graph realizations across exact Git revisions when a source diff hides the topology.', href: '/product/history' }]} />
    </PageSection>
    <section className="border-y border-border/70 bg-muted/25"><div className="mx-auto grid max-w-7xl gap-5 px-5 py-20 md:grid-cols-2 lg:grid-cols-4 lg:px-8 lg:py-28"><Workflow icon={CompassIcon} label="map" /><Workflow icon={CircleAlertIcon} label="impact" /><Workflow icon={BotIcon} label="assist" /><Workflow icon={GitBranchIcon} label="history" /></div></section>
    <PageSection eyebrow="Choose by uncertainty" title="Each workflow shrinks a different kind of unknown." description="Compass is most useful when you can name what is unclear. Pick the surface that keeps that uncertainty visible instead of handing you a larger pile of context.">
      <div className="grid gap-4 md:grid-cols-2">
        <UncertaintyCard label="I do not know where to start" title="Orient with the graph" text="Communities, entry points, and hoverable node details turn a file tree into a first hypothesis." href="/use-cases/codebase-exploration" />
        <UncertaintyCard label="I do not know what this touches" title="Trace the impact path" text="Reverse dependencies and bounded traversal show which targets can be reached, and why." href="/use-cases/impact-analysis" />
        <UncertaintyCard label="I do not know what to give a tool" title="Return focused context" text="A small machine-readable answer keeps an assistant grounded without shipping the whole repository." href="/use-cases/assistants" />
        <UncertaintyCard label="I do not know when it changed" title="Compare realizations" text="Exact Git-bound graph snapshots reveal topology that a line diff can hide." href="/product/history" />
      </div>
    </PageSection>
    <section className="border-t border-border/70 bg-card/45">
      <div className="mx-auto max-w-7xl px-5 py-20 lg:px-8 lg:py-28">
        <div className="grid gap-8 lg:grid-cols-2">
          <ImpactPathDiagram />
          <AssistantContextDiagram />
        </div>
      </div>
    </section>
  </MarketingPage>;
}

function Workflow({ icon: Icon, label }: { icon: typeof CompassIcon; label: string }) { return <div className="flex items-center gap-3 rounded-xl border border-border/80 bg-card/70 p-5"><Icon className="text-primary" /><span className="font-mono text-sm text-muted-foreground">{label}</span></div>; }

function UncertaintyCard({ label, title, text, href }: { label: string; title: string; text: string; href: string }) {
  return <Card className="group border-border/80 bg-card/70 shadow-none transition-transform duration-300 hover:-translate-y-1"><CardHeader className="gap-3"><span className="eyebrow">{label}</span><CardTitle className="font-heading text-2xl tracking-[-0.045em]">{title}</CardTitle></CardHeader><CardContent><Link className="text-sm leading-7 text-muted-foreground underline decoration-border underline-offset-4 transition-colors group-hover:text-foreground" href={href}>{text}</Link></CardContent></Card>;
}
