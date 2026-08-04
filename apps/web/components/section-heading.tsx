export function SectionHeading({
  eyebrow,
  title,
  description,
  align = 'left',
}: {
  eyebrow: string;
  title: string;
  description?: string;
  align?: 'left' | 'center';
}) {
  return (
    <div className={align === 'center' ? 'mx-auto max-w-2xl text-center' : 'max-w-2xl'}>
      <p className="eyebrow">{eyebrow}</p>
      <h2 className="mt-4 font-heading text-3xl font-semibold tracking-[-0.045em] text-foreground sm:text-4xl">
        {title}
      </h2>
      {description && <p className="mt-4 text-base leading-7 text-muted-foreground">{description}</p>}
    </div>
  );
}
