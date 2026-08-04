import type { MetadataRoute } from 'next';

export default function manifest(): MetadataRoute.Manifest {
  return {
    name: 'Compass — local-first code intelligence',
    short_name: 'Compass',
    description:
      'Explore codebases as local, evidence-backed graphs with Compass.',
    id: '/',
    start_url: '/',
    scope: '/',
    display: 'browser',
    background_color: '#f5f7ff',
    theme_color: '#5865f2',
    lang: 'en-US',
    icons: [
      {
        src: '/brand/compass-mark.svg',
        sizes: '128x128',
        type: 'image/svg+xml',
        purpose: 'any',
      },
    ],
  };
}
