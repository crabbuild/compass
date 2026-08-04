import Link from 'next/link';
import { GithubIcon, StarIcon } from 'lucide-react';

import { CompassLockup } from '@/components/compass-mark';
import { formatGitHubStarCount, getGitHubStarCount, githubRepositoryUrl } from '@/lib/github';

export async function SiteFooter() {
  const starCount = await getGitHubStarCount();
  const formattedStarCount = starCount === null ? null : formatGitHubStarCount(starCount);

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
          <Link
            className="inline-flex items-center gap-2 text-sm text-muted-foreground hover:text-foreground"
            href={githubRepositoryUrl}
            target="_blank"
            rel="noreferrer"
            aria-label={formattedStarCount ? `Open Compass on GitHub, ${formattedStarCount} stars` : 'Open Compass on GitHub'}
          >
            <GithubIcon aria-hidden="true" className="size-4" />
            <span>GitHub</span>
            {formattedStarCount && (
              <span className="inline-flex items-center gap-1 font-mono text-xs tabular-nums text-muted-foreground/80" title={`${formattedStarCount} GitHub stars`}>
                <StarIcon aria-hidden="true" className="size-3.5 fill-current" />
                {formattedStarCount}
              </span>
            )}
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
