import {
  AlertCircleIcon,
  CircleDashedIcon,
  LoaderCircleIcon,
  SearchXIcon
} from "lucide-react";

const ICONS = {
  empty: SearchXIcon,
  running: LoaderCircleIcon,
  error: AlertCircleIcon,
  unavailable: CircleDashedIcon
};

export function WorkspaceState({
  kind,
  title,
  description,
  action
}: {
  kind: "empty" | "running" | "error" | "unavailable";
  title: string;
  description: string;
  action?: { label: string; onClick(): void };
}) {
  const Icon = ICONS[kind];
  return (
    <section
      className="workbench-state"
      data-kind={kind}
      role={kind === "error" ? "alert" : kind === "running" ? "status" : undefined}
      aria-live={kind === "error" || kind === "running" ? "polite" : undefined}
    >
      <Icon className={kind === "running" ? "workbench-state-spinner" : undefined} aria-hidden="true" />
      <div>
        <h2>{title}</h2>
        <p>{description}</p>
      </div>
      {action && (
        <button type="button" className="workbench-button" onClick={action.onClick}>
          {action.label}
        </button>
      )}
    </section>
  );
}
