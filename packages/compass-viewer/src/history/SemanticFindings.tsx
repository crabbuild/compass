import { SparklesIcon } from "lucide-react";
import { Badge } from "../components/ui/badge";

export function SemanticFindings({ report }: { report: unknown }) {
  const findings = report && typeof report === "object" && "findings" in report
    && Array.isArray((report as { findings?: unknown }).findings)
    ? (report as { findings: unknown[] }).findings
    : [];
  return (
    <section className="mt-4 rounded-md border bg-card p-4 text-card-foreground">
      <div className="flex items-center gap-2">
        <SparklesIcon />
        <h2 className="text-sm font-semibold">Semantic change findings</h2>
        <Badge variant="secondary">{findings.length}</Badge>
      </div>
      {findings.length > 0 ? (
        <div className="mt-3 flex flex-col gap-2">
          {findings.map((finding, index) => (
            <pre key={index} className="overflow-auto whitespace-pre-wrap rounded-md bg-muted p-3 text-xs">
              {JSON.stringify(finding, null, 2)}
            </pre>
          ))}
        </div>
      ) : (
        <pre className="mt-3 max-h-64 overflow-auto whitespace-pre-wrap rounded-md bg-muted p-3 text-xs">
          {JSON.stringify(report, null, 2)}
        </pre>
      )}
    </section>
  );
}
