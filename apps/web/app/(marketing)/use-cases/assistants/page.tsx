import { AssistantContextDiagram } from '@/components/diagrams';
import { FeatureGrid, MarketingPage, PageSection } from '@/components/marketing-page';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { pageMetadata } from '@/lib/site';

export const metadata = pageMetadata('Compass for assistants', 'Give assistants focused, structured graph context through native skills, hooks, MCP, and editor workflows.');

export default function AssistantsPage() {
  return <MarketingPage eyebrow="Use case / assistants" title="Give assistants a map, not a context dump." description="Compass exposes focused graph queries through native skills, hooks, MCP, and editor workflows so tools can ask smaller, better-scoped questions.">
    <PageSection eyebrow="Focused context" title="Make the answer compact without making it vague.">
      <FeatureGrid items={[{ eyebrow: 'Native', title: 'Works without a hosted index', description: 'Structural extraction and graph queries stay local, so the integration starts from the repository you already have.' }, { eyebrow: 'Typed', title: 'Machine-readable contracts', description: 'JSON and JSONL results preserve stable identities, direction, bounds, and diagnostics.' }, { eyebrow: 'Safe', title: 'Explicit process boundaries', description: 'Optional providers and network operations are visible configuration choices, not hidden fallbacks.' }]} />
    </PageSection>
    <section className="border-y border-border/70 bg-muted/25">
      <div className="mx-auto max-w-7xl px-5 py-20 lg:px-8 lg:py-28">
        <AssistantContextDiagram />
      </div>
    </section>
    <PageSection eyebrow="A focused hand-off" title="Give the tool the smallest context that still explains the answer." description="The integration boundary is a product surface: a question, a bounded query, and a result that carries enough structure for the next action.">
      <div className="grid gap-5 lg:grid-cols-3">
        <AssistantStep index="01" title="Ask" text="Resolve the user’s question into a path, impact, or relationship query." />
        <AssistantStep index="02" title="Bound" text="Apply explicit depth, limit, and provider boundaries before execution." />
        <AssistantStep index="03" title="Return" text="Keep identities, anchors, diagnostics, and provenance in the answer." />
      </div>
    </PageSection>
    <section className="border-t border-border/70 bg-card/45">
      <div className="mx-auto grid max-w-7xl gap-8 px-5 py-20 lg:grid-cols-[1.08fr_0.92fr] lg:items-center lg:px-8 lg:py-28">
        <Card className="overflow-hidden border-border/80 bg-compass-canvas-deep shadow-none"><CardHeader className="border-b border-border/70"><CardTitle className="font-mono text-sm tracking-normal">assistant-context.json</CardTitle></CardHeader><CardContent className="p-6 font-mono text-xs leading-7 text-muted-foreground"><p><span className="text-primary">question</span>: what calls CheckoutHandler?</p><p><span className="text-primary">paths</span>: 3 · depth: 1..4</p><p><span className="text-primary">answer</span>: PaymentGateway, Inventory.reserve</p><p><span className="text-primary">evidence</span>: source anchors + direct edges</p></CardContent></Card>
        <div>
          <p className="eyebrow">Integration choices</p>
          <h2 className="mt-4 font-heading text-3xl font-semibold tracking-[-0.05em]">Fit the boundary to the tool.</h2>
          <p className="mt-5 text-base leading-7 text-muted-foreground">Use the CLI in scripts, MCP for a local tool boundary, native skills for repeatable workflows, or the shared viewer when a human needs to inspect the answer.</p>
          <div className="mt-6 flex flex-wrap gap-2 font-mono text-xs text-muted-foreground"><span className="rounded-full border border-border bg-card px-3 py-2">CLI</span><span className="rounded-full border border-border bg-card px-3 py-2">MCP</span><span className="rounded-full border border-border bg-card px-3 py-2">VS Code</span><span className="rounded-full border border-border bg-card px-3 py-2">JSONL</span></div>
        </div>
      </div>
    </section>
  </MarketingPage>;
}

function AssistantStep({ index, title, text }: { index: string; title: string; text: string }) {
  return <Card className="border-border/80 bg-card/70 shadow-none"><CardContent className="flex min-h-40 flex-col justify-between gap-6 p-6"><span className="font-mono text-xs text-primary">{index}</span><div><p className="font-heading text-xl font-semibold tracking-[-0.04em]">{title}</p><p className="mt-2 text-sm leading-6 text-muted-foreground">{text}</p></div></CardContent></Card>;
}
