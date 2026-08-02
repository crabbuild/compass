import { describe, expect, it, vi } from "vitest";

vi.mock("vscode", () => ({
  Position: class Position {
    constructor(readonly line: number, readonly character: number) {}
  },
  Range: class Range {
    constructor(readonly start: unknown, readonly end: unknown) {}
  }
}));

import type * as vscode from "vscode";
import { fullLineSourceRange } from "./sourceRange";

function documentWithPositions(
  positions: Record<number, [number, number]>
): vscode.TextDocument {
  return {
    positionAt(offset: number) {
      const [line, character] = positions[offset] ?? [0, 0];
      return { line, character };
    },
    validatePosition(position: vscode.Position) {
      return position;
    }
  } as vscode.TextDocument;
}

describe("fullLineSourceRange", () => {
  it("highlights complete recorded lines instead of the symbol byte range", () => {
    const range = fullLineSourceRange(
      documentWithPositions({ 24: [2, 9], 80: [5, 3] }),
      {
        file: "src/main.rs",
        startLine: 3,
        endLine: 6,
        startByte: 24,
        endByte: 80
      }
    );

    expect(range).toMatchObject({
      start: { line: 2, character: 0 },
      end: { line: 6, character: 0 }
    });
  });

  it("highlights the complete start line when no end line is recorded", () => {
    const range = fullLineSourceRange(
      documentWithPositions({ 24: [2, 9], 30: [2, 15] }),
      { file: "src/main.rs", startLine: 3, startByte: 24, endByte: 30 }
    );

    expect(range).toMatchObject({
      start: { line: 2, character: 0 },
      end: { line: 3, character: 0 }
    });
  });

  it("expands byte-only locations to full lines", () => {
    const range = fullLineSourceRange(
      documentWithPositions({ 24: [2, 9], 80: [5, 3] }),
      { file: "src/main.rs", startByte: 24, endByte: 80 }
    );

    expect(range).toMatchObject({
      start: { line: 2, character: 0 },
      end: { line: 6, character: 0 }
    });
  });

  it("does not include an extra line when an exclusive byte end is at a line boundary", () => {
    const range = fullLineSourceRange(
      documentWithPositions({ 24: [2, 9], 80: [6, 0] }),
      { file: "src/main.rs", startByte: 24, endByte: 80 }
    );

    expect(range).toMatchObject({
      start: { line: 2, character: 0 },
      end: { line: 6, character: 0 }
    });
  });
});
