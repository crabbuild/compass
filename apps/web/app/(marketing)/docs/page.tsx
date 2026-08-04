import Link from 'next/link';
import type { Metadata } from 'next';
import { ArrowRightIcon, BookOpenIcon, BoxesIcon, Code2Icon, FileTextIcon } from 'lucide-react';

import { Badge } from '@/components/ui/badge';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { JsonLd } from '@/components/structured-data';
import { pageMetadata } from '@/lib/site';
import { breadcrumbJsonLd } from '@/lib/seo';

const paths = [
  { icon: BookOpenIcon, eyebrow: 'Start', title: 'Build your first graph', description: 'Install Compass, scan a repository, and answer your first structural question.', href: '/docs/getting-started', action: 'Start the guide' },
  { icon: BoxesIcon, eyebrow: 'Learn', title: 'Understand how Compass works', description: 'Learn how source files become an evidence-backed graph and what the results mean.', href: '/docs/concepts/how-it-works', action: 'Explore the concepts' },
  { icon: Code2Icon, eyebrow: 'Use', title: 'Complete a common task', description: 'Explore a codebase, trace change impact, connect an assistant, or automate a workflow.', href: '/docs/guides/exploring-a-codebase', action: 'Browse task guides' },
  { icon: FileTextIcon, eyebrow: 'Look up', title: 'Find commands and formats', description: 'Check exact commands, configuration options, output formats, and CompassQL syntax.', href: '/docs/reference/commands', action: 'Open the reference' },
];

export const metadata: Metadata = pageMetadata(
  'Documentation',
  'Install Compass, learn the core concepts, follow task guides, and look up commands and formats.',
  { path: '/docs' },
);

export default function DocsLandingPage() {
  return (
    <>
      <JsonLd
        data={breadcrumbJsonLd([
          { name: 'Compass', path: '/' },
          { name: 'Documentation', path: '/docs' },
        ])}
      />
      <section className="relative overflow-hidden border-b border-border/70">
        <div className="site-grid pointer-events-none absolute inset-0 opacity-50" aria-hidden="true" />
        <div className="relative mx-auto max-w-7xl px-5 pb-16 pt-20 lg:px-8 lg:pb-24 lg:pt-28">
          <Badge className="rounded-full px-3 py-1 font-mono text-[0.68rem] uppercase tracking-[0.14em]" variant="outline">Compass documentation</Badge>
          <h1 className="mt-6 max-w-4xl font-heading text-[clamp(3rem,7vw,6.4rem)] font-semibold leading-[0.94] tracking-[-0.075em]">Find an answer, then get back to your code.</h1>
          <p className="mt-7 max-w-2xl text-lg leading-8 text-muted-foreground">Start with a working graph, learn the ideas as you need them, or jump straight to a task or reference.</p>
        </div>
      </section>
      <section className="mx-auto max-w-7xl px-5 py-16 lg:px-8 lg:py-24">
        <div className="grid gap-5 md:grid-cols-2">
          {paths.map(({ icon: Icon, action, ...path }) => (
            <Card className="group border-border/80 bg-card/70 shadow-none transition-transform duration-300 hover:-translate-y-1" key={path.href}>
              <CardHeader className="gap-4"><Icon className="text-primary" /><div className="flex flex-col gap-2"><span className="eyebrow">{path.eyebrow}</span><CardTitle className="font-heading text-2xl tracking-[-0.045em]">{path.title}</CardTitle></div></CardHeader>
              <CardContent className="flex flex-col gap-6"><p className="max-w-lg text-sm leading-7 text-muted-foreground">{path.description}</p><Link className="inline-flex items-center gap-2 text-sm font-medium text-primary transition-[gap] group-hover:gap-3" href={path.href}>{action} <ArrowRightIcon data-icon="inline-end" /></Link></CardContent>
            </Card>
          ))}
        </div>
      </section>
    </>
  );
}
