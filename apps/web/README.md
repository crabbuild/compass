# Compass web

The Compass website is a Next.js App Router application using Tailwind CSS v4,
shadcn/ui, and Fumadocs.

## Local development

From the repository root:

```bash
npm run dev:web
```

Useful checks:

```bash
npm run typecheck:web
npm run build:web
```

Set `NEXT_PUBLIC_SITE_URL` when deploying to a domain other than the default
`https://compass.crabbuild.dev`. The value is used for metadata, canonical URLs,
robots, and the sitemap.

## Content and routes

- `app/(marketing)` contains the public marketing shell and product pages.
- `app/docs/[...slug]` renders the repository's canonical Markdown from `docs/`
  through Fumadocs. The `/docs` landing page is intentionally a separate,
  curated entry point.
- `content/blog` contains MDX posts with frontmatter.
- `components/hero-graph.tsx` mounts the shared `@compass/viewer` graph
  renderer used by the VS Code extension, with a small representative
  `GraphViewModel` for the homepage.
- `app/globals.css` owns the Compass tokens, typography, graph animation, and
  the Fumadocs theme bridge.

Generated Fumadocs output lives in `apps/web/.source/` and is ignored by Git.
