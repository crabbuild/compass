import { Alert, AlertDescription, AlertTitle } from "../components/ui/alert";
import type { CallGraphResponse } from "../contracts/callGraph";

export function CoverageNotice({ coverage }: { coverage: CallGraphResponse["coverage"] }) {
  if (coverage.unresolved === 0 && coverage.ambiguous === 0) return null;
  return (
    <Alert>
      <AlertTitle>Partial call resolution</AlertTitle>
      <AlertDescription>{coverage.warning}</AlertDescription>
    </Alert>
  );
}
