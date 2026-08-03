import type { BaseLayoutProps } from 'fumadocs-ui/layouts/shared';

export function baseOptions(): BaseLayoutProps {
  return {
    nav: {
      title: 'Compass',
    },
    links: [
      {
        text: 'Website',
        url: '/',
      },
      {
        text: 'GitHub',
        url: 'https://github.com/crabbuild/compass',
        external: true,
      },
    ],
  };
}
