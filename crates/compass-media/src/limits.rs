//! Central resource ceilings for untrusted document inputs and artifacts.

pub const MEDIA_MAX_RAW_BYTES: u64 = 50 * 1024 * 1024;
pub const OFFICE_MAX_DECOMPRESSED_BYTES: u64 = 512 * 1024 * 1024;
pub const OFFICE_MAX_COMPRESSION_RATIO: u64 = 200;
pub const OFFICE_MEMBER_MAX_BYTES: u64 = 64 * 1024 * 1024;
pub const OFFICE_MAX_MEMBERS: usize = 100_000;

pub const XLSX_MAX_COLUMNS: usize = 16_384;
pub const XLSX_MAX_ROWS: u32 = 100_000;
pub const XLSX_MAX_CELLS: usize = 1_000_000;

pub const DOCUMENT_MAX_BLOCKS: usize = 1_000_000;
pub const DOCUMENT_MAX_LINKS: usize = 1_000_000;
pub const DOCUMENT_MAX_DIAGNOSTICS: usize = 10_000;
pub const DOCUMENT_MAX_METADATA_ENTRIES: usize = 1_024;
pub const DOCUMENT_MAX_TEXT_CHARS: usize = 20_000_000;
pub const DOCUMENT_MAX_FIELD_BYTES: usize = 16 * 1024;
pub const DOCUMENT_MAX_DIAGNOSTIC_MESSAGE_BYTES: usize = 4 * 1024;
pub const DOCUMENT_MAX_DEPTH: usize = 256;

// Preview artifacts are deliberately small, deterministic SVG snapshots. They
// are presentation evidence only; native document blocks remain authoritative.
pub const DOCUMENT_MAX_PREVIEWS: usize = 256;
pub const DOCUMENT_MAX_PREVIEW_REGIONS: usize = 256;
pub const DOCUMENT_MAX_PREVIEW_DIMENSION: u32 = 2_048;
pub const DOCUMENT_MAX_PREVIEW_SVG_BYTES: usize = 512 * 1024;
pub const DOCUMENT_MAX_PREVIEW_TOTAL_BYTES: usize = 8 * 1024 * 1024;
pub const DOCUMENT_PREVIEW_MAX_INLINE_IMAGE_BYTES: usize = 256 * 1024;
pub const DOCUMENT_PREVIEW_MAX_INLINE_IMAGE_LONG_EDGE: u32 = 720;
pub const DOCUMENT_PREVIEW_MAX_TEXT_LINES: usize = 40;
pub const DOCUMENT_PREVIEW_MAX_TEXT_BLOCKS: usize = 256;

pub const OCR_MAX_PDF_PAGES: usize = 200;
pub const OCR_MAX_OOXML_IMAGES: usize = 256;
pub const OCR_MAX_AGGREGATE_PIXELS: u64 = 300_000_000;
