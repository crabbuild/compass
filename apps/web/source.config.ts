import { defineCollections, defineConfig, defineDocs } from 'fumadocs-mdx/config';
import { pageSchema } from 'fumadocs-core/source/schema';
import { z } from 'zod';

export const blog = defineCollections({
  type: 'doc',
  dir: './content/blog',
  schema: pageSchema.extend({
    author: z.string().default('Compass team'),
    date: z.coerce.date(),
    tags: z.array(z.string()).default([]),
    draft: z.boolean().default(false),
    image: z.string().optional(),
  }),
});

// The repository's existing docs remain the canonical source for the public
// documentation. Fumadocs compiles them in place for the web surface.
export const docs = defineDocs({
  dir: '../../docs',
  docs: {
    files: [
      '*.md',
      'concepts/*.md',
      'cookbook/*.md',
      'design/*.md',
      'guides/*.md',
      'implementation/*.md',
      'reference/*.md',
    ],
    schema: (ctx) =>
      pageSchema.extend({
        // Existing repository Markdown intentionally has no frontmatter. Use
        // its first H1 as the web title without modifying the canonical files.
        title: z.string().default(() => {
          const heading = ctx.source.match(/^#\s+(.+)$/m)?.[1]?.trim();
          return heading ?? ctx.path.split('/').pop()?.replace(/\.(md|mdx)$/, '') ?? 'Compass';
        }),
      }),
  },
});

export default defineConfig();
