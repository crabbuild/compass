import { useEffect, useMemo, useState } from "react";
import { createRoot } from "react-dom/client";
import {
  ArchitectureFlow,
  type ArchitectureEvidence,
  type ArchitectureHost,
  type ArchitectureLens,
  type ArchitectureOverview,
  type ArchitectureRoutePage,
  type ArchitectureScope,
  type ArchitectureSearchPage,
  type ArchitectureGroupPage
} from "@compass/viewer";
import { HostToArchitectureMessageSchema } from "../transport/architectureMessages";
import { GraphLoadingState, type GraphLoadingCopy } from "./GraphLoadingState";

declare function acquireVsCodeApi(): { postMessage(message: unknown): void };
const vscode = acquireVsCodeApi();
const element = document.getElementById("root");
if (!element) throw new Error("Compass architecture root is missing");
const root = createRoot(element);

const ARCHITECTURE_LOADING_COPY: GraphLoadingCopy = {
  eyebrow: "Compass architecture",
  title: "Deriving architecture flow",
  steps: ["Exporting evidence", "Indexing typed relationships", "Laying out subsystem routes"]
};

type ArchitectureState = {
  overview?: ArchitectureOverview | undefined;
  groupPage?: ArchitectureGroupPage | undefined;
  routePage?: ArchitectureRoutePage | undefined;
  searchPage?: ArchitectureSearchPage | undefined;
  repositoryId: string;
  generation: number;
  loadingMessage?: string | undefined;
  error?: string | undefined;
};

function ArchitectureApp() {
  const [state, setState] = useState<ArchitectureState>({
    repositoryId: "",
    generation: 0,
    loadingMessage: "Reading graph"
  });

  useEffect(() => {
    const onMessage = (event: MessageEvent) => {
      const parsed = HostToArchitectureMessageSchema.safeParse(event.data);
      if (!parsed.success) return;
      const message = parsed.data;
      if (message.type === "architectureLoading") {
        setState((current) => ({ ...current, loadingMessage: message.message, error: undefined }));
      } else if (message.type === "error") {
        setState((current) => ({ ...current, error: message.message, loadingMessage: undefined }));
      } else {
        setState((current) => {
          if (
            current.repositoryId
            && (
              current.repositoryId !== message.repositoryId
              || message.generation < current.generation
            )
          ) return current;
          const identity = {
            repositoryId: message.repositoryId,
            generation: message.generation,
            error: undefined,
            loadingMessage: undefined
          };
          if (message.type === "architectureOverview") {
            return {
              ...current,
              ...identity,
              overview: message.model,
              groupPage: undefined,
              routePage: undefined
            };
          }
          if (message.type === "architectureGroupPage") {
            return { ...current, ...identity, groupPage: message.model };
          }
          if (message.type === "architectureRoutePage") {
            return { ...current, ...identity, routePage: message.model };
          }
          return { ...current, ...identity, searchPage: message.model };
        });
      }
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, []);

  const host = useMemo<ArchitectureHost>(() => {
    const dataRequest = (
      type: "requestGroup" | "requestRoute" | "searchArchitecture",
      payload: Record<string, unknown>
    ) => {
      vscode.postMessage({
        type,
        requestId: crypto.randomUUID(),
        repositoryId: state.repositoryId,
        generation: state.generation,
        scope: state.overview?.scope ?? "production",
        evidence: state.overview?.evidence ?? "all",
        lens: state.overview?.lens ?? "architecture",
        pageSize: 100,
        ...payload
      });
    };
    return {
      setFilters(scope: ArchitectureScope, evidence: ArchitectureEvidence, lens: ArchitectureLens) {
        vscode.postMessage({
          type: "setArchitectureFilters",
          requestId: crypto.randomUUID(),
          repositoryId: state.repositoryId,
          scope,
          evidence,
          lens
        });
      },
      requestGroup(groupId, kind, page, query) {
        dataRequest("requestGroup", { groupId, kind, page, query });
      },
      requestRoute(routeId, page, query) {
        dataRequest("requestRoute", { routeId, page, query });
      },
      search(query, page) {
        dataRequest("searchArchitecture", { query, page });
      },
      openSource(file) {
        vscode.postMessage({
          type: "openSource",
          requestId: crypto.randomUUID(),
          repositoryId: state.repositoryId,
          file
        });
      }
    };
  }, [
    state.generation,
    state.overview?.evidence,
    state.overview?.lens,
    state.overview?.scope,
    state.repositoryId
  ]);

  if (state.error) {
    return (
      <GraphLoadingState
        state={{ kind: "error", message: state.error }}
        variant="architecture"
        loadingCopy={ARCHITECTURE_LOADING_COPY}
        onRetry={() => vscode.postMessage({ type: "retry" })}
        onShowOutput={() => vscode.postMessage({ type: "showOutput" })}
      />
    );
  }
  if (!state.overview) {
    return (
      <GraphLoadingState
        state={{ kind: "loading" }}
        variant="architecture"
        loadingCopy={{
          ...ARCHITECTURE_LOADING_COPY,
          title: state.loadingMessage ?? ARCHITECTURE_LOADING_COPY.title
        }}
        onRetry={() => vscode.postMessage({ type: "retry" })}
        onShowOutput={() => vscode.postMessage({ type: "showOutput" })}
      />
    );
  }
  return (
    <ArchitectureFlow
      overview={state.overview}
      groupPage={state.groupPage}
      routePage={state.routePage}
      searchPage={state.searchPage}
      loadingMessage={state.loadingMessage}
      host={host}
    />
  );
}

root.render(<ArchitectureApp />);
vscode.postMessage({ type: "ready" });
