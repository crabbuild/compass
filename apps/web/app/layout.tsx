import type { ReactNode } from 'react';
import type { Metadata, Viewport } from 'next';
import { IBM_Plex_Mono, Inter, Space_Grotesk } from 'next/font/google';
import { RootProvider } from 'fumadocs-ui/provider/next';

import './globals.css';
import { JsonLd } from '@/components/structured-data';
import {
  siteDescription,
  siteImagePath,
  siteKeywords,
  siteName,
  siteUrl,
  isProductionDeployment,
} from '@/lib/site';
import { siteJsonLd } from '@/lib/seo';

const spaceGrotesk = Space_Grotesk({
  subsets: ['latin'],
  variable: '--font-space-grotesk',
  display: 'swap',
});

const inter = Inter({
  subsets: ['latin'],
  variable: '--font-inter',
  display: 'swap',
});

const plexMono = IBM_Plex_Mono({
  subsets: ['latin'],
  variable: '--font-plex-mono',
  display: 'swap',
  weight: ['400', '500', '600'],
});

export const metadata: Metadata = {
  metadataBase: new URL(siteUrl),
  title: {
    default: 'Compass — understand the codebase before changing it',
    template: '%s · Compass',
  },
  description: siteDescription,
  keywords: siteKeywords,
  applicationName: siteName,
  generator: 'Next.js',
  referrer: 'origin-when-cross-origin',
  authors: [{ name: 'Compass contributors', url: 'https://github.com/crabbuild/compass' }],
  creator: siteName,
  publisher: siteName,
  category: 'Developer tools',
  alternates: {
    canonical: '/',
  },
  icons: {
    icon: [
      {
        url: '/brand/compass-mark.svg',
        type: 'image/svg+xml',
      },
    ],
    shortcut: ['/brand/compass-mark.svg'],
  },
  manifest: '/manifest.webmanifest',
  openGraph: {
    type: 'website',
    siteName,
    title: 'Compass — understand the codebase before changing it',
    description: siteDescription,
    url: '/',
    locale: 'en_US',
    images: [
      {
        url: siteImagePath,
        width: 1200,
        height: 630,
        alt: 'Compass local-first code graph for understanding a codebase.',
      },
    ],
  },
  twitter: {
    card: 'summary_large_image',
    title: 'Compass — understand the codebase before changing it',
    description: siteDescription,
    images: [
      {
        url: siteImagePath,
        width: 1200,
        height: 630,
        alt: 'Compass local-first code graph for understanding a codebase.',
      },
    ],
  },
  robots: {
    index: isProductionDeployment,
    follow: isProductionDeployment,
    googleBot: {
      index: isProductionDeployment,
      follow: isProductionDeployment,
      'max-image-preview': 'large',
      'max-snippet': -1,
      'max-video-preview': -1,
    },
  },
  verification: {
    ...(process.env.NEXT_PUBLIC_GOOGLE_SITE_VERIFICATION
      ? { google: process.env.NEXT_PUBLIC_GOOGLE_SITE_VERIFICATION }
      : {}),
    ...(process.env.NEXT_PUBLIC_BING_SITE_VERIFICATION
      ? { other: { 'msvalidate.01': process.env.NEXT_PUBLIC_BING_SITE_VERIFICATION } }
      : {}),
  },
};

export const viewport: Viewport = {
  colorScheme: 'light dark',
  themeColor: [
    { media: '(prefers-color-scheme: light)', color: '#f5f7ff' },
    { media: '(prefers-color-scheme: dark)', color: '#11131a' },
  ],
};

export default function RootLayout({ children }: Readonly<{ children: ReactNode }>) {
  return (
    <html lang="en" suppressHydrationWarning>
      <body className={`${spaceGrotesk.variable} ${inter.variable} ${plexMono.variable}`}>
        <RootProvider>
          <JsonLd data={siteJsonLd()} />
          {children}
        </RootProvider>
      </body>
    </html>
  );
}
