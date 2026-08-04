import type { ComponentProps } from 'react';
import type { Metadata } from 'next';
import { notFound } from 'next/navigation';
import { DocsBody, DocsDescription, DocsPage, DocsTitle } from 'fumadocs-ui/layouts/docs/page';
import { createRelativeLink } from 'fumadocs-ui/mdx';

import { getMDXComponents } from '@/components/mdx';
import { source } from '@/lib/source';
import { JsonLd } from '@/components/structured-data';
import { breadcrumbJsonLd, docsArticleJsonLd } from '@/lib/seo';
import { pageMetadata } from '@/lib/site';

export function generateStaticParams() {
  return source.generateParams();
}

export async function generateMetadata(props: {
  params: Promise<{ slug: string[] }>;
}): Promise<Metadata> {
  const params = await props.params;
  const page = source.getPage(params.slug);
  if (!page) return {};

  const description = page.data.description || `Learn ${page.data.title} in the Compass documentation.`;

  return pageMetadata(page.data.title, description, {
    path: page.url,
    keywords: ['Compass documentation', 'code graph', page.data.title],
  });
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
    <>
      <JsonLd
        data={breadcrumbJsonLd([
          { name: 'Compass', path: '/' },
          { name: 'Documentation', path: '/docs' },
          { name: page.data.title, path: page.url },
        ])}
      />
      <JsonLd
        data={docsArticleJsonLd({
          title: page.data.title,
          description: page.data.description || `Learn ${page.data.title} in the Compass documentation.`,
          path: page.url,
        })}
      />
      <DocsPage toc={page.data.toc} full={page.data.full}>
        <DocsTitle>{page.data.title}</DocsTitle>
        <DocsDescription>{page.data.description || `Learn ${page.data.title} in the Compass documentation.`}</DocsDescription>
        <DocsBody>
          <MDX
            components={getMDXComponents({
              a: docsLink,
            })}
          />
        </DocsBody>
      </DocsPage>
    </>
  );
}
