import {
  ArrowRightIcon,
  CheckCircle2Icon,
  DownloadIcon,
  ExternalLinkIcon,
  GithubIcon,
  MonitorDownIcon,
  TerminalIcon,
} from 'lucide-react';
import Link from 'next/link';

import { InstallCommand, PlatformInstaller } from '@/components/install-command';
import { MarketingPage, PageSection } from '@/components/marketing-page';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { buttonVariants } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import { pageMetadata } from '@/lib/site';

export const metadata = pageMetadata('Install', 'Install the Compass CLI on macOS, Linux, or Windows, then add the Compass Codegraph extension to VS Code.');

const vscodeMarketplaceUrl = 'https://marketplace.visualstudio.com/items?itemName=crabbuild.crabbuild-compass-vscode';
const vscodeInstallCommand = 'code --install-extension crabbuild.crabbuild-compass-vscode';

export default function InstallPage() {
  return (
    <MarketingPage
      ctaHref="#cli-install"
      ctaLabel="Choose your platform"
      description="Install the local CLI first, then add Compass Codegraph to VS Code. Your source and graph stay on your machine."
      eyebrow="Install Compass"
      title="From terminal to code graph in two steps."
    >
      <div className="scroll-mt-20" id="cli-install">
        <PageSection
          description="Pick your operating system to get the right official command. The installer detects your CPU architecture and verifies the downloaded release."
          eyebrow="Step 1 · Compass CLI"
          title="Install the local engine."
        >
          <PlatformInstaller />
        </PageSection>
      </div>

      <section className="scroll-mt-20 border-y border-border/70 bg-muted/25" id="vscode-extension">
        <div className="mx-auto grid max-w-7xl gap-10 px-5 py-20 lg:grid-cols-[0.82fr_1.18fr] lg:items-center lg:gap-16 lg:px-8 lg:py-28">
          <div>
            <p className="eyebrow">Step 2 · VS Code</p>
            <h2 className="mt-4 max-w-xl font-heading text-4xl font-semibold leading-[1.02] tracking-[-0.06em] sm:text-5xl">Bring the graph into your editor.</h2>
            <p className="mt-6 max-w-xl text-base leading-8 text-muted-foreground">
              Compass Codegraph connects to the CLI you just installed. Explore relationships, trace callers and callees, inspect change impact, and open exact source locations without leaving VS Code.
            </p>
            <div className="mt-8 flex flex-wrap gap-3">
              <Link
                className={cn(buttonVariants({ variant: 'default', size: 'lg' }), 'gap-2 px-4')}
                href={vscodeMarketplaceUrl}
                target="_blank"
                rel="noreferrer"
              >
                Install from Marketplace <ExternalLinkIcon data-icon="inline-end" />
              </Link>
              <Link className={cn(buttonVariants({ variant: 'outline', size: 'lg' }), 'gap-2 px-4')} href="/docs/guides/vscode">
                Read the setup guide <ArrowRightIcon data-icon="inline-end" />
              </Link>
            </div>
          </div>

          <Card className="border-border/80 bg-card/85 shadow-[0_28px_70px_-50px_color-mix(in_oklch,var(--foreground)_45%,transparent)]">
            <CardHeader className="gap-3 border-b border-border/70 pb-5">
              <span className="grid size-10 place-items-center rounded-lg bg-primary text-primary-foreground"><MonitorDownIcon className="size-5" /></span>
              <CardTitle className="font-heading text-2xl tracking-[-0.045em]">Install the VS Code extension</CardTitle>
              <CardDescription>Install Compass Codegraph from the Visual Studio Marketplace. It requires Compass CLI 0.3.0 or newer.</CardDescription>
            </CardHeader>
            <CardContent className="space-y-6 pt-2">
              <ol className="space-y-5">
                <VscodeStep number="1" title="Install Compass Codegraph" text="Open the Marketplace listing and select Install. VS Code handles the download and future updates." />
                <VscodeStep number="2" title="Open a trusted repository" text="Use a local folder or connect through a supported VS Code remote environment." />
                <VscodeStep number="3" title="Initialize your first graph" text="Select Compass in the activity bar, then choose Initialize repository." />
              </ol>
              <div className="rounded-xl border border-border/70 bg-muted/45 p-4">
                <p className="mb-3 text-sm font-medium">Prefer the terminal? Install with the extension ID:</p>
                <InstallCommand ariaLabel="Copy VS Code extension install command" command={vscodeInstallCommand} prompt=">" />
              </div>
              <div className="flex items-start gap-2 text-sm leading-6 text-muted-foreground">
                <CheckCircle2Icon className="mt-0.5 size-4 shrink-0 text-primary" />
                <p>The extension auto-detects Compass on PATH and in common user-local install locations.</p>
              </div>
            </CardContent>
          </Card>
        </div>
      </section>

      <PageSection eyebrow="Other install paths" title="Need a manual route?">
        <div className="grid gap-6 lg:grid-cols-2">
          <Card className="border-border/80 bg-card/70 shadow-none">
            <CardHeader className="gap-3">
              <DownloadIcon className="text-primary" />
              <CardTitle className="font-heading text-xl tracking-[-0.04em]">Offline release install</CardTitle>
            </CardHeader>
            <CardContent className="flex flex-col gap-4 text-sm leading-7 text-muted-foreground">
              <p>Download the matching archive and checksum from the latest release, verify it, and add Compass to your PATH.</p>
              <Link className="inline-flex items-center gap-2 font-medium text-primary" href="https://github.com/crabbuild/compass/releases/latest" target="_blank" rel="noreferrer">View releases <GithubIcon data-icon="inline-end" /></Link>
            </CardContent>
          </Card>
          <Card className="border-border/80 bg-card/70 shadow-none">
            <CardHeader className="gap-3">
              <TerminalIcon className="text-primary" />
              <CardTitle className="font-heading text-xl tracking-[-0.04em]">Build from source</CardTitle>
            </CardHeader>
            <CardContent className="flex flex-col gap-4 text-sm leading-7 text-muted-foreground">
              <code className="overflow-x-auto rounded-lg border border-border bg-background px-4 py-3 font-mono text-xs whitespace-nowrap text-foreground">cargo install --locked --path crates/compass-cli --bin compass</code>
              <span>Use the pinned Rust 1.97.1+ toolchain documented by the repository.</span>
              <Link className={cn(buttonVariants({ variant: 'outline', size: 'sm' }), 'w-fit')} href="/docs/getting-started">Read getting started</Link>
            </CardContent>
          </Card>
        </div>
      </PageSection>
    </MarketingPage>
  );
}

function VscodeStep({ number, title, text }: { number: string; title: string; text: string }) {
  return (
    <li className="grid grid-cols-[2rem_1fr] gap-3">
      <span className="grid size-8 place-items-center rounded-full border border-primary/25 bg-primary/[0.07] font-mono text-xs font-semibold text-primary">{number}</span>
      <div>
        <p className="font-heading font-semibold tracking-[-0.025em]">{title}</p>
        <p className="mt-1 text-sm leading-6 text-muted-foreground">{text}</p>
      </div>
    </li>
  );
}
