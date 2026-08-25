export function Card({ title }: { title: string }) {
  return <article data-title={title}>{title}</article>;
}
