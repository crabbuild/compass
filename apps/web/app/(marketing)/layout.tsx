import type { ReactNode } from 'react';

import { SiteFooter } from '@/components/site-footer';
import { SiteHeader } from '@/components/site-header';

export default function MarketingLayout({ children }: { children: ReactNode }) {
  return (
    <div className="flex min-h-screen flex-col">
      <SiteHeader />
      <main className="flex-1">{children}</main>
      <SiteFooter />
    </div>
  );
}
