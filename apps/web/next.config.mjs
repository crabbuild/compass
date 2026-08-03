import { createMDX } from 'fumadocs-mdx/next';

/** @type {import('next').NextConfig} */
const config = {
  reactStrictMode: true,
  transpilePackages: ['@compass/viewer', 'fumadocs-core', 'fumadocs-ui'],
  async redirects() {
    return [
      {
        source: '/docs/:path*.md',
        destination: '/docs/:path*',
        permanent: true,
      },
      {
        source: '/blog/meet-compass-local-map',
        destination: '/blog/meet-compass',
        permanent: true,
      },
      {
        source: '/COMPATIBILITY.md',
        destination: '/docs/reference/compatibility',
        permanent: true,
      },
      {
        source: '/SECURITY.md',
        destination: 'https://github.com/crabbuild/compass/blob/main/SECURITY.md',
        permanent: true,
      },
      {
        source: '/MIGRATION.md',
        destination: 'https://github.com/crabbuild/compass/blob/main/MIGRATION.md',
        permanent: true,
      },
      {
        source: '/PERFORMANCE.md',
        destination: 'https://github.com/crabbuild/compass/blob/main/PERFORMANCE.md',
        permanent: true,
      },
      {
        source: '/SUPPORT.md',
        destination: 'https://github.com/crabbuild/compass/blob/main/SUPPORT.md',
        permanent: true,
      },
      {
        source: '/CONTRIBUTING.md',
        destination: 'https://github.com/crabbuild/compass/blob/main/CONTRIBUTING.md',
        permanent: true,
      },
      {
        source: '/CODE_OF_CONDUCT.md',
        destination: 'https://github.com/crabbuild/compass/blob/main/CODE_OF_CONDUCT.md',
        permanent: true,
      },
    ];
  },
};

const withMDX = createMDX();

export default withMDX(config);
