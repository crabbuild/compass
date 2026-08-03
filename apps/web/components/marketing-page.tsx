import Link from 'next/link';
import { ArrowRightIcon } from 'lucide-react';
import type { ReactNode } from 'react';

import { SectionHeading } from '@/components/section-heading';
import { Badge } from '@/components/ui/badge';
import { buttonVariants } from '@/components/ui/button';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { cn } from '@/lib/utils';

export function MarketingPage({
  eyebrow,
  title,
  description,
  children,
  ctaHref = '/install',
  ctaLabel = 'Install Compass',
}: {
  eyebrow: string;
  title: string;
  description: string;
  children: ReactNode;
  ctaHref?: string;
  ctaLabel?: string;
}) {
  return (
    <>
      <section className="relative overflow-hidden border-b border-border/70">
        <div className="site-grid pointer-events-none absolute inset-0 opacity-50" aria-hidden="true" />
        <div className="relative mx-auto max-w-7xl px-5 pb-16 pt-20 lg:px-8 lg:pb-24 lg:pt-28">
          <Badge className="rounded-full px-3 py-1 font-mono text-[0.68rem] uppercase tracking-[0.14em]" variant="outline">{eyebrow}</Badge>
          <h1 className="mt-6 max-w-4xl font-heading text-[clamp(3rem,7vw,6.4rem)] font-semibold leading-[0.94] tracking-[-0.075em]">{title}</h1>
          <p className="mt-7 max-w-2xl text-lg leading-8 text-muted-foreground">{description}</p>
          <Link className={cn(buttonVariants({ size: 'lg' }), 'mt-9 gap-2 px-5')} href={ctaHref}>
            {ctaLabel}<ArrowRightIcon data-icon="inline-end" />
          </Link>
        </div>
      </section>
      {children}
    </>
  );
}

export function FeatureGrid({ items }: { items: Array<{ eyebrow: string; title: string; description: string; href?: string }> }) {
  return (
    <div className="grid gap-5 md:grid-cols-2 lg:grid-cols-3">
      {items.map((item) => (
        <Card className="border-border/80 bg-card/70 shadow-none" key={item.title}>
          <CardHeader className="gap-3">
            <span className="eyebrow">{item.eyebrow}</span>
            <CardTitle className="font-heading text-xl tracking-[-0.04em]">{item.title}</CardTitle>
          </CardHeader>
          <CardContent className="flex flex-col gap-5">
            <p className="text-[0.95rem] leading-7 text-muted-foreground">{item.description}</p>
            {item.href && <Link className="inline-flex items-center gap-2 text-sm font-medium text-primary" href={item.href}>Read more <ArrowRightIcon data-icon="inline-end" /></Link>}
          </CardContent>
        </Card>
      ))}
    </div>
  );
}

export function PageSection({ eyebrow, title, description, children }: { eyebrow: string; title: string; description?: string; children: ReactNode }) {
  return (
    <section className="mx-auto max-w-7xl px-5 py-20 lg:px-8 lg:py-28">
      <SectionHeading eyebrow={eyebrow} title={title} description={description} />
      <div className="mt-12">{children}</div>
    </section>
  );
}
