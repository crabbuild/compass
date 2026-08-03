import { ArrowUpRightIcon, CalendarDaysIcon } from 'lucide-react';
import Link from 'next/link';

import { MarketingPage, PageSection } from '@/components/marketing-page';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { pageMetadata } from '@/lib/site';

export const metadata = pageMetadata('Changelog', 'Follow release-visible changes to the Compass CLI, graph formats, history, integrations, and documentation.');

export default function ChangelogPage() {
  return <MarketingPage eyebrow="Changelog" title="Small releases, visible contracts." description="Follow release-visible changes to the CLI, graph formats, history, integrations, and documentation.">
    <PageSection eyebrow="Latest release notes" title="The details belong close to the product.">
      <Card className="max-w-3xl border-border/80 bg-card/70 shadow-none"><CardHeader className="gap-3"><div className="flex items-center gap-2 font-mono text-xs text-muted-foreground"><CalendarDaysIcon className="text-primary" /> current release line</div><CardTitle className="font-heading text-2xl tracking-[-0.045em]">Compass 0.3.x</CardTitle></CardHeader><CardContent className="flex flex-col gap-5"><p className="text-sm leading-7 text-muted-foreground">Read the repository changelog for the authoritative release history, compatibility notes, and migration context.</p><Link className="inline-flex items-center gap-2 text-sm font-medium text-primary" href="https://github.com/crabbuild/compass/blob/main/CHANGELOG.md" target="_blank" rel="noreferrer">Open CHANGELOG.md <ArrowUpRightIcon data-icon="inline-end" /></Link></CardContent></Card>
    </PageSection>
  </MarketingPage>;
}
