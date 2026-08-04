import type { MetadataRoute } from 'next';

import { blogSource } from '@/lib/blog';
import { siteUrl } from '@/lib/site';
import { source } from '@/lib/source';

export default function sitemap(): MetadataRoute.Sitemap {
  const routes = [
    '',
    '/product',
    '/use-cases',
    '/integrations',
    '/security',
    '/roadmap',
    '/about',
    '/changelog',
    '/install',
    '/docs',
    '/blog',
  ];
  const staticEntries: MetadataRoute.Sitemap = routes.map((route) => ({
    url: `${siteUrl}${route}`,
    changeFrequency: route === '/blog' ? 'weekly' : 'monthly',
    priority: route === '' ? 1 : route === '/docs' || route === '/install' ? 0.9 : 0.7,
  }));

  const docsEntries = source.getPages().map((page) => ({
    url: `${siteUrl}${page.url}`,
    changeFrequency: 'monthly' as const,
    priority: 0.6,
  }));

  const blogEntries = blogSource.getPages()
    .filter((page) => !page.data.draft)
    .map((page) => ({
      url: `${siteUrl}${page.url}`,
      lastModified: page.data.date,
      changeFrequency: 'monthly' as const,
      priority: 0.6,
    }));

  const seen = new Set<string>();
  return [...staticEntries, ...docsEntries, ...blogEntries].filter((entry) => {
    if (seen.has(entry.url)) return false;
    seen.add(entry.url);
    return true;
  });
}
