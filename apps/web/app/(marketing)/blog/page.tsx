import type { Metadata } from 'next';
import Link from 'next/link';
import { ArrowRightIcon, CalendarDaysIcon } from 'lucide-react';

import { Badge } from '@/components/ui/badge';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { blogSource } from '@/lib/blog';

export const metadata: Metadata = {
  title: 'Blog',
  description: 'Stories about code graphs, provenance, local-first tooling, and building Compass.',
};

export default function BlogPage() {
  const posts = blogSource.getPages().filter((page) => !page.data.draft).sort((a, b) => b.data.date.getTime() - a.data.date.getTime());
  const [featured, ...rest] = posts;

  return (
    <>
      <section className="relative overflow-hidden border-b border-border/70">
        <div className="site-grid pointer-events-none absolute inset-0 opacity-50" aria-hidden="true" />
        <div className="relative mx-auto grid max-w-7xl gap-10 px-5 pb-16 pt-20 lg:grid-cols-[1fr_0.7fr] lg:items-end lg:px-8 lg:pb-24 lg:pt-28">
          <div>
            <p className="eyebrow">Compass writing</p>
            <h1 className="mt-5 max-w-4xl font-heading text-[clamp(3rem,7vw,6.4rem)] font-semibold leading-[0.94] tracking-[-0.075em]">Stories for complex systems.</h1>
            <p className="mt-7 max-w-2xl text-lg leading-8 text-muted-foreground">Product launches, implementation stories, and practical ways to make a codebase easier to navigate—one inspectable relationship at a time.</p>
          </div>
          <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-2">
            <BlogStat value={String(posts.length).padStart(2, '0')} label="stories published" />
            <BlogStat value="local" label="first by default" />
            <BlogStat value="∞" label="questions worth tracing" />
          </div>
        </div>
      </section>
      <section className="mx-auto max-w-7xl px-5 py-16 lg:px-8 lg:py-24">
        {featured ? (
          <div className="grid gap-6 lg:grid-cols-[1.25fr_0.75fr]">
            <FeaturedPost post={featured} />
            <div className="grid gap-4 sm:grid-cols-3 lg:grid-cols-1">
              <TopicCard eyebrow="Graph craft" title="Make the shape explainable." text="Architecture is more useful when every edge can lead back to source." />
              <TopicCard eyebrow="Local first" title="Keep the boundary close." text="No hosted index is required to ask a precise question about your code." />
              <TopicCard eyebrow="Shipping" title="Turn context into motion." text="A smaller answer gives the next change a cleaner starting point." />
            </div>
          </div>
        ) : <Card className="border-border/80 bg-card/70 shadow-none"><CardContent className="p-8 text-sm text-muted-foreground">New stories are being prepared.</CardContent></Card>}
      </section>
      {rest.length > 0 && <section className="border-y border-border/70 bg-muted/25"><div className="mx-auto max-w-7xl px-5 py-16 lg:px-8 lg:py-24"><div className="flex items-end justify-between gap-6"><div><p className="eyebrow">More stories</p><h2 className="mt-3 font-heading text-3xl font-semibold tracking-[-0.05em]">Build a better mental model.</h2></div><span className="hidden font-mono text-xs text-muted-foreground sm:block">{rest.length} more {rest.length === 1 ? 'story' : 'stories'}</span></div><div className="mt-10 grid gap-5 md:grid-cols-2">{rest.map((post) => <BlogCard key={post.url} post={post} />)}</div></div></section>}
      <section className="border-t border-border/70 bg-primary text-primary-foreground"><div className="mx-auto flex max-w-7xl flex-col gap-8 px-5 py-20 lg:flex-row lg:items-end lg:justify-between lg:px-8 lg:py-24"><div className="max-w-2xl"><p className="eyebrow text-primary-foreground/70">Keep exploring</p><h2 className="mt-4 font-heading text-4xl font-semibold tracking-[-0.06em] sm:text-5xl">The best context is close to the code.</h2><p className="mt-5 max-w-xl text-base leading-7 text-primary-foreground/75">Install Compass, open a graph, and turn one unanswered question into a path you can inspect.</p></div><Link className="inline-flex w-fit items-center gap-2 rounded-md bg-primary-foreground px-5 py-3 text-sm font-medium text-primary transition-colors hover:bg-primary-foreground/90" href="/install">Start with Compass <ArrowRightIcon data-icon="inline-end" /></Link></div></section>
    </>
  );
}

function FeaturedPost({ post }: { post: ReturnType<typeof blogSource.getPages>[number] }) {
  return <Card className="overflow-hidden border-border/80 bg-card/70 shadow-none"><CardHeader className="gap-6 border-b border-border/70 p-6 sm:p-8"><div className="flex flex-wrap items-center justify-between gap-3"><Badge>Featured story</Badge><span className="flex items-center gap-1.5 font-mono text-xs text-muted-foreground"><CalendarDaysIcon /> {formatDate(post.data.date)}</span></div><CardTitle className="max-w-3xl font-heading text-3xl tracking-[-0.05em] sm:text-4xl">{post.data.title}</CardTitle></CardHeader><CardContent className="flex flex-col gap-7 p-6 sm:p-8"><p className="max-w-2xl text-base leading-8 text-muted-foreground">{post.data.description}</p><div className="flex flex-wrap items-center gap-4"><Link className="inline-flex items-center gap-2 text-sm font-medium text-primary" href={post.url}>Read the story <ArrowRightIcon data-icon="inline-end" /></Link><span className="font-mono text-xs text-muted-foreground">{post.data.author}</span></div></CardContent></Card>;
}

function TopicCard({ eyebrow, title, text }: { eyebrow: string; title: string; text: string }) {
  return <Card className="border-border/80 bg-card/70 shadow-none"><CardContent className="flex h-full flex-col justify-between gap-6 p-5"><div><p className="eyebrow">{eyebrow}</p><p className="mt-3 font-heading text-xl font-semibold tracking-[-0.04em]">{title}</p></div><p className="text-sm leading-6 text-muted-foreground">{text}</p></CardContent></Card>;
}

function BlogCard({ post }: { post: ReturnType<typeof blogSource.getPages>[number] }) {
  return <Card className="group border-border/80 bg-card/70 shadow-none transition-transform duration-300 hover:-translate-y-1"><CardHeader className="gap-4"><div className="flex items-center justify-between gap-3"><Badge variant="outline">{post.data.tags[0] ?? 'Compass'}</Badge><span className="flex items-center gap-1.5 font-mono text-xs text-muted-foreground"><CalendarDaysIcon /> {formatDate(post.data.date)}</span></div><CardTitle className="max-w-lg font-heading text-2xl tracking-[-0.045em]">{post.data.title}</CardTitle></CardHeader><CardContent className="flex flex-col gap-6"><p className="text-sm leading-7 text-muted-foreground">{post.data.description}</p><Link className="inline-flex items-center gap-2 text-sm font-medium text-primary transition-[gap] group-hover:gap-3" href={post.url}>Read story <ArrowRightIcon data-icon="inline-end" /></Link></CardContent></Card>;
}

function BlogStat({ value, label }: { value: string; label: string }) {
  return <div className="rounded-xl border border-border/80 bg-card/70 p-4"><span className="font-heading text-2xl font-semibold tracking-[-0.05em] text-primary">{value}</span><span className="mt-2 block font-mono text-[0.62rem] uppercase tracking-[0.12em] text-muted-foreground">{label}</span></div>;
}

function formatDate(date: Date) { return new Intl.DateTimeFormat('en', { dateStyle: 'medium' }).format(date); }
