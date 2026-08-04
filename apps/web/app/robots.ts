import type { MetadataRoute } from 'next';

import { isProductionDeployment, siteUrl } from '@/lib/site';

export default function robots(): MetadataRoute.Robots {
  if (!isProductionDeployment) {
    return {
      rules: { userAgent: '*', disallow: '/' },
      host: new URL(siteUrl).host,
    };
  }

  return {
    rules: [
      {
        userAgent: '*',
        allow: '/',
        disallow: ['/api/'],
      },
    ],
    sitemap: `${siteUrl}/sitemap.xml`,
    host: new URL(siteUrl).host,
  };
}
