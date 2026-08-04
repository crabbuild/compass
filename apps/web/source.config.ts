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

// The repository's user documentation remains the canonical source for the
// public website. Contributor-focused design and implementation documents stay
// in the repository, but are intentionally excluded from this collection.
export const docs = defineDocs({
  dir: '../../docs',
  docs: {
    files: [
      'README.md',
      'getting-started.md',
      'COMPASSQL.md',
      'COMPASSQL_SUPPORT.md',
      'concepts/*.md',
      'cookbook/*.md',
      'guides/*.md',
      '!guides/compass-store-operations.md',
      'reference/*.md',
      '!reference/universal-semantic-evidence.md',
    ],
    schema: (ctx) =>
      pageSchema.extend({
        // Existing repository Markdown intentionally has no frontmatter. Use
        // its first H1 as the web title without modifying the canonical files.
        title: z.string().default(() => {
          const heading = ctx.source.match(/^#\s+(.+)$/m)?.[1]?.trim();
          return heading ?? ctx.path.split('/').pop()?.replace(/\.(md|mdx)$/, '') ?? 'Compass';
        }),
        description: z.string().default(() => {
          const paragraph = ctx.source
            .replace(/^---[\s\S]*?---\s*/m, '')
            .split(/\n\s*\n/)
            .map((block) => block.trim())
            .find((block) => block && !block.startsWith('#') && !block.startsWith('```'));
          const plain = paragraph?.replace(/[>*_`]/g, '').replace(/\s+/g, ' ').trim();
          if (!plain) return 'Learn how Compass makes codebases easier to understand.';
          if (plain.length <= 160) return plain;
          return `${plain.slice(0, 157).replace(/\s+\S*$/, '')}…`;
        }),
      }),
  },
});

export default defineConfig();
