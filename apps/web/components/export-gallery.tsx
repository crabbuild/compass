'use client';

import Image from 'next/image';
import Link from 'next/link';
import { useState } from 'react';
import {
  ArrowUpRightIcon,
  FileDiffIcon,
  NetworkIcon,
  SearchIcon,
  SparklesIcon,
} from 'lucide-react';

import { cn } from '@/lib/utils';

const frames = [
  {
    id: 'overview',
    label: 'Overview',
    eyebrow: 'Map the system',
    title: 'See the whole repository before you touch a file.',
    description:
      'The export starts with a bounded graph and a searchable inspector. You can orient around the shape of a real package without losing the source context underneath it.',
    image: '/screenshots/compass-viewer-overview.png',
    alt: 'Compass export HTML overview showing the Compass viewer package graph and inspector.',
    detail: '568 nodes · 548 edges · local snapshot',
    source: 'packages/compass-viewer',
    icon: NetworkIcon,
  },
  {
    id: 'inspect',
    label: 'Inspect',
    eyebrow: 'Keep evidence attached',
    title: 'Jump from a symbol to its source anchor.',
    description:
      'Selecting a node pins its kind, language, line range, and file in the inspector. The graph stays useful because every visual answer has somewhere concrete to go next.',
    image: '/screenshots/compass-viewer-inspector.png',
    alt: 'Compass export HTML with a selected TypeScript node and its source inspector.',
    detail: 'src/graph/VisNetworkCanvas.tsx · line 199',
    source: 'packages/compass-viewer',
    icon: SearchIcon,
  },
  {
    id: 'compare',
    label: 'Compare',
    eyebrow: 'Follow history',
    title: 'See what changed across exact revisions.',
    description:
      'History views put changed source, graph structure, and review context beside each other. You can check the blast radius without treating a diff as the whole story.',
    image: '/screenshots/compass-history-changed-graph.png',
    alt: 'Compass codebase evolution changed graph view with an inspector panel.',
    detail: 'changed graph · revision-aware review',
    source: 'Compass history workspace',
    icon: FileDiffIcon,
  },
  {
    id: 'evidence',
    label: 'Explain',
    eyebrow: 'Read the evidence',
    title: 'Understand the reason, not just the result.',
    description:
      'Semantic findings keep category, severity, and supporting details in the same review surface. That makes a graph easier to trust when the change is consequential.',
    image: '/screenshots/compass-history-semantic-findings.png',
    alt: 'Compass codebase evolution semantic findings view with evidence details.',
    detail: 'semantic findings · evidence-backed review',
    source: 'Compass history workspace',
    icon: SparklesIcon,
  },
] as const;

type FrameId = (typeof frames)[number]['id'];

export function ExportGallery() {
  const [activeId, setActiveId] = useState<FrameId>('overview');
  const active = frames.find((frame) => frame.id === activeId) ?? frames[0];
  const ActiveIcon = active.icon;

  return (
    <section className="border-y border-border/70 bg-muted/20" aria-labelledby="export-gallery-title">
      <div className="mx-auto max-w-7xl px-5 py-24 lg:px-8 lg:py-32">
        <div className="grid gap-5 lg:grid-cols-[minmax(0,0.84fr)_minmax(0,1.16fr)] lg:items-end">
          <div>
            <p className="eyebrow">From a real repository</p>
            <h2 id="export-gallery-title" className="mt-4 max-w-xl font-heading text-3xl font-semibold tracking-[-0.055em] sm:text-4xl">
              See the graph, then follow the evidence.
            </h2>
          </div>
          <p className="max-w-2xl text-base leading-7 text-muted-foreground lg:justify-self-end">
            The graph captures are built from the <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-[0.85em] text-foreground">packages/compass-viewer</code> source; the history panels show the same evidence model across revision-aware review. Move from orientation to a source line, then into history, without leaving the same local graph story.
          </p>
        </div>

        <div className="mt-10 overflow-hidden rounded-2xl border border-border/80 bg-card shadow-[0_24px_80px_-52px_color-mix(in_oklch,var(--primary)_55%,transparent)]">
          <div className="flex flex-col gap-4 border-b border-border/70 px-4 py-4 sm:px-6 lg:flex-row lg:items-center lg:justify-between">
            <div role="tablist" aria-label="Compass export views" className="flex min-w-0 gap-1 overflow-x-auto pb-1 lg:pb-0">
              {frames.map((frame) => {
                const Icon = frame.icon;
                const isActive = frame.id === activeId;
                return (
                  <button
                    key={frame.id}
                    id={`export-tab-${frame.id}`}
                    type="button"
                    role="tab"
                    aria-selected={isActive}
                    aria-controls="export-panel"
                    data-active={isActive}
                    className={cn(
                      'inline-flex shrink-0 items-center gap-2 rounded-lg px-3 py-2 text-sm font-medium text-muted-foreground transition-colors',
                      'hover:bg-muted/80 hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring',
                      isActive && 'bg-primary/[0.09] text-primary',
                    )}
                    onClick={() => setActiveId(frame.id)}
                  >
                    <Icon aria-hidden="true" className="size-4" strokeWidth={1.8} />
                    {frame.label}
                  </button>
                );
              })}
            </div>
            <span className="hidden shrink-0 font-mono text-[0.65rem] uppercase tracking-[0.16em] text-muted-foreground sm:block">
              compass export html
            </span>
          </div>

          <div
            id="export-panel"
            role="tabpanel"
            aria-labelledby={`export-tab-${active.id}`}
            tabIndex={0}
            className="grid gap-0 lg:grid-cols-[minmax(0,1.55fr)_minmax(18rem,0.75fr)]"
          >
            <figure className="min-w-0 border-b border-border/70 bg-[#eef0ff] p-3 dark:bg-[#121318] sm:p-5 lg:border-b-0 lg:border-r">
              <div className="relative aspect-[16/10] overflow-hidden rounded-xl border border-black/[0.08] bg-background shadow-inner dark:border-white/[0.08]">
                <Image
                  key={active.image}
                  src={active.image}
                  alt={active.alt}
                  fill
                  sizes="(min-width: 1024px) 62vw, 100vw"
                  className="object-contain object-center"
                  priority={active.id === 'overview'}
                />
              </div>
              <figcaption className="flex flex-wrap items-center justify-between gap-2 px-1 pt-3 font-mono text-[0.65rem] uppercase tracking-[0.12em] text-muted-foreground">
                <span>{active.source}</span>
                <span>{active.detail}</span>
              </figcaption>
            </figure>

            <div className="flex flex-col justify-between gap-8 p-6 sm:p-8 lg:p-9">
              <div>
                <div className="flex size-11 items-center justify-center rounded-xl border border-primary/20 bg-primary/[0.08] text-primary">
                  <ActiveIcon aria-hidden="true" className="size-5" strokeWidth={1.8} />
                </div>
                <p className="eyebrow mt-7">{active.eyebrow}</p>
                <h3 className="mt-3 font-heading text-2xl font-semibold leading-tight tracking-[-0.045em] sm:text-3xl">{active.title}</h3>
                <p className="mt-4 text-[0.95rem] leading-7 text-muted-foreground">{active.description}</p>
              </div>
              <div className="flex flex-col gap-4 border-t border-border/70 pt-5">
                <p className="font-mono text-xs leading-6 text-muted-foreground">
                  <span className="mr-2 text-primary">01</span>
                  local artifact · inspectable output · source-aware UX
                </p>
                <Link className="inline-flex w-fit items-center gap-2 text-sm font-medium text-primary transition-[gap] hover:gap-3" href="/product#code-graph">
                  Explore the code graph
                  <ArrowUpRightIcon data-icon="inline-end" />
                </Link>
              </div>
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}
