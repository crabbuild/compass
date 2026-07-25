export function currentGraphExportArgs(
  graphPath: string,
  nodeLimit: number,
  communityId?: number
): string[] {
  return [
    "export",
    "json",
    "--graph",
    graphPath,
    "--node-limit",
    String(nodeLimit),
    ...(communityId === undefined ? [] : ["--community", String(communityId)])
  ];
}

export function historicalGraphExportArgs(
  commit: string,
  output: string,
  nodeLimit: number,
  communityId?: number
): string[] {
  return [
    "history",
    "export",
    commit,
    "--format",
    "json",
    "--node-limit",
    String(nodeLimit),
    "--output",
    output,
    ...(communityId === undefined ? [] : ["--community", String(communityId)])
  ];
}
