'use client';

import { CheckIcon, CopyIcon } from 'lucide-react';
import { useState } from 'react';

import { Button } from '@/components/ui/button';

const command = 'curl -LsSf https://github.com/crabbuild/compass/releases/latest/download/install.sh | sh';

export function InstallCommand() {
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
    <div className="flex w-full max-w-xl items-center gap-2 rounded-xl border border-border/80 bg-card px-3 py-2 shadow-sm">
      <code className="min-w-0 flex-1 truncate font-mono text-xs text-muted-foreground sm:text-sm">
        <span className="mr-2 text-compass-amber">$</span>
        {command}
      </code>
      <Button aria-label={copied ? 'Install command copied' : 'Copy install command'} size="icon-sm" variant="ghost" onClick={copyCommand}>
        {copied ? <CheckIcon className="text-primary" /> : <CopyIcon />}
      </Button>
    </div>
  );
}
