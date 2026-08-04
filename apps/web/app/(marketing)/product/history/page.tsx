import { CheckCircle2Icon, GitCompareArrowsIcon, GitCommitHorizontalIcon, ShieldCheckIcon } from 'lucide-react';

import { HistoryComparisonDiagram } from '@/components/diagrams';
import { FeatureGrid, MarketingPage, PageSection } from '@/components/marketing-page';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { pageMetadata } from '@/lib/site';

export const metadata = pageMetadata('Versioned history', 'Compare graph realizations tied to exact Git commits without rewriting the historical snapshots you are investigating.');

export default function HistoryPage() {
  return <MarketingPage eyebrow="Versioned history" title="Compare two code graphs, not just two source diffs." description="Choose exact Git revisions, identify the immutable graph behind each one, and see the nodes, relationships, and evidence that changed.">
      <PageSection eyebrow="History surface" title="Identify both graphs before you inspect the change." description="Every comparison keeps the baseline and current graph tied to an exact commit, then shows their topology and evidence side by side.">
        <FeatureGrid items={[{ eyebrow: 'Commit', title: 'Exact revision binding', description: 'Ask questions against an immutable graph realization for a known commit.' }, { eyebrow: 'Topology', title: 'Graph-to-graph comparison', description: 'Find added, removed, and changed nodes and relationships without rewriting history.' }, { eyebrow: 'Evidence', title: 'Confidence stays visible', description: 'Separate direct, inferred, unresolved, and ambiguous results instead of flattening them.' }]} />
      </PageSection>
      <section className="border-y border-border/70 bg-muted/25">
        <div className="mx-auto max-w-7xl px-5 py-20 lg:px-8 lg:py-28">
          <HistoryComparisonDiagram />
        </div>
      </section>
      <section className="border-y border-border/70 bg-muted/25"><div className="mx-auto grid max-w-7xl gap-5 px-5 py-20 md:grid-cols-3 lg:px-8 lg:py-28"><HistorySignal icon={GitCommitHorizontalIcon} title="commit" text="bind the question to an exact revision" /><HistorySignal icon={GitCompareArrowsIcon} title="compare" text="diff graph structure and source evidence" /><HistorySignal icon={ShieldCheckIcon} title="preserve" text="keep published realizations immutable" /></div></section>
      <PageSection eyebrow="A safe comparison loop" title="History gives the question a stable object." description="When a source diff is too narrow, compare the realizations that produced the system before and after the change.">
        <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-4">
          <HistoryStep index="01" title="Select" text="choose exact parent and current revisions" />
          <HistoryStep index="02" title="Materialize" text="build or reuse each compatible realization" />
          <HistoryStep index="03" title="Compare" text="classify topology and evidence changes" />
          <HistoryStep index="04" title="Inspect" text="jump from a changed edge to its source anchor" />
        </div>
      </PageSection>
      <section className="border-t border-border/70 bg-card/45">
        <div className="mx-auto grid max-w-7xl gap-8 px-5 py-20 lg:grid-cols-[0.85fr_1.15fr] lg:items-center lg:px-8 lg:py-28">
          <div>
            <p className="eyebrow">What the diff refuses to do</p>
            <h2 className="mt-4 font-heading text-3xl font-semibold tracking-[-0.05em]">It never rewrites the historical answer to make the new one fit.</h2>
            <p className="mt-5 text-base leading-7 text-muted-foreground">Published realizations are immutable. If evidence is missing or a target is ambiguous, the comparison reports that state instead of manufacturing a clean result.</p>
          </div>
          <Card className="border-border/80 bg-card/70 shadow-none"><CardHeader><CardTitle className="font-heading text-xl tracking-[-0.04em]">Comparison checks</CardTitle></CardHeader><CardContent className="grid gap-4 text-sm leading-7 text-muted-foreground sm:grid-cols-2"><span className="flex gap-2"><CheckCircle2Icon className="mt-1 size-4 shrink-0 text-primary" />exact revision identity</span><span className="flex gap-2"><CheckCircle2Icon className="mt-1 size-4 shrink-0 text-primary" />compatible profile + schema</span><span className="flex gap-2"><CheckCircle2Icon className="mt-1 size-4 shrink-0 text-primary" />evidence-gated change</span><span className="flex gap-2"><CheckCircle2Icon className="mt-1 size-4 shrink-0 text-primary" />reviewable source anchors</span></CardContent></Card>
        </div>
      </section>
    </MarketingPage>;
}

function HistorySignal({ icon: Icon, title, text }: { icon: typeof GitCommitHorizontalIcon; title: string; text: string }) { return <div className="flex items-start gap-4 rounded-xl border border-border/80 bg-card/70 p-6"><Icon className="mt-1 shrink-0 text-primary" /><div className="flex flex-col gap-1"><span className="font-mono text-sm text-primary">{title}</span><span className="text-sm leading-6 text-muted-foreground">{text}</span></div></div>; }

function HistoryStep({ index, title, text }: { index: string; title: string; text: string }) {
  return <Card className="border-border/80 bg-card/70 shadow-none"><CardContent className="flex min-h-36 flex-col justify-between gap-5 p-5"><span className="font-mono text-xs text-primary">{index}</span><div className="flex flex-col gap-1"><span className="font-heading text-lg font-semibold tracking-[-0.035em]">{title}</span><span className="text-sm leading-6 text-muted-foreground">{text}</span></div></CardContent></Card>;
}
