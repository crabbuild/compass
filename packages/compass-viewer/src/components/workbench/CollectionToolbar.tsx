import { SearchIcon, XIcon } from "lucide-react";

export function CollectionToolbar({
  value,
  label,
  placeholder,
  resultCount,
  onChange
}: {
  value: string;
  label: string;
  placeholder: string;
  resultCount: number;
  onChange(value: string): void;
}) {
  return (
    <div className="workbench-collection-toolbar">
      <label className="workbench-search">
        <SearchIcon aria-hidden="true" />
        <span className="sr-only">{label}</span>
        <input
          type="search"
          value={value}
          placeholder={placeholder}
          aria-label={label}
          onChange={(event) => onChange(event.target.value)}
        />
        {value && (
          <button
            type="button"
            className="workbench-search-clear"
            aria-label={`Clear ${label.toLocaleLowerCase()}`}
            onClick={() => onChange("")}
          >
            <XIcon aria-hidden="true" />
          </button>
        )}
      </label>
      <span className="workbench-result-count" role="status" aria-live="polite">
        {resultCount.toLocaleString()} {resultCount === 1 ? "result" : "results"}
      </span>
    </div>
  );
}
