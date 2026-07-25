export type InitOptions = {
  root: string;
  includes: string[];
  excludes: string[];
  force: boolean;
};

export function buildInitArgs(options: InitOptions): string[] {
  const args = ["init", options.root];
  for (const include of options.includes) args.push("--include", include);
  for (const exclude of options.excludes) args.push("--exclude", exclude);
  if (options.force) args.push("--force");
  return [...args, "--yes", "--events", "jsonl"];
}

export function buildUpdateArgs(options: { root: string; noViz?: boolean }): string[] {
  return ["update", options.root, ...(options.noViz ? ["--no-viz"] : []), "--events", "jsonl"];
}

export function buildWatchArgs(options: {
  root: string;
  debounceSeconds: number;
  poll: boolean;
}): string[] {
  return [
    "watch",
    options.root,
    "--debounce",
    String(options.debounceSeconds),
    ...(options.poll ? ["--poll"] : []),
    "--events",
    "jsonl"
  ];
}
