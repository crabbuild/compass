import type { Metadata } from 'next';
import { notFound } from 'next/navigation';
import { ArrowLeftIcon } from 'lucide-react';
import Link from 'next/link';
import { DocsBody } from 'fumadocs-ui/layouts/docs/page';

import { getMDXComponents } from '@/components/mdx';
import { blogSource } from '@/lib/blog';

export function generateStaticParams() {
  return blogSource.getPages().map((page) => ({ slug: page.slugs[0] }));
}

export async function generateMetadata(props: { params: Promise<{ slug: string }> }): Promise<Metadata> {
  const { slug } = await props.params;
  const post = blogSource.getPage([slug]);
  if (!post) return {};
  return { title: post.data.title, description: post.data.description, alternates: { canonical: post.url } };
}

export default async function BlogPostPage(props: { params: Promise<{ slug: string }> }) {
  const { slug } = await props.params;
  const post = blogSource.getPage([slug]);
  if (!post || post.data.draft) notFound();
  const MDX = post.data.body;

  return <article className="mx-auto max-w-4xl px-5 py-16 lg:px-8 lg:py-24"><Link className="inline-flex items-center gap-2 text-sm text-muted-foreground hover:text-foreground" href="/blog"><ArrowLeftIcon data-icon="inline-start" /> All notes</Link><header className="mt-12 border-b border-border/70 pb-10"><p className="eyebrow">{post.data.tags.join(' / ') || 'Compass'}</p><h1 className="mt-5 font-heading text-4xl font-semibold leading-tight tracking-[-0.06em] sm:text-5xl">{post.data.title}</h1><p className="mt-5 text-lg leading-8 text-muted-foreground">{post.data.description}</p><p className="mt-7 font-mono text-xs text-muted-foreground">{post.data.author} · {new Intl.DateTimeFormat('en', { dateStyle: 'long' }).format(post.data.date)}</p></header><DocsBody className="mt-12"><MDX components={getMDXComponents()} /></DocsBody></article>;
}
