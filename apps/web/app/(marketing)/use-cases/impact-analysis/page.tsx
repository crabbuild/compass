import { ImpactPathDiagram } from '@/components/diagrams';
import { FeatureGrid, MarketingPage, PageSection } from '@/components/marketing-page';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { pageMetadata } from '@/lib/site';

export const metadata = pageMetadata('Impact analysis', 'Trace bounded, explainable change impact from a symbol through directed dependencies and source evidence.');

export default function ImpactAnalysisPage() {
  return <MarketingPage eyebrow="Use case / impact" title="See what a change can reach before it reaches production." description="Start at a symbol, walk directed incoming relationships, and keep the evidence for every affected target in view.">
    <PageSection eyebrow="Blast radius" title="Impact answers should be bounded and explainable.">
      <FeatureGrid items={[{ eyebrow: 'Select', title: 'Anchor on the changed thing', description: 'Use a source path, node identity, or exact query result to establish the starting point.' }, { eyebrow: 'Traverse', title: 'Walk incoming dependencies', description: 'Follow the graph in the direction that matches the question instead of scanning everything.' }, { eyebrow: 'Explain', title: 'Keep the path visible', description: 'Return source anchors, edge types, and provenance alongside each affected node.' }]} />
    </PageSection>
    <section className="border-y border-border/70 bg-muted/25">
      <div className="mx-auto max-w-7xl px-5 py-20 lg:px-8 lg:py-28">
        <ImpactPathDiagram />
      </div>
    </section>
    <PageSection eyebrow="A reviewable blast radius" title="Separate reachable from merely possible." description="Compass keeps traversal limits and evidence states beside the result, so a review can distinguish a direct dependency from an unresolved lead.">
      <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-4">
        <ImpactCheck title="Start" text="exact symbol, path, or node id" />
        <ImpactCheck title="Direction" text="incoming relationships for reachability" />
        <ImpactCheck title="Bounds" text="depth, fan-out, and result limits" />
        <ImpactCheck title="Evidence" text="edge type, anchor, and provenance" />
      </div>
    </PageSection>
    <section className="border-t border-border/70 bg-card/45">
      <div className="mx-auto grid max-w-7xl gap-8 px-5 py-20 lg:grid-cols-[0.88fr_1.12fr] lg:items-center lg:px-8 lg:py-28">
        <div>
          <p className="eyebrow">A change review in four lines</p>
          <h2 className="mt-4 font-heading text-3xl font-semibold tracking-[-0.05em]">The useful question is not “what depends on this?” It is “which path proves it?”</h2>
          <p className="mt-5 text-base leading-7 text-muted-foreground">Keep the graph result next to the diff. Reviewers can inspect the edge, jump to the source range, and decide whether the path is actionable.</p>
        </div>
        <Card className="overflow-hidden border-border/80 bg-compass-canvas-deep shadow-none"><CardHeader className="border-b border-border/70"><CardTitle className="font-mono text-sm tracking-normal">impact.json</CardTitle></CardHeader><CardContent className="p-6 font-mono text-xs leading-7 text-muted-foreground"><p><span className="text-primary">path</span>: payments.rs → Gateway → Checkout</p><p><span className="text-primary">edge</span>: CALLS · direct · line 42</p><p><span className="text-primary">bound</span>: depth 1..3 · limit 100</p><p><span className="text-primary">status</span>: reviewable · provenance retained</p></CardContent></Card>
      </div>
    </section>
  </MarketingPage>;
}

function ImpactCheck({ title, text }: { title: string; text: string }) {
  return <Card className="border-border/80 bg-card/70 shadow-none"><CardHeader className="gap-2"><span className="eyebrow">{title}</span><CardTitle className="font-heading text-lg tracking-[-0.035em]">{text}</CardTitle></CardHeader><CardContent><p className="text-sm leading-6 text-muted-foreground">Kept explicit in the query result.</p></CardContent></Card>;
}
