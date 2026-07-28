import type { CallDirection } from "@compass/viewer/contracts/callGraph";

type CallGraphRoot = {
  file: string;
  byte: number;
  line: number;
};

export function callGraphCommandArguments(
  request: readonly string[],
  graphPath: string
): string[] {
  // Keep the editor interaction on the required structural graph. Program IR
  // remains an explicit CLI enrichment because reparsing it on every reveal
  // can be much more expensive than resolving the requested neighborhood.
  return [
    "call-graph",
    ...request,
    "--max-nodes",
    "500",
    "--max-edges",
    "1000",
    "--graph",
    graphPath,
    "--format",
    "json"
  ];
}

export function callGraphRootArguments(
  root: CallGraphRoot,
  direction: CallDirection,
  depth: number
): string[] {
  return [
    "--file",
    root.file,
    "--byte",
    String(root.byte),
    "--line",
    String(root.line),
    "--direction",
    direction,
    "--depth",
    String(depth)
  ];
}

export function callGraphExpansionArguments(
  symbol: string,
  direction: CallDirection,
  depth: number
): string[] {
  return [
    "--symbol",
    symbol,
    "--direction",
    direction,
    "--depth",
    String(depth)
  ];
}
