import type { ComponentProps } from 'react';
import type { Metadata } from 'next';
import { notFound } from 'next/navigation';
import { DocsBody, DocsDescription, DocsPage, DocsTitle } from 'fumadocs-ui/layouts/docs/page';
import { createRelativeLink } from 'fumadocs-ui/mdx';

import { getMDXComponents } from '@/components/mdx';
import { source } from '@/lib/source';

export function generateStaticParams() {
  return source.generateParams();
}

export async function generateMetadata(props: {
  params: Promise<{ slug: string[] }>;
}): Promise<Metadata> {
  const params = await props.params;
  const page = source.getPage(params.slug);
  if (!page) return {};

  return {
    title: page.data.title,
    description: page.data.description,
    alternates: {
      canonical: page.url,
    },
  };
}

export default async function Page(props: { params: Promise<{ slug: string[] }> }) {
  const params = await props.params;
  const page = source.getPage(params.slug);
  if (!page) notFound();

  const MDX = page.data.body;
  const RelativeLink = createRelativeLink(source, page);
  const docsLink = async ({ href, ...linkProps }: ComponentProps<'a'>) => {
    const normalizedHref =
      typeof href === 'string' &&
      !href.startsWith('.') &&
      !href.startsWith('/') &&
      !href.startsWith('#') &&
      (href.split('#', 1)[0].endsWith('.md') || href.split('#', 1)[0].endsWith('.mdx'))
        ? `./${href}`
        : href;

    return RelativeLink({ ...linkProps, href: normalizedHref });
  };

  return (
    <DocsPage toc={page.data.toc} full={page.data.full}>
      <DocsTitle>{page.data.title}</DocsTitle>
      <DocsDescription>{page.data.description}</DocsDescription>
      <DocsBody>
        <MDX
          components={getMDXComponents({
            a: docsLink,
          })}
        />
      </DocsBody>
    </DocsPage>
  );
}
