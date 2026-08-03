import { ArrowRightIcon, CheckCircle2Icon, GitPullRequestArrowIcon, ShieldCheckIcon } from 'lucide-react';
import Link from 'next/link';

import { AutomationSurfaceDiagram, HistoryComparisonDiagram } from '@/components/diagrams';
import { FeatureGrid, MarketingPage, PageSection } from '@/components/marketing-page';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { buttonVariants } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import { pageMetadata } from '@/lib/site';

export const metadata = pageMetadata('CI integration', 'Carry Compass graph snapshots and bounded query results through pull-request and continuous-integration workflows.');

export default function CiPage() {
  return <MarketingPage eyebrow="Integration / CI" title="Review structure with the same discipline as source." description="Use Compass in CI when a line diff is not enough. Build or reuse exact graph realizations, run a read-only query, and make the result available beside the change without inventing a verdict.">
    <PageSection eyebrow="Review surface" title="A pull request can carry its graph evidence." description="CI is the delivery boundary: it should publish the artifacts and status a reviewer needs, while policy and interpretation remain explicit.">
      <AutomationSurfaceDiagram />
    </PageSection>
    <section className="border-y border-border/70 bg-muted/25"><div className="mx-auto max-w-7xl px-5 py-20 lg:px-8 lg:py-28"><FeatureGrid items={[{ eyebrow: 'Revision', title: 'Bind to exact commits', description: 'Use the parent and current revisions as stable graph inputs.' }, { eyebrow: 'Evidence', title: 'Keep source anchors', description: 'Make a relationship reviewable by carrying its source location with the output.' }, { eyebrow: 'Bounds', title: 'Fail explicitly on limits', description: 'A truncated traversal is a signal to inspect, not an empty pass.' }, { eyebrow: 'Delivery', title: 'Publish machine output', description: 'Keep JSON or JSONL available to the next CI step and the reviewer.' }]} /></div></section>
    <PageSection eyebrow="History-aware review" title="Topology changes deserve a stable comparison." description="When a pull request rearranges architecture, compare graph realizations without rewriting the snapshots that explain the change.">
      <HistoryComparisonDiagram />
      <div className="mt-6 grid gap-4 md:grid-cols-3"><CiCheck icon={GitPullRequestArrowIcon} title="review" text="show what the change reaches" /><CiCheck icon={ShieldCheckIcon} title="preserve" text="keep published snapshots immutable" /><CiCheck icon={CheckCircle2Icon} title="explain" text="attach evidence to every result" /></div>
    </PageSection>
    <section className="border-t border-border/70 bg-primary text-primary-foreground"><div className="mx-auto flex max-w-7xl flex-col gap-6 px-5 py-16 lg:flex-row lg:items-center lg:justify-between lg:px-8"><div><p className="font-heading text-2xl font-semibold tracking-[-0.04em]">Keep CI honest about what it knows.</p><p className="mt-2 text-sm text-primary-foreground/75">Start with artifacts and evidence, then add policy as a separate decision.</p></div><Link className={cn(buttonVariants({ variant: 'secondary' }), 'gap-2')} href="/product/history">Explore versioned history <ArrowRightIcon data-icon="inline-end" /></Link></div></section>
  </MarketingPage>;
}

function CiCheck({ icon: Icon, title, text }: { icon: typeof GitPullRequestArrowIcon; title: string; text: string }) {
  return <Card className="border-border/80 bg-card/70 shadow-none"><CardContent className="flex items-start gap-3 p-5"><Icon className="mt-1 size-4 shrink-0 text-primary" /><div><p className="font-mono text-xs uppercase tracking-[0.12em] text-primary">{title}</p><p className="mt-1 text-sm leading-6 text-muted-foreground">{text}</p></div></CardContent></Card>;
}
