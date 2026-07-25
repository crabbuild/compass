import { ChevronLeftIcon, ChevronRightIcon } from "lucide-react";

export function Pagination({
  page,
  pageCount,
  start,
  end,
  total,
  label,
  onPageChange
}: {
  page: number;
  pageCount: number;
  start: number;
  end: number;
  total: number;
  label: string;
  onPageChange(page: number): void;
}) {
  return (
    <nav className="workbench-pagination" aria-label={`${label} pagination`}>
      <span className="workbench-pagination-range" role="status" aria-live="polite">
        {start.toLocaleString()}–{end.toLocaleString()} of {total.toLocaleString()} {label}
      </span>
      <div className="workbench-pagination-actions">
        <button
          type="button"
          aria-label={`Previous ${label} page`}
          disabled={page <= 1}
          onClick={() => onPageChange(page - 1)}
        >
          <ChevronLeftIcon aria-hidden="true" />
        </button>
        <span aria-label={`Page ${page} of ${pageCount}`}>
          {page} / {pageCount}
        </span>
        <button
          type="button"
          aria-label={`Next ${label} page`}
          disabled={page >= pageCount}
          onClick={() => onPageChange(page + 1)}
        >
          <ChevronRightIcon aria-hidden="true" />
        </button>
      </div>
    </nav>
  );
}
