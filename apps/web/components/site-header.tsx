import Link from 'next/link';
import { GithubIcon, MenuIcon, StarIcon } from 'lucide-react';

import { CompassLockup } from '@/components/compass-mark';
import { ThemeToggle } from '@/components/theme-toggle';
import { buttonVariants } from '@/components/ui/button';
import { formatGitHubStarCount, getGitHubStarCount, githubRepositoryUrl } from '@/lib/github';
import {
  Sheet,
  SheetClose,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
  SheetTrigger,
} from '@/components/ui/sheet';
import { cn } from '@/lib/utils';

const links = [
  { href: '/product', label: 'Product' },
  { href: '/use-cases', label: 'Use cases' },
  { href: '/integrations', label: 'Integrations' },
  { href: '/docs', label: 'Docs' },
  { href: '/blog', label: 'Blog' },
];

export async function SiteHeader() {
  const starCount = await getGitHubStarCount();
  const formattedStarCount = starCount === null ? null : formatGitHubStarCount(starCount);

  return (
    <header className="sticky top-0 z-40 border-b border-border/70 bg-background/90 backdrop-blur-xl">
      <div className="mx-auto flex h-16 max-w-7xl items-center justify-between gap-4 px-5 lg:px-8">
        <Link className="shrink-0" href="/" aria-label="Compass home">
          <CompassLockup />
        </Link>

        <nav className="hidden items-center gap-1 md:flex" aria-label="Primary navigation">
          {links.map((link) => (
            <Link
              className="rounded-md px-3 py-2 text-sm text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
              href={link.href}
              key={link.href}
            >
              {link.label}
            </Link>
          ))}
        </nav>

        <div className="hidden items-center gap-2 md:flex">
          <ThemeToggle />
          <Link
            className="inline-flex items-center gap-1.5 px-3 py-2 text-sm text-muted-foreground transition-colors hover:text-foreground"
            href={githubRepositoryUrl}
            target="_blank"
            rel="noreferrer"
            aria-label={formattedStarCount ? `Open Compass on GitHub, ${formattedStarCount} stars` : 'Open Compass on GitHub'}
          >
            <GithubIcon aria-hidden="true" className="size-4" />
            GitHub
            {formattedStarCount && (
              <span className="ml-1 inline-flex items-center gap-1 font-mono text-xs tabular-nums text-muted-foreground/80" title={`${formattedStarCount} GitHub stars`}>
                <StarIcon aria-hidden="true" className="size-3.5 fill-current" />
                {formattedStarCount}
              </span>
            )}
          </Link>
          <Link className={cn(buttonVariants({ size: 'sm' }))} href="/install">
            Install Compass
          </Link>
        </div>

        <div className="flex items-center gap-1 md:hidden">
          <ThemeToggle />
          <Sheet>
            <SheetTrigger
              render={
                <button
                  aria-label="Open navigation"
                  className={buttonVariants({ size: 'icon-sm', variant: 'outline' })}
                  type="button"
                >
                  <MenuIcon />
                </button>
              }
            />
            <SheetContent className="w-[min(88vw,22rem)]" side="right">
              <SheetHeader className="border-b border-border/70">
                <SheetTitle>
                  <CompassLockup />
                </SheetTitle>
                <SheetDescription>Navigate the Compass site.</SheetDescription>
              </SheetHeader>
              <nav className="flex flex-col gap-1 px-4 py-3" aria-label="Mobile navigation">
                {links.map((link) => (
                  <SheetClose
                    key={link.href}
                    render={
                      <Link
                        className="rounded-lg px-3 py-3 text-base text-foreground transition-colors hover:bg-muted"
                        href={link.href}
                      />
                    }
                  >
                    {link.label}
                  </SheetClose>
                ))}
                <SheetClose
                  render={
                    <Link className={cn(buttonVariants({ className: 'mt-3 w-full' }))} href="/install" />
                  }
                >
                  Install Compass
                </SheetClose>
              </nav>
            </SheetContent>
          </Sheet>
        </div>
      </div>
    </header>
  );
}
