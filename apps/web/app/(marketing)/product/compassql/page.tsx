import { ArrowRightIcon, BracesIcon, CheckCircle2Icon, CircleAlertIcon, GaugeIcon, ListChecksIcon } from 'lucide-react';
import Link from 'next/link';

import { ImpactPathDiagram } from '@/components/diagrams';
import { MarketingPage, PageSection } from '@/components/marketing-page';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { buttonVariants } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import { pageMetadata } from '@/lib/site';

export const metadata = pageMetadata('CompassQL', 'Ask deterministic, bounded, read-only structural questions with stable output and explicit diagnostics.');

export default function CompassQlPage() {
  return <MarketingPage eyebrow="CompassQL" title="Ask structural questions with a contract." description="CompassQL is a deterministic, bounded, read-only query surface for finding paths, relationships, and impact without inventing meaning.">
    <PageSection eyebrow="Query flow" title="Readable input. Stable output." description="Use CompassQL interactively in the CLI or as a machine contract in automation.">
      <div className="grid gap-6 lg:grid-cols-[1.1fr_0.9fr]">
        <Card className="overflow-hidden border-border/80 bg-compass-canvas-deep shadow-none"><CardHeader className="border-b border-border/70"><CardTitle className="flex items-center gap-3 font-mono text-sm tracking-normal"><BracesIcon className="text-compass-amber" /> impact.cql</CardTitle></CardHeader><CardContent className="p-6 font-mono text-sm leading-8"><p><span className="text-compass-amber">MATCH</span> (changed)-[:CALLS|IMPORTS_FROM*1..3]-&gt;(affected)</p><p><span className="text-compass-amber">WHERE</span> changed.source_file = <span className="text-primary">&quot;src/payments.rs&quot;</span></p><p><span className="text-compass-amber">RETURN</span> affected.id, affected.source</p><p><span className="text-compass-amber">ORDER BY</span> affected.id</p><p><span className="text-muted-foreground">LIMIT 100</span></p></CardContent></Card>
        <Card className="border-border/80 bg-card/70 shadow-none"><CardHeader><CardTitle className="font-heading text-xl tracking-[-0.04em]">A query result should tell you why.</CardTitle></CardHeader><CardContent className="flex flex-col gap-4 text-sm leading-7 text-muted-foreground"><span className="flex gap-2"><CheckCircle2Icon className="mt-1 shrink-0 text-primary" /> deterministic ordering</span><span className="flex gap-2"><CheckCircle2Icon className="mt-1 shrink-0 text-primary" /> bounded path expansion</span><span className="flex gap-2"><CheckCircle2Icon className="mt-1 shrink-0 text-primary" /> explicit unsupported syntax</span><span className="flex gap-2"><CheckCircle2Icon className="mt-1 shrink-0 text-primary" /> JSON and JSONL output</span></CardContent></Card>
      </div>
    </PageSection>
    <section className="border-y border-border/70 bg-muted/25">
      <div className="mx-auto max-w-7xl px-5 py-20 lg:px-8 lg:py-28">
        <div className="grid gap-8 lg:grid-cols-[0.92fr_1.08fr] lg:items-center">
          <div>
            <p className="eyebrow">Query anatomy</p>
            <h2 className="mt-4 font-heading text-3xl font-semibold tracking-[-0.05em] sm:text-4xl">Readable clauses make the boundary obvious.</h2>
            <p className="mt-5 text-base leading-7 text-muted-foreground">CompassQL keeps the familiar shape of a graph query, then makes path expansion, result size, and unsupported syntax explicit.</p>
          </div>
          <div className="grid gap-4 sm:grid-cols-3">
            <QueryClause icon={BracesIcon} label="MATCH" text="choose a relationship and direction" />
            <QueryClause icon={GaugeIcon} label="WHERE" text="narrow the corpus with a predicate" />
            <QueryClause icon={ListChecksIcon} label="RETURN" text="select stable fields and anchors" />
          </div>
        </div>
      </div>
    </section>
    <PageSection eyebrow="Impact without a context dump" title="Bound the path, then show the reason." description="A useful traversal is small enough to read and rich enough to audit. The query engine keeps both properties in view.">
      <ImpactPathDiagram />
      <div className="mt-6 grid gap-4 md:grid-cols-3">
        <QuerySignal icon={GaugeIcon} title="bounded" text="depth and result limits are part of the contract" />
        <QuerySignal icon={CheckCircle2Icon} title="deterministic" text="stable ordering makes output repeatable" />
        <QuerySignal icon={CircleAlertIcon} title="honest" text="unsupported or ambiguous results stay explicit" />
      </div>
    </PageSection>
    <section className="border-t border-border/70 bg-card/45">
      <div className="mx-auto max-w-7xl px-5 py-20 lg:px-8 lg:py-28">
        <div className="grid gap-5 md:grid-cols-2 lg:grid-cols-4">
          <ContractCard label="Input" text="human-readable CompassQL" />
          <ContractCard label="Planner" text="bounded paths + predicates" />
          <ContractCard label="Diagnostics" text="typed unsupported syntax" />
          <ContractCard label="Output" text="JSON or JSONL with anchors" />
        </div>
      </div>
    </section>
    <section className="border-t border-border/70 bg-primary text-primary-foreground"><div className="mx-auto flex max-w-7xl flex-col gap-6 px-5 py-16 lg:flex-row lg:items-center lg:justify-between lg:px-8"><div><p className="font-heading text-2xl font-semibold tracking-[-0.04em]">Read the language contract before you automate it.</p><p className="mt-2 text-sm text-primary-foreground/75">Understand support, bounds, and diagnostics in the reference docs.</p></div><Link className={cn(buttonVariants({ variant: 'secondary' }), 'gap-2')} href="/docs/COMPASSQL">Read CompassQL <ArrowRightIcon data-icon="inline-end" /></Link></div></section>
  </MarketingPage>;
}

function QueryClause({ icon: Icon, label, text }: { icon: typeof BracesIcon; label: string; text: string }) {
  return <div className="flex min-h-36 flex-col gap-4 rounded-xl border border-border/80 bg-card/75 p-5"><Icon className="text-primary" /><span className="font-mono text-xs uppercase tracking-[0.12em] text-primary">{label}</span><span className="text-sm leading-6 text-muted-foreground">{text}</span></div>;
}

function QuerySignal({ icon: Icon, title, text }: { icon: typeof GaugeIcon; title: string; text: string }) {
  return <div className="flex items-start gap-3 rounded-xl border border-border/80 bg-card/70 p-5"><Icon className="mt-1 size-4 shrink-0 text-primary" /><div className="flex flex-col gap-1"><span className="font-mono text-xs uppercase tracking-[0.12em] text-primary">{title}</span><span className="text-sm leading-6 text-muted-foreground">{text}</span></div></div>;
}

function ContractCard({ label, text }: { label: string; text: string }) {
  return <div className="rounded-xl border border-border/80 bg-card/70 p-5"><span className="eyebrow">{label}</span><p className="mt-3 font-heading text-lg font-semibold tracking-[-0.035em]">{text}</p></div>;
}
