import type { ReactNode } from 'react';
import type { Metadata } from 'next';
import { IBM_Plex_Mono, Inter, Space_Grotesk } from 'next/font/google';
import { RootProvider } from 'fumadocs-ui/provider/next';

import './globals.css';
import { siteUrl } from '@/lib/site';

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
  description:
    'A fast, local-first knowledge graph for understanding codebases, tracing impact, and querying architecture with evidence.',
  applicationName: 'Compass',
  generator: 'Next.js',
  icons: {
    icon: '/brand/compass-mark.svg',
  },
  openGraph: {
    type: 'website',
    siteName: 'Compass',
    title: 'Compass — understand the codebase before changing it',
    description:
      'Turn a repository into a local, inspectable map of entities, relationships, provenance, and change impact.',
  },
};

export default function RootLayout({ children }: Readonly<{ children: ReactNode }>) {
  return (
    <html lang="en" suppressHydrationWarning>
      <body className={`${spaceGrotesk.variable} ${inter.variable} ${plexMono.variable}`}>
        <RootProvider>{children}</RootProvider>
      </body>
    </html>
  );
}
