import type { CallDirection } from "@compass/viewer/contracts/callGraph";

type CallGraphRoot = {
  file: string;
  byte: number;
  line: number;
};

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
