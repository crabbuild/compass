import { ArrowUpRightIcon, CircleDotIcon, FlagIcon, GitMergeIcon } from 'lucide-react';
import Link from 'next/link';

import { MarketingPage, PageSection } from '@/components/marketing-page';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { pageMetadata } from '@/lib/site';

export const metadata = pageMetadata('Roadmap', 'See what Compass ships today, what is in progress, and which ideas remain explicitly aspirational.');

const items = [
  [CircleDotIcon, 'Available now', 'Native structural extraction, graph queries, CompassQL, history, exports, and the VS Code workflow.'],
  [GitMergeIcon, 'In progress', 'Broader language evidence, richer semantic validation, and deeper integration qualification.'],
  [FlagIcon, 'Aspirational', 'Future surfaces will be labeled when they have evidence, not just a compelling diagram.'],
] as const;

export default function RoadmapPage() {
  return <MarketingPage eyebrow="Roadmap" title="A direction with evidence attached." description="Compass changes quickly. The roadmap separates what is available from committed plans and ideas worth exploring.">
    <PageSection eyebrow="Current status" title="Read the label before you read the promise.">
      <div className="grid gap-5 md:grid-cols-3">{items.map(([Icon, title, text]) => <Card className="border-border/80 bg-card/70 shadow-none" key={title}><CardHeader className="gap-4"><Icon className="text-primary" /><CardTitle className="font-heading text-xl tracking-[-0.04em]">{title}</CardTitle></CardHeader><CardContent><p className="text-sm leading-7 text-muted-foreground">{text}</p></CardContent></Card>)}</div>
    </PageSection>
    <div className="mx-auto max-w-7xl px-5 pb-20 lg:px-8 lg:pb-28"><Link className="inline-flex items-center gap-2 text-sm font-medium text-primary" href="/docs/roadmap">Open the detailed roadmap <ArrowUpRightIcon data-icon="inline-end" /></Link></div>
  </MarketingPage>;
}
