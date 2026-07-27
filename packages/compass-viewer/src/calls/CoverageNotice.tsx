import { Alert, AlertDescription, AlertTitle } from "../components/ui/alert";
import type { CallGraphResponse } from "../contracts/callGraph";

export function CoverageNotice({ coverage }: { coverage: CallGraphResponse["coverage"] }) {
  if (!coverage.partial && coverage.unresolved === 0 && coverage.ambiguous === 0) return null;
  return (
    <Alert>
      <AlertTitle>Partial call coverage</AlertTitle>
      <AlertDescription>{coverage.warning}</AlertDescription>
    </Alert>
  );
}
