export type Page<T> = {
  items: T[];
  page: number;
  pageCount: number;
  pageSize: number;
  total: number;
  start: number;
  end: number;
};

export function clampPage(page: number, pageCount: number): number {
  const normalizedPage = Math.trunc(page) || 1;
  return Math.min(Math.max(1, normalizedPage), Math.max(1, pageCount));
}

export function paginate<T>(
  items: readonly T[],
  page: number,
  pageSize: number
): Page<T> {
  const normalizedSize = Math.max(1, Math.trunc(pageSize) || 1);
  const pageCount = Math.max(1, Math.ceil(items.length / normalizedSize));
  const normalizedPage = clampPage(page, pageCount);
  const offset = (normalizedPage - 1) * normalizedSize;
  const visible = items.slice(offset, offset + normalizedSize);
  return {
    items: visible,
    page: normalizedPage,
    pageCount,
    pageSize: normalizedSize,
    total: items.length,
    start: visible.length === 0 ? 0 : offset + 1,
    end: offset + visible.length
  };
}
