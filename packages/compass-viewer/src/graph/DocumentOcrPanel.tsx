import {
  AlertTriangle,
  CheckCircle2,
  ChevronLeft,
  ChevronRight,
  Eye,
  FileImage,
  MapPin,
  ScanText,
  ShieldAlert
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import type {
  DocumentLocator,
  DocumentOrigin,
  DocumentPreview,
  GraphDocument,
  GraphNode,
  GraphViewModel
} from "../contracts/graph";

type EvidenceTab = "combined" | "native" | "ocr";

export type DocumentContext = {
  sourceFile?: string;
  root?: GraphNode;
  blocks: GraphNode[];
};

type OcrLocator = DocumentLocator & {
  kind: "ocr";
  owner?: DocumentLocator;
  candidate_id?: string;
  width?: number;
  height?: number;
  polygon?: Array<{ x: number; y: number }>;
  occurrence?: number;
};

type OcrBlock = GraphNode & {
  document: GraphDocument & {
    origin: Extract<DocumentOrigin, { kind: "ocr" }>;
    locator: OcrLocator;
  };
};

const TABS: Array<{ id: EvidenceTab; label: string }> = [
  { id: "combined", label: "Combined" },
  { id: "native", label: "Native" },
  { id: "ocr", label: "OCR evidence" }
];

export function documentContextForNode(
  model: GraphViewModel,
  selected: GraphNode
): DocumentContext | undefined {
  if (!selected.document) return undefined;
  const sourceFile = selected.source?.file;
  const candidates = model.nodes.filter((node) => {
    if (!node.document) return false;
    return sourceFile === undefined || node.source?.file === sourceFile;
  });
  const root = candidates.find((node) => node.document?.role === "root")
    ?? (selected.document.role === "root" ? selected : undefined);
  const blocks = candidates.filter((node) => node.document?.role === "block");
  if (selected.document.role === "block" && !blocks.some((node) => node.id === selected.id)) {
    blocks.push(selected);
  }
  if (!root && blocks.length === 0) return undefined;
  blocks.sort(documentBlockOrder);
  return {
    ...(sourceFile !== undefined ? { sourceFile } : {}),
    ...(root ? { root } : {}),
    blocks
  };
}

export function DocumentOcrPanel({
  context,
  selectedId,
  onFocus
}: {
  context: DocumentContext;
  selectedId?: string;
  onFocus(nodeId: string): void;
}) {
  const [tab, setTab] = useState<EvidenceTab>("combined");
  const ocrBlocks = useMemo(
    () => context.blocks.filter(isOcrBlock),
    [context.blocks]
  );
  const nativeBlocks = useMemo(
    () => context.blocks.filter((block) => !isOcrBlock(block)),
    [context.blocks]
  );
  const [previewId, setPreviewId] = useState<string | undefined>(
    selectedId && ocrBlocks.some((block) => block.id === selectedId)
      ? selectedId
      : ocrBlocks[0]?.id
  );
  useEffect(() => {
    setTab("combined");
    setPreviewId(
      selectedId && ocrBlocks.some((block) => block.id === selectedId)
        ? selectedId
        : ocrBlocks[0]?.id
    );
  }, [context.sourceFile, ocrBlocks, selectedId]);

  const visibleBlocks = tab === "ocr"
    ? ocrBlocks
    : tab === "native"
      ? nativeBlocks
      : context.blocks;
  const rootDocument = context.root?.document;
  const coverage = rootDocument?.visualCoverage
    ?? (ocrBlocks.length > 0 ? "complete" : "not_requested");
  const mode = rootDocument?.ocrMode
    ?? (ocrBlocks.length > 0 ? "auto" : "off");
  const profile = profileName(rootDocument?.ocrProfile)
    ?? ocrBlocks.map((block) => profileName(ocrOrigin(block)?.profile)).find(Boolean);
  const warningStatus = coverage === "failed"
    ? "failed"
    : coverage === "partial"
      ? "partial"
      : rootDocument?.complete === false
        ? "incomplete"
        : undefined;
  const previewBlock = ocrBlocks.find((block) => block.id === previewId);

  return (
    <section className="compass-document-evidence" aria-labelledby="compass-document-evidence-title">
      <header className="compass-document-evidence-header">
        <div className="compass-document-evidence-heading">
          <span className="compass-document-evidence-mark" aria-hidden="true">
            <ScanText />
          </span>
          <span>
            <small>Document evidence</small>
            <h3 id="compass-document-evidence-title">
              {formatDocumentName(context.sourceFile, rootDocument?.format)}
            </h3>
          </span>
        </div>
        <OcrStatusChip
          mode={mode}
          coverage={coverage}
        {...(profile ? { profile } : {})}
        />
      </header>

      {warningStatus && (
        <div className="compass-document-warning" data-status={warningStatus} role="alert">
          <ShieldAlert aria-hidden="true" />
          <span>
            <strong>
              {warningStatus === "failed"
                ? "OCR could not complete"
                : warningStatus === "partial"
                  ? "OCR coverage is partial"
                  : "Document extraction is incomplete"}
            </strong>
            <small>
              {warningStatus === "incomplete"
                ? "Compass retained the available evidence; review the document diagnostics before relying on it as complete."
                : "Native text is still available. Some visual regions could not be processed."}
            </small>
          </span>
        </div>
      )}

      <div className="compass-document-tabs" role="tablist" aria-label="Document evidence views">
        {TABS.map((item, index) => {
          const count = item.id === "ocr"
            ? ocrBlocks.length
            : item.id === "native"
              ? nativeBlocks.length
              : context.blocks.length;
          const active = tab === item.id;
          return (
            <button
              key={item.id}
              id={`compass-document-tab-${item.id}`}
              className="compass-document-tab"
              type="button"
              role="tab"
              aria-selected={active}
              aria-controls="compass-document-panel"
              tabIndex={active ? 0 : -1}
              onClick={() => setTab(item.id)}
              onKeyDown={(event) => {
                const nextIndex = event.key === "ArrowRight"
                  ? (index + 1) % TABS.length
                  : event.key === "ArrowLeft"
                    ? (index - 1 + TABS.length) % TABS.length
                    : event.key === "Home"
                      ? 0
                      : event.key === "End"
                        ? TABS.length - 1
                        : undefined;
                if (nextIndex === undefined) return;
                event.preventDefault();
                const next = TABS[nextIndex];
                if (!next) return;
                setTab(next.id);
                document.getElementById(`compass-document-tab-${next.id}`)?.focus();
              }}
            >
              {item.label}
              <span>{count}</span>
            </button>
          );
        })}
      </div>

      <div
        id="compass-document-panel"
        className="compass-document-tab-panel"
        role="tabpanel"
        aria-labelledby={`compass-document-tab-${tab}`}
        tabIndex={0}
      >
        {visibleBlocks.length > 0 ? (
          <div className="compass-document-block-list">
            {visibleBlocks.map((block) => (
              <DocumentBlockCard
                key={block.id}
                block={block}
                active={block.id === selectedId}
                onClick={() => {
                  if (isOcrBlock(block)) setPreviewId(block.id);
                  onFocus(block.id);
                }}
              />
            ))}
          </div>
        ) : (
          <div className="compass-document-empty">
            <FileImage aria-hidden="true" />
            <strong>{tab === "ocr" ? "No OCR evidence" : "No document blocks"}</strong>
            <span>
              {tab === "ocr"
                ? "Run extraction with a managed OCR profile to inspect visual text."
                : "This document did not publish readable native blocks."}
            </span>
          </div>
        )}
      </div>

      {previewBlock && (
        <OcrPreview
          block={previewBlock}
          siblings={ocrBlocks}
          {...(rootDocument?.previews ? { previews: rootDocument.previews } : {})}
          onFocus={(nodeId) => {
            setPreviewId(nodeId);
            onFocus(nodeId);
          }}
        />
      )}
      {!previewBlock && rootDocument?.previews && rootDocument.previews.length > 0 && (
        <DocumentPreviewGallery previews={rootDocument.previews} />
      )}
    </section>
  );
}

function OcrStatusChip({
  mode,
  coverage,
  profile
}: {
  mode: string;
  coverage: string;
  profile?: string;
}) {
  const isHealthy = coverage === "complete";
  const isRequested = mode !== "off";
  const statusLabel = `OCR ${isRequested ? modeLabel(mode) : "Off"} · ${
    isRequested ? coverageLabel(coverage) : "Native only"
  }${profile ? ` · ${profile}` : ""}`;
  return (
    <span
      className="compass-ocr-status-chip"
      data-status={coverage}
      role="status"
      aria-label={statusLabel}
      title={statusLabel}
    >
      {isHealthy ? <CheckCircle2 aria-hidden="true" /> : <AlertTriangle aria-hidden="true" />}
      <span>
        <strong>OCR {isRequested ? modeLabel(mode) : "Off"}</strong>
        <small>{isRequested ? coverageLabel(coverage) : "Native only"}</small>
      </span>
      {profile && <em>{profile}</em>}
    </span>
  );
}

function DocumentBlockCard({
  block,
  active,
  onClick
}: {
  block: GraphNode;
  active: boolean;
  onClick(): void;
}) {
  const ocr = ocrOrigin(block);
  const text = block.document?.text ?? block.label;
  const locator = block.document?.locator;
  return (
    <button
      className="compass-document-block"
      data-origin={ocr ? "ocr" : "native"}
      data-active={active}
      type="button"
      aria-pressed={active}
      onClick={onClick}
    >
      <span className="compass-document-block-topline">
        <span className="compass-document-block-kind">
          {ocr ? <ScanText aria-hidden="true" /> : <FileImage aria-hidden="true" />}
          {ocr ? "OCR evidence" : formatBlockKind(block.document?.kind)}
        </span>
        {ocr && <ConfidenceBadge confidence={ocr.confidence_bps} />}
      </span>
      <span className="compass-document-block-text">{text || "(empty block)"}</span>
      <span className="compass-document-block-locator">
        <MapPin aria-hidden="true" />
        {formatLocator(locator)}
        {ocr && <Eye aria-hidden="true" />}
      </span>
    </button>
  );
}

function ConfidenceBadge({ confidence }: { confidence: number }) {
  const percentage = Math.max(0, Math.min(100, confidence / 100));
  const band = percentage >= 85 ? "high" : percentage >= 60 ? "medium" : "low";
  return (
    <span className="compass-ocr-confidence" data-confidence={band}>
      {percentage.toFixed(2)}%
    </span>
  );
}

function DocumentPreviewGallery({ previews }: { previews: DocumentPreview[] }) {
  const [index, setIndex] = useState(0);
  useEffect(() => {
    setIndex((current) => Math.min(current, Math.max(0, previews.length - 1)));
  }, [previews.length]);
  const preview = previews[index];
  if (!preview) return null;
  return (
    <div className="compass-ocr-preview compass-document-preview-gallery" aria-label="Document preview">
      <header>
        <span>
          <strong>{preview.label}</strong>
          <small>{formatLocator(preview.locator)} · Normalized preview</small>
        </span>
        {previews.length > 1 && (
          <span className="compass-document-preview-controls">
            <button
              type="button"
              aria-label="Previous document preview"
              disabled={index === 0}
              onClick={() => setIndex((current) => Math.max(0, current - 1))}
            >
              <ChevronLeft aria-hidden="true" />
            </button>
            <span>{index + 1} / {previews.length}</span>
            <button
              type="button"
              aria-label="Next document preview"
              disabled={index === previews.length - 1}
              onClick={() => setIndex((current) => Math.min(previews.length - 1, current + 1))}
            >
              <ChevronRight aria-hidden="true" />
            </button>
          </span>
        )}
      </header>
      <div className="compass-document-preview-stage">
        <img
          className="compass-document-preview-image"
          src={previewDataUrl(preview.svg)}
          alt={`${preview.label} document preview`}
        />
      </div>
      <p>Compass generated a bounded local snapshot. Native text remains the authoritative evidence.</p>
    </div>
  );
}

function OcrPreview({
  block,
  siblings,
  previews,
  onFocus
}: {
  block: OcrBlock;
  siblings: OcrBlock[];
  previews?: DocumentPreview[];
  onFocus(nodeId: string): void;
}) {
  const locator = block.document.locator;
  const width = boundedDimension(locator.width);
  const height = boundedDimension(locator.height);
  const candidateId = locator.candidate_id;
  const regions = siblings.filter((candidate) => {
    const candidateLocator = candidate.document.locator;
    return candidateLocator.candidate_id === candidateId
      && candidateLocator.width === locator.width
      && candidateLocator.height === locator.height;
  });
  if (!width || !height) return null;
  const preview = previewForLocator(previews, locator);
  return (
    <div className="compass-ocr-preview" aria-label="OCR region preview">
      <header>
        <span>
          <strong>{preview?.label ?? "Visual region"}</strong>
          <small>{formatLocator(locator.owner)} · {width} × {height}px</small>
        </span>
        <span className="compass-ocr-preview-key">
          <i />OCR boxes{preview ? " · normalized preview" : ""}
        </span>
      </header>
      {preview ? (
        <div className="compass-document-preview-stage">
          <img
            className="compass-document-preview-image"
            src={previewDataUrl(preview.svg)}
            alt={`${preview.label} document preview`}
          />
          <svg
            className="compass-document-preview-overlay"
            viewBox={`0 0 ${preview.width} ${preview.height}`}
            role="img"
            aria-label={`OCR bounding boxes over ${preview.label}`}
            preserveAspectRatio="none"
          >
            {regions.map((candidate) => {
              const points = polygonPointsForPreview(candidate.document.locator, preview);
              if (!points) return null;
              return ocrPolygon(candidate, points, candidate.id === block.id, onFocus);
            })}
          </svg>
        </div>
      ) : (
        <svg
          className="compass-ocr-preview-canvas"
          viewBox={`0 0 ${width} ${height}`}
          role="img"
          aria-label={`OCR bounding boxes for ${formatLocator(locator.owner)}`}
        >
          <defs>
            <pattern id="compass-ocr-preview-grid" width="32" height="32" patternUnits="userSpaceOnUse">
              <path d="M 32 0 L 0 0 0 32" fill="none" stroke="currentColor" strokeOpacity="0.12" strokeWidth="1" />
            </pattern>
          </defs>
          <rect width={width} height={height} fill="var(--compass-ocr-preview-surface)" />
          <rect width={width} height={height} fill="url(#compass-ocr-preview-grid)" />
          {regions.map((candidate) => {
            const candidateLocator = candidate.document.locator;
            const points = polygonPoints(candidateLocator.polygon, width, height);
            if (!points) return null;
            return ocrPolygon(candidate, points, candidate.id === block.id, onFocus);
          })}
        </svg>
      )}
      <p>
        {preview
          ? "OCR boxes are mapped to the embedded image region. Click a box to inspect its text and confidence."
          : "Click a highlighted region to inspect its text and confidence."}
      </p>
    </div>
  );
}

function ocrPolygon(
  candidate: OcrBlock,
  points: string,
  active: boolean,
  onFocus: (nodeId: string) => void
) {
  return (
    <polygon
      key={candidate.id}
      className="compass-ocr-preview-box"
      data-active={active}
      points={points}
      role="button"
      tabIndex={0}
      aria-label={`${candidate.document.text ?? candidate.label} · ${formatLocator(candidate.document.locator)}`}
      onClick={() => onFocus(candidate.id)}
      onKeyDown={(event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          onFocus(candidate.id);
        }
      }}
    >
      <title>{candidate.document.text ?? candidate.label}</title>
    </polygon>
  );
}

function isOcrBlock(block: GraphNode): block is OcrBlock {
  return ocrOrigin(block) !== undefined && block.document?.locator?.kind === "ocr";
}

function ocrOrigin(block: GraphNode): Extract<DocumentOrigin, { kind: "ocr" }> | undefined {
  return block.document?.origin?.kind === "ocr" ? block.document.origin : undefined;
}

function documentBlockOrder(left: GraphNode, right: GraphNode): number {
  const leftOrdinal = left.document?.ordinal ?? Number.MAX_SAFE_INTEGER;
  const rightOrdinal = right.document?.ordinal ?? Number.MAX_SAFE_INTEGER;
  return leftOrdinal - rightOrdinal || left.id.localeCompare(right.id);
}

function profileName(profile: Record<string, unknown> | undefined): string | undefined {
  const value = profile?.profile;
  return typeof value === "string" ? prettyProfile(value) : undefined;
}

function prettyProfile(value: string): string {
  return value
    .replace(/^pp-/, "PP-")
    .replace(/ocrv(\d+)/i, "OCRv$1")
    .replace(/-(small|medium)$/i, (_, size: string) => ` ${size[0]?.toUpperCase()}${size.slice(1)}`);
}

function modeLabel(value: string): string {
  return value.length > 0 ? `${value[0]?.toUpperCase()}${value.slice(1)}` : "Off";
}

function coverageLabel(value: string): string {
  return {
    complete: "Complete",
    partial: "Partial coverage",
    failed: "Failed",
    not_requested: "Not requested"
  }[value] ?? "Unknown";
}

function formatDocumentName(sourceFile: string | undefined, format: string | undefined): string {
  const name = sourceFile?.split(/[\\/]/).pop() ?? "Document";
  return format ? `${name} · ${format.toUpperCase()}` : name;
}

function formatBlockKind(value: string | undefined): string {
  if (!value) return "Native text";
  return value
    .replaceAll("_", " ")
    .replace(/\b\w/g, (character) => character.toUpperCase());
}

function formatLocator(locator: DocumentLocator | undefined): string {
  if (!locator) return "Location unavailable";
  switch (locator.kind) {
    case "ocr": {
      const owner = field(locator, "owner");
      const occurrence = numberValue(field(locator, "occurrence")) ?? 0;
      return `${formatLocator(isLocator(owner) ? owner : undefined)} · Region ${occurrence + 1}`;
    }
    case "pdf":
      return `Page ${numberValue(field(locator, "page")) ?? "?"}`;
    case "page":
      return `Page ${numberValue(field(locator, "page")) ?? "?"}`;
    case "slide":
      return `Slide ${numberValue(field(locator, "slide")) ?? "?"}${field(locator, "shape") !== undefined ? ` · Shape ${String(field(locator, "shape"))}` : ""}`;
    case "spreadsheet": {
      const sheet = field(locator, "sheet");
      return `${typeof sheet === "string" ? sheet : "Sheet"}!${columnName(numberValue(field(locator, "column")) ?? 1)}${numberValue(field(locator, "row")) ?? "?"}`;
    }
    case "package": {
      const part = field(locator, "part");
      const path = field(locator, "path");
      const partText = typeof part === "string" ? part : "Package part";
      if (/\/(?:media|embeddings)\//i.test(partText) || /\.(?:avif|bmp|gif|jpe?g|png|tiff?|webp)$/i.test(partText)) {
        return "Embedded image";
      }
      const shortPart = partText.split(/[\\/]/).pop() || "Package part";
      return typeof path === "string" && path.length > 0
        ? `${shortPart} · ${path}`
        : shortPart;
    }
    case "text_range": {
      const startLine = numberValue(field(locator, "start_line"));
      const endLine = numberValue(field(locator, "end_line"));
      return startLine !== undefined
        ? `Lines ${startLine}${endLine !== undefined && endLine !== startLine ? `–${endLine}` : ""}`
        : "Text range";
    }
    default:
      return "Document location";
  }
}

function previewForLocator(
  previews: DocumentPreview[] | undefined,
  locator: OcrLocator
): DocumentPreview | undefined {
  if (!previews || previews.length === 0) return undefined;
  const candidateId = locator.candidate_id;
  if (candidateId) {
    const candidatePreview = previews.find((preview) =>
      preview.regions.some((region) => region.candidate_id === candidateId)
    );
    if (candidatePreview) return candidatePreview;
  }
  const owner = locator.owner;
  if (isLocator(owner)) {
    if (owner.kind === "slide") {
      const slide = numberValue(field(owner, "slide"));
      const slidePreview = previews.find((preview) =>
        preview.kind === "slide"
        && preview.locator.kind === "slide"
        && numberValue(field(preview.locator, "slide")) === slide
      );
      if (slidePreview) return slidePreview;
    }
    if (owner.kind === "spreadsheet") {
      const sheet = field(owner, "sheet");
      const sheetPreview = previews.find((preview) =>
        preview.kind === "sheet"
        && preview.locator.kind === "spreadsheet"
        && field(preview.locator, "sheet") === sheet
      );
      if (sheetPreview) return sheetPreview;
    }
  }
  return previews[0];
}

function previewDataUrl(svg: string): string {
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
}

function polygonPointsForPreview(
  locator: OcrLocator,
  preview: DocumentPreview
): string | undefined {
  const candidateId = locator.candidate_id;
  const region = candidateId
    ? preview.regions.find((value) => value.candidate_id === candidateId)
    : undefined;
  const sourceWidth = boundedDimension(locator.width);
  const sourceHeight = boundedDimension(locator.height);
  if (!region || !sourceWidth || !sourceHeight) return undefined;
  if (!Array.isArray(locator.polygon)) return undefined;
  const points = locator.polygon.slice(0, 256).flatMap((point) => {
    if (typeof point !== "object" || point === null) return [];
    const x = numberValue(field(point, "x"));
    const y = numberValue(field(point, "y"));
    if (x === undefined || y === undefined) return [];
    const mappedX = region.x + clamp(x / sourceWidth, 0, 1) * region.width;
    const mappedY = region.y + clamp(y / sourceHeight, 0, 1) * region.height;
    return [`${mappedX},${mappedY}`];
  });
  return points.length >= 3 ? points.join(" ") : undefined;
}

function field(value: object, key: string): unknown {
  return (value as Record<string, unknown>)[key];
}

function isLocator(value: unknown): value is DocumentLocator {
  return typeof value === "object"
    && value !== null
    && typeof field(value, "kind") === "string";
}

function numberValue(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

function columnName(value: number): string {
  let current = Math.max(1, Math.floor(value));
  let output = "";
  while (current > 0) {
    const remainder = (current - 1) % 26;
    output = String.fromCharCode(65 + remainder) + output;
    current = Math.floor((current - 1) / 26);
  }
  return output;
}

function boundedDimension(value: number | undefined): number | undefined {
  return value !== undefined && Number.isFinite(value) && value > 0 && value <= 10_000
    ? value
    : undefined;
}

function polygonPoints(value: unknown, width: number, height: number): string | undefined {
  if (!Array.isArray(value)) return undefined;
  const points = value.slice(0, 256).flatMap((point) => {
    if (typeof point !== "object" || point === null) return [];
    const x = numberValue(field(point, "x"));
    const y = numberValue(field(point, "y"));
    return x !== undefined && y !== undefined
      ? [`${clamp(x, 0, width)},${clamp(y, 0, height)}`]
      : [];
  });
  return points.length >= 3 ? points.join(" ") : undefined;
}

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.max(minimum, Math.min(maximum, value));
}
