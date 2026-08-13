import type { Metadata } from 'next';

export const siteName = 'Compass';
export const siteDescription =
  'A fast, local-first knowledge graph for understanding codebases, tracing impact, and querying architecture with evidence.';
export const siteUrl = (process.env.NEXT_PUBLIC_SITE_URL ?? 'https://compass.crab.build').replace(/\/$/, '');
export const isProductionDeployment = process.env.VERCEL_ENV !== 'preview' && process.env.VERCEL_ENV !== 'development';
// Keep the public social-card URL stable and image-like. Some link unfurlers
// are less reliable with Next.js' extensionless metadata image route, and a
// dedicated path also lets us invalidate a previously failed X card crawl.
export const siteImagePath = '/social-card.png';
export const siteKeywords = [
  'code graph',
  'codebase navigation',
  'repository intelligence',
  'software architecture',
  'impact analysis',
  'CompassQL',
  'local-first developer tools',
];

export function absoluteUrl(path = '/'): string {
  return new URL(path, siteUrl).toString();
}

export type PageMetadataOptions = {
  path?: string;
  keywords?: string[];
  image?: string;
  imageAlt?: string;
  type?: 'website' | 'article';
  publishedTime?: string;
  modifiedTime?: string;
  authors?: string[];
  tags?: string[];
  noIndex?: boolean;
};

export function pageMetadata(
  title: string,
  description: string,
  options: PageMetadataOptions = {},
): Metadata {
  const fullTitle = `${title} · Compass`;
  const path = options.path ?? '/';
  const image = options.image ?? siteImagePath;
  const imageAlt = options.imageAlt ?? `${title} — Compass`;
  const type = options.type ?? 'website';

  return {
    title,
    description,
    keywords: options.keywords ?? siteKeywords,
    alternates: {
      canonical: path,
    },
    openGraph: {
      type,
      title: fullTitle,
      description,
      url: path,
      siteName,
      locale: 'en_US',
      images: [
        {
          url: image,
          width: 1200,
          height: 630,
          alt: imageAlt,
        },
      ],
      ...(type === 'article'
        ? {
            publishedTime: options.publishedTime,
            modifiedTime: options.modifiedTime,
            authors: options.authors,
            tags: options.tags,
          }
        : {}),
    },
    twitter: {
      card: 'summary_large_image',
      title: fullTitle,
      description,
      images: [
        {
          url: image,
          width: 1200,
          height: 630,
          alt: imageAlt,
        },
      ],
    },
    robots: options.noIndex || !isProductionDeployment
      ? {
          index: false,
          follow: false,
        }
      : {
          index: true,
          follow: true,
          googleBot: {
            index: true,
            follow: true,
            'max-image-preview': 'large',
            'max-snippet': -1,
            'max-video-preview': -1,
          },
        },
  };
}
