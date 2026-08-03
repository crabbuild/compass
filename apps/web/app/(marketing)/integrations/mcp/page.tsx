import { ArrowRightIcon, BracesIcon, CheckCircle2Icon, LockKeyholeIcon, NetworkIcon } from 'lucide-react';
import Link from 'next/link';

import { AssistantContextDiagram, McpSurfaceDiagram } from '@/components/diagrams';
import { FeatureGrid, MarketingPage, PageSection } from '@/components/marketing-page';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { buttonVariants } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import { pageMetadata } from '@/lib/site';

export const metadata = pageMetadata('MCP integration', 'Give assistants focused Compass graph context through a local MCP server and typed, bounded results.');

export default function McpPage() {
  return <MarketingPage eyebrow="Integration / MCP" title="Give tools a map, not a context dump." description="Compass exposes local graph resources and read-only query operations through an explicit MCP boundary. Assistants receive a small, structured answer with the same evidence you see in the viewer.">
    <PageSection eyebrow="Tool boundary" title="Ask one precise question at a time." description="MCP is useful when a tool needs structural context but does not need every file. The server validates the request, applies limits, and returns typed graph data.">
      <McpSurfaceDiagram />
    </PageSection>
    <section className="border-y border-border/70 bg-muted/25">
      <div className="mx-auto max-w-7xl px-5 py-20 lg:px-8 lg:py-28">
        <FeatureGrid items={[{ eyebrow: 'Resource', title: 'Read a graph resource', description: 'Expose the current local snapshot through compass:// resources for tools that need orientation.' }, { eyebrow: 'Operation', title: 'Run a bounded query', description: 'Use typed CompassQL input with explicit depth, result, and response limits.' }, { eyebrow: 'Result', title: 'Return source-aware fields', description: 'Keep stable identity, direction, relation, source anchors, and diagnostics in the answer.' }, { eyebrow: 'Boundary', title: 'Stay local by default', description: 'The MCP process does not imply a hosted index, credential, or hidden network fallback.' }]} />
      </div>
    </section>
    <PageSection eyebrow="The context hand-off" title="A tool can ask less and understand more." description="The graph answer remains compact because it carries structure, not because it drops the reason behind the result.">
      <div className="grid gap-8 lg:grid-cols-[1.1fr_0.9fr] lg:items-center">
        <AssistantContextDiagram />
        <Card className="border-border/80 bg-card/70 shadow-none"><CardHeader><CardTitle className="font-heading text-2xl tracking-[-0.045em]">A useful MCP response includes</CardTitle></CardHeader><CardContent className="flex flex-col gap-4 text-sm leading-7 text-muted-foreground"><span className="flex gap-3"><NetworkIcon className="mt-1 size-4 shrink-0 text-primary" />stable node and edge identities</span><span className="flex gap-3"><BracesIcon className="mt-1 size-4 shrink-0 text-primary" />machine-readable JSON fields</span><span className="flex gap-3"><CheckCircle2Icon className="mt-1 size-4 shrink-0 text-primary" />explicit bounds and diagnostics</span><span className="flex gap-3"><LockKeyholeIcon className="mt-1 size-4 shrink-0 text-primary" />a local process boundary</span></CardContent></Card>
      </div>
    </PageSection>
    <section className="border-t border-border/70 bg-primary text-primary-foreground"><div className="mx-auto flex max-w-7xl flex-col gap-6 px-5 py-16 lg:flex-row lg:items-center lg:justify-between lg:px-8"><div><p className="font-heading text-2xl font-semibold tracking-[-0.04em]">Connect the tool to the question.</p><p className="mt-2 text-sm text-primary-foreground/75">Read the integration contract, then install Compass locally.</p></div><Link className={cn(buttonVariants({ variant: 'secondary' }), 'gap-2')} href="/docs/reference/compatibility">Read the contract <ArrowRightIcon data-icon="inline-end" /></Link></div></section>
  </MarketingPage>;
}
