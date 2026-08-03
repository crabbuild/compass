import { ArrowRightIcon, CheckCircle2Icon, FileCode2Icon, TerminalSquareIcon } from 'lucide-react';
import Link from 'next/link';

import { AutomationSurfaceDiagram } from '@/components/diagrams';
import { FeatureGrid, MarketingPage, PageSection } from '@/components/marketing-page';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { buttonVariants } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import { pageMetadata } from '@/lib/site';

export const metadata = pageMetadata('CLI integration', 'Use Compass from scripts and CI with deterministic builds, CompassQL queries, bounded output, and portable artifacts.');

export default function CliPage() {
  return <MarketingPage eyebrow="Integration / CLI" title="Make graph questions part of the command line." description="The Compass executable is the integration surface underneath every other workflow. Build a local snapshot, run a bounded CompassQL query, and carry the result into scripts or review tooling.">
    <PageSection eyebrow="Automation surface" title="A command sequence you can inspect." description="The CLI keeps scope, snapshot, query, and output as separate steps so a failure is actionable and a successful result is portable.">
      <AutomationSurfaceDiagram />
    </PageSection>
    <section className="border-y border-border/70 bg-muted/25"><div className="mx-auto max-w-7xl px-5 py-20 lg:px-8 lg:py-28"><FeatureGrid items={[{ eyebrow: 'Build', title: 'Publish graph.json', description: 'Create a coherent local artifact set rooted at compass-out/.' }, { eyebrow: 'Query', title: 'Use --cql explicitly', description: 'Run the documented read-only CompassQL subset with stable ordering and bounds.' }, { eyebrow: 'Output', title: 'Choose JSON or JSONL', description: 'Feed scripts and automation without parsing human-readable prose.' }, { eyebrow: 'Repeat', title: 'Keep the command nearby', description: 'Save the query file or command as a small, reviewable contract.' }]} /></div></section>
    <PageSection eyebrow="A practical command set" title="Keep the hand-off obvious." description="These are the artifacts a script can reason about without reaching into Compass internals.">
      <div className="grid gap-6 lg:grid-cols-[1.05fr_0.95fr]">
        <Card className="overflow-hidden border-border/80 bg-compass-canvas-deep shadow-none"><CardHeader className="border-b border-border/70"><CardTitle className="flex items-center gap-3 font-mono text-sm tracking-normal"><TerminalSquareIcon className="text-compass-amber" /> terminal</CardTitle></CardHeader><CardContent className="p-6 font-mono text-xs leading-7 text-muted-foreground"><p><span className="text-compass-amber">$</span> compass build .</p><p><span className="text-compass-amber">$</span> compass query --cql </p><p className="pl-4">&quot;MATCH (n:Function) RETURN n.id LIMIT 20&quot; </p><p className="pl-4">--format json</p><p className="mt-3 text-primary">wrote compass-out/graph.json</p></CardContent></Card>
        <Card className="border-border/80 bg-card/70 shadow-none"><CardHeader><CardTitle className="font-heading text-2xl tracking-[-0.045em]">What the script can trust</CardTitle></CardHeader><CardContent className="flex flex-col gap-4 text-sm leading-7 text-muted-foreground"><span className="flex gap-3"><CheckCircle2Icon className="mt-1 size-4 shrink-0 text-primary" />versioned graph and query contracts</span><span className="flex gap-3"><FileCode2Icon className="mt-1 size-4 shrink-0 text-primary" />repository-relative source anchors</span><span className="flex gap-3"><CheckCircle2Icon className="mt-1 size-4 shrink-0 text-primary" />explicit limits and failure states</span></CardContent></Card>
      </div>
    </PageSection>
    <section className="border-t border-border/70 bg-primary text-primary-foreground"><div className="mx-auto flex max-w-7xl flex-col gap-6 px-5 py-16 lg:flex-row lg:items-center lg:justify-between lg:px-8"><div><p className="font-heading text-2xl font-semibold tracking-[-0.04em]">Put one structural question in your build.</p><p className="mt-2 text-sm text-primary-foreground/75">Start with a local snapshot, then grow the contract as the question earns it.</p></div><Link className={cn(buttonVariants({ variant: 'secondary' }), 'gap-2')} href="/install">Read install steps <ArrowRightIcon data-icon="inline-end" /></Link></div></section>
  </MarketingPage>;
}
