export function buildHistoryArgs(options: {
  revision: string;
  all: boolean;
  firstParent: boolean;
  rebuild?: boolean;
  profile?: { kind: "configured" | "code-only" } | { kind: "from"; source: string };
}): string[] {
  return [
    "history",
    options.rebuild ? "rebuild" : "build",
    options.revision,
    ...(options.all ? ["--all"] : []),
    ...(options.firstParent ? ["--first-parent"] : []),
    ...(options.profile?.kind === "code-only" ? ["--code-only"] : []),
    ...(options.profile?.kind === "from" ? ["--profile-from", options.profile.source] : []),
    "--format",
    "json",
    "--events",
    "jsonl"
  ];
}

export function buildEnableHistoryArgs(profile: "code-only" | "default"): string[] {
  return [
    "history",
    "enable",
    ...(profile === "code-only" ? ["--code-only"] : [])
  ];
}
