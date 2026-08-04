import defaultMdxComponents from 'fumadocs-ui/mdx';
import type { MDXComponents } from 'mdx/types';

import {
  EvidenceAnatomyDiagram,
  EvidenceStates,
  GuideSteps,
  LaunchMapDiagram,
} from '@/components/blog-visuals';

export function getMDXComponents(components?: MDXComponents) {
  return {
    ...defaultMdxComponents,
    EvidenceAnatomyDiagram,
    EvidenceStates,
    GuideSteps,
    LaunchMapDiagram,
    ...components,
  } satisfies MDXComponents;
}

export const useMDXComponents = getMDXComponents;

declare global {
  type MDXProvidedComponents = ReturnType<typeof getMDXComponents>;
}
