import type { BaseLayoutProps } from 'fumadocs-ui/layouts/shared';

import { CompassLockup } from '@/components/compass-mark';

export function baseOptions(): BaseLayoutProps {
  return {
    nav: {
      title: <CompassLockup />,
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
