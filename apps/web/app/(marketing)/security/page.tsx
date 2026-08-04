import { CheckCircle2Icon, CloudOffIcon, KeyRoundIcon, ShieldCheckIcon } from 'lucide-react';

import { FeatureGrid, MarketingPage, PageSection } from '@/components/marketing-page';
import { pageMetadata } from '@/lib/site';

export const metadata = pageMetadata('Security and privacy', 'Understand Compass local-first defaults, optional provider boundaries, bounded inputs, and validated outputs.', { path: '/security' });

export default function SecurityPage() {
  return <MarketingPage eyebrow="Security and privacy" title="Local-first is a boundary, not a slogan." description="Compass keeps structural extraction and graph queries on the machine where the repository lives. Optional network and credential paths are explicit.">
    <PageSection eyebrow="Trust boundaries" title="Know what happens before you enable it.">
      <FeatureGrid items={[{ eyebrow: 'Default', title: 'No hosted dependency', description: 'The native CLI does not require Python, embeddings, a vector database, model credentials, or runtime grammar downloads.' }, { eyebrow: 'Optional', title: 'Provider paths are visible', description: 'Semantic enrichment and external integrations are configured capabilities, never silent fallbacks.' }, { eyebrow: 'Bounded', title: 'Inputs are treated as untrusted', description: 'Source files, archives, network responses, queries, and subprocess output stay within explicit limits.' }]} />
    </PageSection>
    <section className="border-y border-border/70 bg-muted/25"><div className="mx-auto grid max-w-7xl gap-5 px-5 py-20 md:grid-cols-2 lg:grid-cols-4 lg:px-8 lg:py-28"><Boundary icon={CloudOffIcon} title="local" /><Boundary icon={KeyRoundIcon} title="explicit credentials" /><Boundary icon={ShieldCheckIcon} title="validated outputs" /><Boundary icon={CheckCircle2Icon} title="deterministic" /></div></section>
  </MarketingPage>;
}

function Boundary({ icon: Icon, title }: { icon: typeof CloudOffIcon; title: string }) { return <div className="flex items-center gap-3 rounded-xl border border-border/80 bg-card/70 p-5"><Icon className="text-primary" /><span className="font-mono text-sm text-muted-foreground">{title}</span></div>; }
