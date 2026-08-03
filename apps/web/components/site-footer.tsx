import Link from 'next/link';
import { ArrowUpRightIcon } from 'lucide-react';

import { CompassLockup } from '@/components/compass-mark';

export function SiteFooter() {
  return (
    <footer className="border-t border-border/70 bg-muted/25">
      <div className="mx-auto grid max-w-7xl gap-10 px-5 py-12 lg:grid-cols-[1.4fr_1fr_1fr_1fr] lg:px-8">
        <div className="flex flex-col gap-4">
          <Link href="/" aria-label="Compass home">
            <CompassLockup />
          </Link>
          <p className="max-w-xs text-sm leading-6 text-muted-foreground">
            A local-first knowledge graph for understanding codebases before changing them.
          </p>
          <span className="font-mono text-xs text-muted-foreground">compass / native / inspectable</span>
        </div>
        <FooterColumn
          title="Explore"
          links={[
            ['Product', '/product'],
            ['Use cases', '/use-cases'],
            ['Integrations', '/integrations'],
            ['Roadmap', '/roadmap'],
          ]}
        />
        <FooterColumn
          title="Learn"
          links={[
            ['Documentation', '/docs'],
            ['Blog', '/blog'],
            ['Install', '/install'],
            ['Security', '/security'],
          ]}
        />
        <div className="flex flex-col gap-3">
          <span className="eyebrow">Project</span>
          <Link className="inline-flex items-center gap-1.5 text-sm text-muted-foreground hover:text-foreground" href="https://github.com/crabbuild/compass" target="_blank" rel="noreferrer">
            GitHub <ArrowUpRightIcon data-icon="inline-end" />
          </Link>
          <Link className="text-sm text-muted-foreground hover:text-foreground" href="/about">About Compass</Link>
          <Link className="text-sm text-muted-foreground hover:text-foreground" href="/changelog">Changelog</Link>
        </div>
      </div>
      <div className="mx-auto flex max-w-7xl flex-col gap-2 border-t border-border/60 px-5 py-5 text-xs text-muted-foreground sm:flex-row sm:items-center sm:justify-between lg:px-8">
        <span>Built for people who need the edges to make sense.</span>
        <span>© {new Date().getFullYear()} Compass contributors</span>
      </div>
    </footer>
  );
}

function FooterColumn({ title, links }: { title: string; links: string[][] }) {
  return (
    <div className="flex flex-col gap-3">
      <span className="eyebrow">{title}</span>
      {links.map(([label, href]) => (
        <Link className="text-sm text-muted-foreground transition-colors hover:text-foreground" href={href} key={href}>
          {label}
        </Link>
      ))}
    </div>
  );
}
