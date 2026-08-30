import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { GraphNode, GraphViewModel } from "../contracts/graph";
import { documentContextForNode, DocumentOcrPanel } from "./DocumentOcrPanel";

const profile = {
  engine: "OAR-OCR",
  engine_version: "0.9.2",
  profile: "pp-ocrv6-small",
  model_digests: {},
  languages: [],
  preprocessing_version: 2
};

const root: GraphNode = {
  id: "document",
  label: "scan.pdf",
  kind: "document",
  community: 0,
  source: { file: "docs/scan.pdf" },
  document: {
    role: "root",
    kind: "document",
    format: "pdf",
    ocrMode: "auto",
    visualCoverage: "partial",
    complete: false,
    ocrProfile: profile
  }
};

const native: GraphNode = {
  id: "native",
  label: "Native title",
  kind: "paragraph",
  community: 0,
  source: { file: "docs/scan.pdf" },
  document: {
    role: "block",
    kind: "paragraph",
    text: "Native title",
    origin: { kind: "native" },
    locator: { kind: "pdf", page: 4, item: 2 },
    ordinal: 1
  }
};

const ocr: GraphNode = {
  id: "ocr",
  label: "Invoice total",
  kind: "paragraph",
  community: 0,
  source: { file: "docs/scan.pdf" },
  document: {
    role: "block",
    kind: "paragraph",
    text: "Invoice total",
    origin: { kind: "ocr", profile, confidence_bps: 9_234 },
    locator: {
      kind: "ocr",
      owner: { kind: "pdf", page: 4, item: 1 },
      candidate_id: "page-4",
      width: 1_000,
      height: 800,
      polygon: [
        { x: 80, y: 100 },
        { x: 390, y: 100 },
        { x: 390, y: 160 },
        { x: 80, y: 160 }
      ],
      occurrence: 0
    },
    ordinal: 2
  }
};

const model: GraphViewModel = {
  schema: "compass.viewer.graph/1",
  title: "OCR fixture",
  stats: { nodes: 3, edges: 2, communities: 1, aggregated: false },
  nodes: [root, native, ocr],
  edges: [
    { id: "contains-native", source: root.id, target: native.id, relation: "contains" },
    { id: "contains-ocr", source: root.id, target: ocr.id, relation: "contains" }
  ],
  communities: [{ id: 0, label: "Documents", color: "#4e79a7", hidden: false }],
  hyperedges: []
};

describe("DocumentOcrPanel", () => {
  it("groups document blocks and preserves the selected document context", () => {
    const context = documentContextForNode(model, ocr);
    expect(context?.root?.id).toBe("document");
    expect(context?.blocks.map((block) => block.id)).toEqual(["native", "ocr"]);
  });

  it("shows OCR status, confidence, locators, warning, and a bounding-box preview", () => {
    const context = documentContextForNode(model, ocr);
    if (!context) throw new Error("document context missing");
    const onFocus = vi.fn();
    render(<DocumentOcrPanel context={context} selectedId={ocr.id} onFocus={onFocus} />);

    expect(screen.getByText("OCR Auto")).toBeInTheDocument();
    expect(screen.getByText("Partial coverage")).toBeInTheDocument();
    expect(screen.getByText("92.34%")).toBeInTheDocument();
    expect(screen.getAllByText("Page 4").length).toBeGreaterThan(0);
    expect(screen.getByRole("alert")).toHaveTextContent("OCR coverage is partial");
    expect(screen.getByRole("img", { name: /OCR bounding boxes/i })).toBeInTheDocument();

    fireEvent.click(screen.getByRole("tab", { name: /OCR evidence/i }));
    expect(screen.getAllByText("Invoice total").length).toBeGreaterThan(0);
    const ocrBlock = screen
      .getAllByRole("button", { name: /Invoice total/i })
      .find((button) => button.classList.contains("compass-document-block"));
    if (!ocrBlock) throw new Error("OCR block button missing");
    fireEvent.click(ocrBlock);
    expect(onFocus).toHaveBeenCalledWith("ocr");
  });

  it("does not mislabel native-only parse incompleteness as an OCR failure", () => {
    const nativeOnly = {
      ...model,
      nodes: model.nodes
        .filter((node) => node.id !== "ocr")
        .map((node) => node.id === "document"
          ? {
            ...node,
            document: {
              ...node.document,
              complete: false,
              visualCoverage: "not_requested" as const,
              ocrMode: "off" as const
            }
          }
          : node)
    };
    const context = documentContextForNode(nativeOnly, native);
    if (!context) throw new Error("document context missing");
    render(<DocumentOcrPanel context={context} selectedId={native.id} onFocus={vi.fn()} />);

    expect(screen.getByText("Native only")).toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent("Document extraction is incomplete");
    expect(screen.getByRole("alert")).not.toHaveTextContent("OCR coverage is partial");
  });

  it("renders a safe Office snapshot and maps OCR boxes onto its image region", () => {
    const previewModel: GraphViewModel = {
      ...model,
      nodes: model.nodes.map((node) => node.id === "document"
        ? {
          ...node,
          document: {
            ...node.document,
            format: "docx",
            previews: [{
              schema: "compass.document.preview/1",
              kind: "page",
              locator: { kind: "page", page: 1 },
              label: "Page 1 · Normalized preview",
              width: 100,
              height: 100,
              svg: "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 100 100\"><rect width=\"100\" height=\"100\" fill=\"#fff\"/></svg>",
              regions: [{ candidate_id: "page-4", x: 10, y: 10, width: 80, height: 70 }],
              digest: "sha256:ab968df6659867783ee1bf8296158be7b199ea12776cc0b1f1df5c213ac83421"
            }]
          }
        }
        : node)
    };
    const context = documentContextForNode(previewModel, ocr);
    if (!context) throw new Error("document context missing");
    const onFocus = vi.fn();
    render(<DocumentOcrPanel context={context} selectedId={ocr.id} onFocus={onFocus} />);

    expect(screen.getByRole("img", { name: /document preview/i })).toBeInTheDocument();
    expect(screen.getByRole("img", { name: /OCR bounding boxes over Page 1/i })).toBeInTheDocument();
    const box = screen.getAllByRole("button", { name: /Invoice total/ })
      .find((button) => button.classList.contains("compass-ocr-preview-box"));
    if (!box) throw new Error("OCR overlay box missing");
    fireEvent.click(box);
    expect(onFocus).toHaveBeenCalledWith("ocr");
  });
});
