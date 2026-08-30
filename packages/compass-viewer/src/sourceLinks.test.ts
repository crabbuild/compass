import { describe, expect, it, vi } from "vitest";
import {
  openExportSource,
  remoteSourceUrl,
  type SourceNavigation
} from "./sourceLinks";

const commit = "0123456789abcdef0123456789abcdef01234567";

function navigation(provider: SourceNavigation["provider"]): SourceNavigation {
  return {
    provider,
    repositoryUrl: `https://${provider}.example.com/acme/compass`,
    revision: commit
  };
}

describe("remote source links", () => {
  it("builds provider-specific immutable line links", () => {
    const source = { file: "src/a file.rs", startLine: 12, endLine: 18 };
    expect(remoteSourceUrl(navigation("github"), source)).toBe(
      `https://github.example.com/acme/compass/blob/${commit}/src/a%20file.rs#L12-L18`
    );
    expect(remoteSourceUrl(navigation("gitlab"), source)).toBe(
      `https://gitlab.example.com/acme/compass/-/blob/${commit}/src/a%20file.rs#L12-18`
    );
    expect(remoteSourceUrl(navigation("bitbucket"), source)).toBe(
      `https://bitbucket.example.com/acme/compass/src/${commit}/src/a%20file.rs#lines-12:18`
    );
  });

  it("uses a full historical revision override", () => {
    const historical = "abcdef0123456789abcdef0123456789abcdef01";
    expect(remoteSourceUrl(
      navigation("github"),
      { file: "src/lib.rs", startLine: 4 },
      historical
    )).toContain(`/blob/${historical}/src/lib.rs#L4`);
  });

  it("rejects mutable revisions and unsafe repository paths", () => {
    expect(remoteSourceUrl(
      navigation("github"),
      { file: "../secret", startLine: 1 }
    )).toBeUndefined();
    expect(remoteSourceUrl(
      navigation("github"),
      { file: "src/lib.rs", startLine: 1 },
      "main"
    )).toBeUndefined();
  });

  it("reports when a standalone export has no safe source target", () => {
    const opened = vi.fn();
    window.addEventListener("compass:open-source", opened, { once: true });

    expect(openExportSource(undefined, {
      file: "src/lib.rs",
      startLine: 4,
      endLine: 8
    })).toEqual({ kind: "unavailable" });
    expect(opened).toHaveBeenCalledOnce();
  });
});
