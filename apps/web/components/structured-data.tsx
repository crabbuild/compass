type JsonLdProps = {
  data: unknown;
};

/**
 * Render JSON-LD in the initial HTML so crawlers can understand the site
 * without waiting for client-side JavaScript.
 */
export function JsonLd({ data }: JsonLdProps) {
  const serialized = (JSON.stringify(data) ?? '').replace(/</g, '\\u003c');

  return (
    <script
      type="application/ld+json"
      dangerouslySetInnerHTML={{ __html: serialized }}
    />
  );
}
