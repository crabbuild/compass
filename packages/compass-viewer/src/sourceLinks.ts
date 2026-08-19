import { z } from "zod";
import type { SourceLocation } from "./contracts/graph";

export const SourceNavigationSchema = z.strictObject({
  provider: z.enum(["github", "gitlab", "bitbucket"]),
  repositoryUrl: z.string().url(),
  revision: z.string().regex(/^(?:[0-9a-f]{40}|[0-9a-f]{64})$/i)
});

export type SourceNavigation = z.infer<typeof SourceNavigationSchema>;

export type ExportSourceOpenResult =
  | { kind: "opened"; url: string }
  | { kind: "unavailable" };

export function remoteSourceUrl(
  navigation: SourceNavigation,
  source: SourceLocation,
  revision = navigation.revision
): string | undefined {
  if (!isImmutableRevision(revision)) return undefined;
  const path = encodedRepositoryPath(source.file);
  if (!path) return undefined;
  const base = navigation.repositoryUrl.replace(/\/$/, "");
  const lineAnchor = sourceLineAnchor(navigation.provider, source);
  if (navigation.provider === "gitlab") {
    return `${base}/-/blob/${revision}/${path}${lineAnchor}`;
  }
  if (navigation.provider === "bitbucket") {
    return `${base}/src/${revision}/${path}${lineAnchor}`;
  }
  return `${base}/blob/${revision}/${path}${lineAnchor}`;
}

export function openExportSource(
  navigation: SourceNavigation | undefined,
  source: SourceLocation,
  revision?: string
): ExportSourceOpenResult {
  window.dispatchEvent(new CustomEvent("compass:open-source", {
    detail: source
  }));
  if (!navigation) return { kind: "unavailable" };
  const url = remoteSourceUrl(navigation, source, revision ?? navigation.revision);
  if (!url) return { kind: "unavailable" };
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.target = "_blank";
  anchor.rel = "noopener noreferrer";
  anchor.click();
  return { kind: "opened", url };
}

function isImmutableRevision(value: string): boolean {
  return /^(?:[0-9a-f]{40}|[0-9a-f]{64})$/i.test(value);
}

function encodedRepositoryPath(value: string): string | undefined {
  if (!value || value.startsWith("/") || value.startsWith("\\") || value.includes("\0")) {
    return undefined;
  }
  const segments = value.replaceAll("\\", "/").split("/");
  if (segments.some((segment) => !segment || segment === "." || segment === "..")) {
    return undefined;
  }
  return segments.map(encodeURIComponent).join("/");
}

function sourceLineAnchor(
  provider: SourceNavigation["provider"],
  source: SourceLocation
): string {
  const start = source.startLine;
  if (start === undefined) return "";
  const end = Math.max(start, source.endLine ?? start);
  if (provider === "gitlab") {
    return end === start ? `#L${start}` : `#L${start}-${end}`;
  }
  if (provider === "bitbucket") {
    return end === start ? `#lines-${start}` : `#lines-${start}:${end}`;
  }
  return end === start ? `#L${start}` : `#L${start}-L${end}`;
}
