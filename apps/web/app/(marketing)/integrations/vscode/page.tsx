import { ArrowRightIcon, CheckCircle2Icon, GitBranchIcon, NetworkIcon, SearchIcon } from 'lucide-react';
import Link from 'next/link';

import { EditorSurfaceDiagram } from '@/components/diagrams';
import { FeatureGrid, MarketingPage, PageSection } from '@/components/marketing-page';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { buttonVariants } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import { pageMetadata } from '@/lib/site';

export const metadata = pageMetadata('VS Code integration', 'Keep Compass graph exploration, focused call flows, architecture views, queries, and Git evolution beside the code.');

export default function VscodePage() {
  return <MarketingPage eyebrow="Integration / VS Code" title="Keep the graph beside the code." description="The first-party Compass Codegraph extension brings current graphs, focused call flows, architecture views, queries, and Git evolution into the editor. It uses the same graph model as the CLI and exported viewer.">
    <PageSection eyebrow="Editor surface" title="Move from a symbol to the surrounding system." description="The extension is for the moments when a file is not enough: inspect a node, trace a relationship, and open the exact source anchor without losing your place.">
      <EditorSurfaceDiagram />
    </PageSection>
    <section className="border-y border-border/70 bg-muted/25">
      <div className="mx-auto max-w-7xl px-5 py-20 lg:px-8 lg:py-28">
        <FeatureGrid items={[{ eyebrow: 'Explore', title: 'Current graph surface', description: 'Open the latest local graph and focus a node neighborhood without leaving the workspace.', href: '/product/code-graph' }, { eyebrow: 'Focus', title: 'Cursor-rooted call graphs', description: 'Start from the symbol under the cursor and expand only the caller or callee path that helps.', href: '/use-cases/impact-analysis' }, { eyebrow: 'Compare', title: 'Exact Git evolution', description: 'See how graph topology changes across revisions while source navigation stays available.', href: '/product/history' }, { eyebrow: 'Query', title: 'Natural language and CompassQL', description: 'Ask a focused question, inspect structured results, and keep the query available for the next review.', href: '/product/compassql' }]} />
      </div>
    </section>
    <PageSection eyebrow="A short editor loop" title="Inspect, focus, navigate, repeat." description="The extension makes the graph useful by keeping each interaction close to a source-level next step.">
      <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-4">
        <EditorStep icon={SearchIcon} title="Open" text="Load the current graph snapshot." />
        <EditorStep icon={NetworkIcon} title="Focus" text="Select a node to reveal its neighborhood." />
        <EditorStep icon={GitBranchIcon} title="Trace" text="Hover edges for relation and evidence." />
        <EditorStep icon={CheckCircle2Icon} title="Navigate" text="Double-click to open the source range." />
      </div>
    </PageSection>
    <section className="border-t border-border/70 bg-primary text-primary-foreground"><div className="mx-auto flex max-w-7xl flex-col gap-6 px-5 py-16 lg:flex-row lg:items-center lg:justify-between lg:px-8"><div><p className="font-heading text-2xl font-semibold tracking-[-0.04em]">Start with the editor you already use.</p><p className="mt-2 text-sm text-primary-foreground/75">Install Compass locally, then open the extension from the workspace.</p></div><Link className={cn(buttonVariants({ variant: 'secondary' }), 'gap-2')} href="/install">Install Compass <ArrowRightIcon data-icon="inline-end" /></Link></div></section>
  </MarketingPage>;
}

function EditorStep({ icon: Icon, title, text }: { icon: typeof SearchIcon; title: string; text: string }) {
  return <Card className="border-border/80 bg-card/70 shadow-none"><CardContent className="flex min-h-36 flex-col justify-between gap-6 p-5"><Icon className="text-primary" /><div><p className="font-heading text-lg font-semibold tracking-[-0.035em]">{title}</p><p className="mt-1 text-sm leading-6 text-muted-foreground">{text}</p></div></CardContent></Card>;
}
