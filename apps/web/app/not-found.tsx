import Link from 'next/link';
import { ArrowLeftIcon, CompassIcon } from 'lucide-react';

import { buttonVariants } from '@/components/ui/button';
import { cn } from '@/lib/utils';

export default function NotFound() {
  return <main className="grid min-h-[70vh] place-items-center px-5 py-24"><div className="flex max-w-md flex-col items-center text-center"><div className="grid size-14 place-items-center rounded-2xl border border-border bg-muted/60 text-primary"><CompassIcon /></div><p className="eyebrow mt-7">404 / off the map</p><h1 className="mt-4 font-heading text-4xl font-semibold tracking-[-0.06em]">That path does not exist.</h1><p className="mt-4 text-base leading-7 text-muted-foreground">The page may have moved, or the route was never part of this graph.</p><Link className={cn(buttonVariants({ variant: 'outline' }), 'mt-8 gap-2')} href="/"><ArrowLeftIcon data-icon="inline-start" /> Back to Compass</Link></div></main>;
}
