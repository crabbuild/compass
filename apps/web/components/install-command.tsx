'use client';

import {
  AppleIcon,
  CheckCircle2Icon,
  CheckIcon,
  CopyIcon,
  LaptopIcon,
  TerminalIcon,
} from 'lucide-react';
import { useState } from 'react';

import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';

const shellCommand = "curl --proto '=https' --tlsv1.2 -LsSf https://github.com/crabbuild/compass/releases/latest/download/install.sh | sh";
const windowsCommand = 'irm https://github.com/crabbuild/compass/releases/latest/download/install.ps1 | iex';

const installers = [
  {
    id: 'macos',
    name: 'macOS',
    detail: 'Apple Silicon or Intel',
    terminal: 'Terminal',
    prompt: '$',
    command: shellCommand,
    icon: AppleIcon,
  },
  {
    id: 'linux',
    name: 'Linux',
    detail: 'ARM64 or AMD64',
    terminal: 'Terminal',
    prompt: '$',
    command: shellCommand,
    icon: TerminalIcon,
  },
  {
    id: 'windows',
    name: 'Windows',
    detail: 'x64 or ARM64',
    terminal: 'PowerShell',
    prompt: 'PS>',
    command: windowsCommand,
    icon: LaptopIcon,
  },
] as const;

type PlatformId = (typeof installers)[number]['id'];

export function PlatformInstaller() {
  const [platform, setPlatform] = useState<PlatformId>('macos');
  const selected = installers.find((installer) => installer.id === platform) ?? installers[0];

  return (
    <div>
      <div aria-label="Choose your operating system" className="grid gap-3 md:grid-cols-3" role="group">
        {installers.map((installer) => {
          const Icon = installer.icon;
          const isSelected = installer.id === platform;

          return (
            <button
              aria-pressed={isSelected}
              className={cn(
                'group flex min-h-28 items-center gap-4 rounded-xl border bg-card/70 px-5 py-5 text-left shadow-none transition-[border-color,background-color,box-shadow,transform] hover:-translate-y-0.5 hover:border-primary/40 focus-visible:outline-none focus-visible:ring-3 focus-visible:ring-ring/40',
                isSelected
                  ? 'border-primary/55 bg-primary/[0.06] shadow-[0_12px_32px_-24px_color-mix(in_oklch,var(--primary)_70%,transparent)] ring-1 ring-primary/20'
                  : 'border-border/80',
              )}
              key={installer.id}
              onClick={() => setPlatform(installer.id)}
              type="button"
            >
              <span className={cn('grid size-10 shrink-0 place-items-center rounded-lg bg-muted text-muted-foreground transition-colors', isSelected && 'bg-primary text-primary-foreground')}>
                <Icon className="size-5" />
              </span>
              <span className="flex min-w-0 flex-1 flex-col gap-1">
                <span className="font-heading text-lg font-semibold tracking-[-0.03em]">{installer.name}</span>
                <span className="text-sm text-muted-foreground">{installer.detail}</span>
              </span>
              <span aria-hidden="true" className={cn('size-2 rounded-full border border-border bg-background', isSelected && 'border-primary bg-primary')} />
            </button>
          );
        })}
      </div>

      <div aria-live="polite" className="mt-4 overflow-hidden rounded-xl border border-border/80 bg-card/70 shadow-sm">
        <div className="flex flex-col gap-2 border-b border-border/70 px-5 py-4 sm:flex-row sm:items-center sm:justify-between">
          <div>
            <p className="font-heading text-lg font-semibold tracking-[-0.035em]">Install on {selected.name}</p>
            <p className="mt-1 text-sm text-muted-foreground">Open {selected.terminal}, then paste and run this command.</p>
          </div>
          <span className="w-fit rounded-full border border-border bg-background px-2.5 py-1 font-mono text-[0.65rem] uppercase tracking-[0.12em] text-muted-foreground">
            Official release installer
          </span>
        </div>
        <div className="space-y-5 p-5">
          <CopyableCommand
            ariaLabel={`Copy ${selected.name} install command`}
            command={selected.command}
            key={selected.id}
            prompt={selected.prompt}
          />
          <div className="grid gap-3 border-t border-border/70 pt-5 sm:grid-cols-2">
            <InstallStep number="1" title="Run the installer" text="Compass verifies the release checksum and installs the matching binary." />
            <InstallStep number="2" title="Confirm it works" text="Open a new terminal and run compass --version." />
          </div>
        </div>
      </div>
    </div>
  );
}

export function InstallCommand({
  command = shellCommand,
  prompt = '$',
  ariaLabel = 'Copy install command',
  className,
}: {
  command?: string;
  prompt?: string;
  ariaLabel?: string;
  className?: string;
}) {
  return <CopyableCommand ariaLabel={ariaLabel} className={className} command={command} prompt={prompt} />;
}

function CopyableCommand({
  command,
  prompt,
  ariaLabel,
  className,
}: {
  command: string;
  prompt: string;
  ariaLabel: string;
  className?: string;
}) {
  const [copied, setCopied] = useState(false);

  async function copyCommand() {
    try {
      await navigator.clipboard.writeText(command);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1800);
    } catch {
      setCopied(false);
    }
  }

  return (
    <div className={cn('flex w-full max-w-full items-center gap-2 rounded-xl border border-border/80 bg-background px-3 py-2 shadow-sm', className)}>
      <code className="min-w-0 flex-1 overflow-x-auto py-1 font-mono text-xs whitespace-nowrap text-foreground sm:text-sm">
        <span className="mr-2 select-none text-compass-amber">{prompt}</span>
        {command}
      </code>
      <Button aria-label={copied ? 'Command copied' : ariaLabel} className="shrink-0" size="icon-sm" variant="ghost" onClick={copyCommand}>
        {copied ? <CheckIcon className="text-primary" /> : <CopyIcon />}
      </Button>
    </div>
  );
}

function InstallStep({ number, title, text }: { number: string; title: string; text: string }) {
  return (
    <div className="flex gap-3">
      <span className="grid size-7 shrink-0 place-items-center rounded-full bg-secondary font-mono text-xs font-semibold text-primary">{number}</span>
      <div>
        <p className="flex items-center gap-2 font-heading font-semibold tracking-[-0.02em]">
          {title}
          {number === '2' && <CheckCircle2Icon className="size-4 text-primary" />}
        </p>
        <p className="mt-1 text-sm leading-6 text-muted-foreground">{text}</p>
      </div>
    </div>
  );
}
