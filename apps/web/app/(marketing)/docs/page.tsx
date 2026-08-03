import Link from 'next/link';
import type { Metadata } from 'next';
import { ArrowRightIcon, BookOpenIcon, BoxesIcon, Code2Icon, ShieldCheckIcon } from 'lucide-react';

import { Badge } from '@/components/ui/badge';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';

const paths = [
  { icon: BookOpenIcon, eyebrow: 'Evaluate', title: 'Understand the product', description: 'Start with the getting started guide, the pipeline, graph model, and security boundary.', href: '/docs/README' },
  { icon: BoxesIcon, eyebrow: 'Use', title: 'Get useful answers', description: 'Explore a codebase, trace impact, integrate Compass, or set up the editor workflow.', href: '/docs/getting-started' },
  { icon: Code2Icon, eyebrow: 'Query', title: 'Learn CompassQL', description: 'Read the language contract and support matrix before you automate structural checks.', href: '/docs/COMPASSQL' },
  { icon: ShieldCheckIcon, eyebrow: 'Contribute', title: 'Work on Compass', description: 'Follow the architecture, language, implementation, and extension boundaries.', href: '/docs/implementation/workspace-tour' },
];

export const metadata: Metadata = {
  title: 'Documentation',
  description: 'Learn, use, query, and contribute to Compass through the canonical project documentation.',
};

export default function DocsLandingPage() {
  return (
    <>
      <section className="relative overflow-hidden border-b border-border/70">
        <div className="site-grid pointer-events-none absolute inset-0 opacity-50" aria-hidden="true" />
        <div className="relative mx-auto max-w-7xl px-5 pb-16 pt-20 lg:px-8 lg:pb-24 lg:pt-28">
          <Badge className="rounded-full px-3 py-1 font-mono text-[0.68rem] uppercase tracking-[0.14em]" variant="outline">Compass documentation</Badge>
          <h1 className="mt-6 max-w-4xl font-heading text-[clamp(3rem,7vw,6.4rem)] font-semibold leading-[0.94] tracking-[-0.075em]">A clear route into a complex workspace.</h1>
          <p className="mt-7 max-w-2xl text-lg leading-8 text-muted-foreground">Choose the path that matches the question in front of you. The pages below are backed by the repository&apos;s canonical documentation.</p>
        </div>
      </section>
      <section className="mx-auto max-w-7xl px-5 py-16 lg:px-8 lg:py-24">
        <div className="grid gap-5 md:grid-cols-2">
          {paths.map(({ icon: Icon, ...path }) => (
            <Card className="group border-border/80 bg-card/70 shadow-none transition-transform duration-300 hover:-translate-y-1" key={path.href}>
              <CardHeader className="gap-4"><Icon className="text-primary" /><div className="flex flex-col gap-2"><span className="eyebrow">{path.eyebrow}</span><CardTitle className="font-heading text-2xl tracking-[-0.045em]">{path.title}</CardTitle></div></CardHeader>
              <CardContent className="flex flex-col gap-6"><p className="max-w-lg text-sm leading-7 text-muted-foreground">{path.description}</p><Link className="inline-flex items-center gap-2 text-sm font-medium text-primary transition-[gap] group-hover:gap-3" href={path.href}>Open path <ArrowRightIcon data-icon="inline-end" /></Link></CardContent>
            </Card>
          ))}
        </div>
      </section>
    </>
  );
}
