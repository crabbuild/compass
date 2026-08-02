import {
  CallGraphResponseSchema,
  type CallGraphResponse
} from "@compass/viewer/contracts/callGraph";
import type { RepositorySession } from "../workspace/repositorySession";
import { callGraphCommandArguments } from "./callGraphArguments";

export async function runCallGraph(
  session: RepositorySession,
  request: readonly string[],
  signal?: AbortSignal
): Promise<CallGraphResponse> {
  return session.processes.runJson(
    session.root,
    callGraphCommandArguments(request, session.graphPath),
    CallGraphResponseSchema,
    signal
  );
}
