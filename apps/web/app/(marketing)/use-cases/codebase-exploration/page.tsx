import { EvidenceDiagram } from '@/components/diagrams';
import { FeatureGrid, MarketingPage, PageSection } from '@/components/marketing-page';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { pageMetadata } from '@/lib/site';

export const metadata = pageMetadata('Codebase exploration', 'Build a mental model of an unfamiliar repository with communities, paths, provenance, and repeatable queries.');

export default function CodebaseExplorationPage() {
  return <MarketingPage eyebrow="Use case / exploration" title="Build a mental model before you build a plan." description="Compass helps you find the communities, boundaries, and paths that make a repository understandable when the README is not enough.">
    <PageSection eyebrow="A repeatable workflow" title="Four moves from unfamiliar to oriented.">
      <FeatureGrid items={[{ eyebrow: '01', title: 'Find the center', description: 'Start with communities and high-connectivity nodes to see where the system concentrates.' }, { eyebrow: '02', title: 'Follow a path', description: 'Trace directed relationships from an entry point to the implementation you need.' }, { eyebrow: '03', title: 'Check provenance', description: 'Inspect whether a relationship is direct, inferred, unresolved, or ambiguous.' }, { eyebrow: '04', title: 'Save the question', description: 'Turn the useful query into a repeatable CLI or CompassQL workflow.' }]} />
    </PageSection>
    <section className="border-y border-border/70 bg-muted/25">
      <div className="mx-auto grid max-w-7xl gap-8 px-5 py-20 lg:grid-cols-[0.9fr_1.1fr] lg:items-center lg:px-8 lg:py-28">
        <div>
          <p className="eyebrow">A Monday morning workflow</p>
          <h2 className="mt-4 font-heading text-3xl font-semibold tracking-[-0.05em] sm:text-4xl">Go from “where is this?” to a useful first hypothesis.</h2>
          <p className="mt-5 text-base leading-7 text-muted-foreground">Exploration is not about reading every file. It is about choosing a small, defensible path through the system and knowing where that path came from.</p>
        </div>
        <div className="grid gap-4 sm:grid-cols-2">
          <ExploreStep index="01" title="Open" text="load the local graph and scan communities" />
          <ExploreStep index="02" title="Hover" text="inspect node identity, path, and kind" />
          <ExploreStep index="03" title="Trace" text="follow one directed edge into the code" />
          <ExploreStep index="04" title="Record" text="save the query and source anchors" />
        </div>
      </div>
    </section>
    <PageSection eyebrow="Evidence on demand" title="The visual answer and the source answer stay connected." description="The same node and edge model powers the viewer, CompassQL output, and exported graph artifacts.">
      <div className="grid gap-8 lg:grid-cols-[1.1fr_0.9fr] lg:items-center">
        <EvidenceDiagram />
        <Card className="border-border/80 bg-card/70 shadow-none"><CardHeader><CardTitle className="font-heading text-2xl tracking-[-0.045em]">You leave with more than a picture.</CardTitle></CardHeader><CardContent className="flex flex-col gap-4 text-sm leading-7 text-muted-foreground"><span>• a shortlist of meaningful entry points</span><span>• a path you can explain to a teammate</span><span>• source anchors for the next edit</span><span>• a repeatable question for the next session</span></CardContent></Card>
      </div>
    </PageSection>
    <section className="border-t border-border/70 bg-card/45"><div className="mx-auto max-w-7xl px-5 py-20 lg:px-8 lg:py-28"><div className="grid gap-5 md:grid-cols-3"><Outcome title="Orient" text="Understand the shape before choosing a file." /><Outcome title="Explain" text="Share the path, not just the conclusion." /><Outcome title="Repeat" text="Turn discovery into a durable workflow." /></div></div></section>
  </MarketingPage>;
}

function ExploreStep({ index, title, text }: { index: string; title: string; text: string }) {
  return <Card className="border-border/80 bg-card/70 shadow-none"><CardContent className="flex min-h-32 flex-col justify-between gap-5 p-5"><span className="font-mono text-xs text-primary">{index}</span><div className="flex flex-col gap-1"><span className="font-heading text-lg font-semibold tracking-[-0.035em]">{title}</span><span className="text-sm leading-6 text-muted-foreground">{text}</span></div></CardContent></Card>;
}

function Outcome({ title, text }: { title: string; text: string }) {
  return <div className="rounded-xl border border-border/80 bg-card/70 p-6"><p className="font-heading text-xl font-semibold tracking-[-0.04em]">{title}</p><p className="mt-2 text-sm leading-6 text-muted-foreground">{text}</p></div>;
}
