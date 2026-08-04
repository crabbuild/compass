import { pageMetadata } from '@/lib/site';
import { FeatureGrid, MarketingPage, PageSection } from '@/components/marketing-page';

export const metadata = pageMetadata('About', 'Learn why Compass is built as a local-first, deterministic, inspectable tool for understanding complex codebases.', { path: '/about' });

export default function AboutPage() {
  return <MarketingPage eyebrow="About Compass" title="A native tool for making complex systems legible." description="Compass is an independent local-first product inspired by the code-graph workflow: extract structure, preserve evidence, and make the next question smaller.">
    <PageSection eyebrow="Project principles" title="The product earns trust in the details.">
      <FeatureGrid items={[{ eyebrow: 'Local-first', title: 'Work where the code is', description: 'The default workflow has no remote service, hosted index, or required model credential.' }, { eyebrow: 'Deterministic', title: 'Equivalent inputs stay equivalent', description: 'Discovery, identity, ordering, canonical encoding, and output remain stable for equivalent inputs.' }, { eyebrow: 'Inspectable', title: 'Keep the edges visible', description: 'Relationships preserve direction, multiplicity, anchors, and provenance.' }]} />
    </PageSection>
  </MarketingPage>;
}
