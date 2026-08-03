import type { Metadata } from 'next';

export const siteUrl = (process.env.NEXT_PUBLIC_SITE_URL ?? 'https://compass.crabbuild.dev').replace(/\/$/, '');

export function pageMetadata(title: string, description: string): Metadata {
  const fullTitle = `${title} · Compass`;

  return {
    title,
    description,
    openGraph: {
      type: 'website',
      title: fullTitle,
      description,
    },
    twitter: {
      card: 'summary',
      title: fullTitle,
      description,
    },
  };
}
