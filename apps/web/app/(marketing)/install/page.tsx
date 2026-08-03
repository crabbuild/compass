import { AppleIcon, DownloadIcon, GithubIcon, LaptopIcon, TerminalIcon } from 'lucide-react';
import Link from 'next/link';

import { InstallCommand } from '@/components/install-command';
import { MarketingPage, PageSection } from '@/components/marketing-page';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { buttonVariants } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import { pageMetadata } from '@/lib/site';

export const metadata = pageMetadata('Install', 'Install Compass locally on macOS, Linux, or Windows, or build it from source with the pinned Rust toolchain.');

export default function InstallPage() {
  return <MarketingPage eyebrow="Install Compass" title="Start with a local graph in minutes." description="Use the verified release installer on macOS, Linux, or Windows—or build Compass from source with the pinned Rust toolchain.">
    <PageSection eyebrow="Choose your path" title="The first command should feel low-risk.">
      <div className="grid gap-5 md:grid-cols-3"><InstallCard icon={AppleIcon} title="macOS" text="Apple Silicon or Intel" /><InstallCard icon={TerminalIcon} title="Linux" text="ARM64 or AMD64" /><InstallCard icon={LaptopIcon} title="Windows" text="x64 or ARM64" /></div>
      <Card className="mt-6 max-w-3xl border-border/80 bg-card/70 shadow-none"><CardHeader><CardTitle className="font-heading text-xl tracking-[-0.04em]">macOS and Linux installer</CardTitle></CardHeader><CardContent><InstallCommand /></CardContent></Card>
    </PageSection>
    <section className="border-y border-border/70 bg-muted/25"><div className="mx-auto grid max-w-7xl gap-6 px-5 py-20 lg:grid-cols-2 lg:px-8 lg:py-28"><Card className="border-border/80 bg-card/70 shadow-none"><CardHeader className="gap-3"><DownloadIcon className="text-primary" /><CardTitle className="font-heading text-xl tracking-[-0.04em]">Offline release install</CardTitle></CardHeader><CardContent className="flex flex-col gap-4 text-sm leading-7 text-muted-foreground"><p>Download the matching archive and checksum from the latest release, verify it, and add Compass to your PATH.</p><Link className="inline-flex items-center gap-2 font-medium text-primary" href="https://github.com/crabbuild/compass/releases/latest" target="_blank" rel="noreferrer">View releases <GithubIcon data-icon="inline-end" /></Link></CardContent></Card><Card className="border-border/80 bg-card/70 shadow-none"><CardHeader className="gap-3"><TerminalIcon className="text-primary" /><CardTitle className="font-heading text-xl tracking-[-0.04em]">Build from source</CardTitle></CardHeader><CardContent className="flex flex-col gap-4 text-sm leading-7 text-muted-foreground"><code className="rounded-lg border border-border bg-background px-4 py-3 font-mono text-xs">cargo install --locked --path crates/compass-cli --bin compass</code><span>Use the pinned Rust 1.97.1+ toolchain documented by the repository.</span><Link className={cn(buttonVariants({ variant: 'outline', size: 'sm' }), 'w-fit')} href="/docs/getting-started">Read getting started</Link></CardContent></Card></div></section>
  </MarketingPage>;
}

function InstallCard({ icon: Icon, title, text }: { icon: typeof AppleIcon; title: string; text: string }) { return <Card className="border-border/80 bg-card/70 shadow-none"><CardContent className="flex items-center gap-4 p-6"><Icon className="text-primary" /><div className="flex flex-col gap-1"><span className="font-heading text-lg font-semibold tracking-[-0.03em]">{title}</span><span className="text-sm text-muted-foreground">{text}</span></div></CardContent></Card>; }
