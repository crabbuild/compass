//! Bounded, pure-Rust document decoding with provenance-preserving artifacts.

pub mod document;
pub mod limits;
mod ooxml;
mod processing;
mod raster;

use std::collections::BTreeMap;
use std::fs;
use std::io::Cursor;
use std::path::Path;

use document::{
    DocumentArtifact, DocumentBlock, DocumentBlockKind, DocumentError, DocumentFormat,
    DocumentLocator,
};
use oxidize_pdf::parser::{PdfDocument, PdfReader};

pub use document::{
    DOCUMENT_INSPECT_SCHEMA, DOCUMENT_NORMALIZER_VERSION, DOCUMENT_SCHEMA, DiagnosticSeverity,
    DocumentDiagnostic, DocumentLink, DocumentLinkKind, DocumentOrigin, VisualCoverage,
};
pub use limits::{
    MEDIA_MAX_RAW_BYTES, OCR_MAX_AGGREGATE_PIXELS, OCR_MAX_OOXML_IMAGES, OCR_MAX_PDF_PAGES,
    OFFICE_MAX_COMPRESSION_RATIO, OFFICE_MAX_DECOMPRESSED_BYTES,
};
pub use ooxml::{RasterCandidate, decode_docx, decode_pptx, decode_xlsx, raster_candidates};
pub use processing::{
    DocumentProcessingOptions, decode_document_with_ocr, decode_document_with_ocr_cancellable,
};
pub use raster::{
    PDF_RASTERIZER_IDENTITY, PdfRasterCandidate, rasterize_pdf_pages,
    rasterize_pdf_pages_cancellable,
};

pub type MediaError = DocumentError;

/// Decode one bounded source into Compass's stable document artifact.
pub fn decode_document(
    logical_path: &Path,
    bytes: &[u8],
) -> Result<DocumentArtifact, DocumentError> {
    if bytes.len() as u64 > MEDIA_MAX_RAW_BYTES {
        return Err(DocumentError::Rejected(format!(
            "source is {} bytes; maximum is {MEDIA_MAX_RAW_BYTES}",
            bytes.len()
        )));
    }
    let artifact = match extension(logical_path).as_str() {
        "pdf" => decode_pdf(bytes)?,
        "docx" => decode_docx(bytes)?,
        "xlsx" => decode_xlsx(bytes)?,
        "pptx" => decode_pptx(bytes)?,
        "txt" => decode_plain_text(bytes, DocumentFormat::Text)?,
        "md" | "markdown" => decode_plain_text(bytes, DocumentFormat::Markdown)?,
        "html" | "htm" => decode_plain_text(bytes, DocumentFormat::Html)?,
        "rtf" => {
            return Err(DocumentError::Unsupported(
                "rtf (enum vocabulary exists, decoder is not implemented)".to_owned(),
            ));
        }
        _ => decode_plain_text(bytes, DocumentFormat::Text)?,
    };
    artifact.validate()?;
    Ok(artifact)
}

/// Extract compatibility Markdown/text through the versioned artifact path.
pub fn extract_text(path: &Path) -> Result<String, MediaError> {
    let bytes = read_bounded(path)?;
    let artifact = decode_document(path, &bytes)?;
    render_document_markdown(&artifact)
}

/// Legacy best-effort surface. New callers must use [`extract_text`] or [`decode_document`].
#[must_use]
pub fn extract_text_compat(path: &Path) -> String {
    extract_text(path).unwrap_or_default()
}

pub fn extract_pdf_text(path: &Path) -> Result<String, MediaError> {
    let bytes = read_bounded(path)?;
    render_document_markdown(&decode_pdf(&bytes)?)
}

pub fn docx_to_markdown(path: &Path) -> Result<String, MediaError> {
    let bytes = read_bounded(path)?;
    render_document_markdown(&decode_docx(&bytes)?)
}

pub fn xlsx_to_markdown(path: &Path) -> Result<String, MediaError> {
    let bytes = read_bounded(path)?;
    render_document_markdown(&decode_xlsx(&bytes)?)
}

pub fn pptx_to_markdown(path: &Path) -> Result<String, MediaError> {
    let bytes = read_bounded(path)?;
    render_document_markdown(&decode_pptx(&bytes)?)
}

pub fn validate_office_archive(path: &Path) -> Result<(), MediaError> {
    let bytes = read_bounded(path)?;
    ooxml::Package::open(&bytes).map(|_| ())
}

pub fn render_document_markdown(artifact: &DocumentArtifact) -> Result<String, DocumentError> {
    artifact.validate()?;
    match artifact.format {
        DocumentFormat::Xlsx => render_spreadsheet(artifact),
        _ => render_ordered_blocks(artifact),
    }
}

fn decode_pdf(bytes: &[u8]) -> Result<DocumentArtifact, DocumentError> {
    let owned = bytes.to_vec();
    std::panic::catch_unwind(move || {
        let reader = PdfReader::new(Cursor::new(owned)).map_err(|error| error.to_string())?;
        let document = PdfDocument::new(reader);
        let pages = document.extract_text().map_err(|error| error.to_string())?;
        let mut artifact = DocumentArtifact::new(DocumentFormat::Pdf);
        for (index, page) in pages.into_iter().enumerate() {
            let page_number = one_based_u32(index, "PDF page")?;
            let page_block = artifact
                .push_block(
                    None,
                    DocumentBlockKind::Page,
                    String::new(),
                    DocumentLocator::Pdf {
                        page: page_number,
                        item: 1,
                    },
                )
                .map_err(|error| error.to_string())?;
            if !page.text.is_empty() {
                artifact
                    .push_block(
                        Some(page_block),
                        DocumentBlockKind::Paragraph,
                        page.text,
                        DocumentLocator::Pdf {
                            page: page_number,
                            item: 2,
                        },
                    )
                    .map_err(|error| error.to_string())?;
            }
        }
        Ok::<_, String>(artifact)
    })
    .map_err(|_| DocumentError::Parse("PDF parser panicked".to_owned()))?
    .map_err(DocumentError::Parse)
}

fn decode_plain_text(
    bytes: &[u8],
    format: DocumentFormat,
) -> Result<DocumentArtifact, DocumentError> {
    let text = String::from_utf8_lossy(bytes).into_owned();
    let end_byte = u64::try_from(bytes.len())
        .map_err(|_| DocumentError::Rejected("text byte length overflow".to_owned()))?;
    let end_line = u32::try_from(text.lines().count().max(1))
        .map_err(|_| DocumentError::Rejected("text line count overflow".to_owned()))?;
    let mut artifact = DocumentArtifact::new(format);
    artifact.push_block(
        None,
        DocumentBlockKind::Paragraph,
        text,
        DocumentLocator::TextRange {
            start_byte: 0,
            end_byte,
            start_line: 1,
            end_line,
        },
    )?;
    Ok(artifact)
}

fn read_bounded(path: &Path) -> Result<Vec<u8>, DocumentError> {
    let metadata = fs::metadata(path).map_err(|source| DocumentError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.len() > MEDIA_MAX_RAW_BYTES {
        return Err(DocumentError::Rejected(format!(
            "{} is {} bytes; maximum is {MEDIA_MAX_RAW_BYTES}",
            path.display(),
            metadata.len()
        )));
    }
    let bytes = fs::read(path).map_err(|source| DocumentError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if bytes.len() as u64 > MEDIA_MAX_RAW_BYTES {
        return Err(DocumentError::Rejected(
            "source grew beyond the media limit while being read".to_owned(),
        ));
    }
    Ok(bytes)
}

fn render_ordered_blocks(artifact: &DocumentArtifact) -> Result<String, DocumentError> {
    let children = child_index(&artifact.blocks);
    let mut lines = Vec::new();
    let mut skipped = std::collections::BTreeSet::new();
    for block in &artifact.blocks {
        if skipped.contains(&block.ordinal) {
            continue;
        }
        match &block.kind {
            DocumentBlockKind::Table => {
                let rendered = render_table(artifact, block.ordinal, &children, &mut skipped);
                if !rendered.is_empty() {
                    lines.extend(rendered);
                }
            }
            DocumentBlockKind::Heading { level } => {
                lines.push(format!(
                    "{} {}",
                    "#".repeat(usize::from(*level)),
                    block.text
                ));
            }
            DocumentBlockKind::DocumentTitle => lines.push(format!("# {}", block.text)),
            DocumentBlockKind::ListItem => lines.push(format!("- {}", block.text)),
            DocumentBlockKind::Code => lines.push(format!("```\n{}\n```", block.text)),
            DocumentBlockKind::Quote => lines.push(format!("> {}", block.text)),
            DocumentBlockKind::Paragraph | DocumentBlockKind::Note => {
                lines.push(block.text.clone());
            }
            DocumentBlockKind::Slide if artifact.format == DocumentFormat::Pptx => {
                if let DocumentLocator::Slide { slide, .. } = block.locator {
                    lines.push(format!("## Slide {slide}"));
                }
            }
            DocumentBlockKind::Page
            | DocumentBlockKind::Sheet
            | DocumentBlockKind::Slide
            | DocumentBlockKind::List
            | DocumentBlockKind::Row
            | DocumentBlockKind::Cell
            | DocumentBlockKind::Other { .. } => {}
        }
    }
    Ok(lines.join("\n"))
}

fn render_spreadsheet(artifact: &DocumentArtifact) -> Result<String, DocumentError> {
    let children = child_index(&artifact.blocks);
    let mut sections = Vec::new();
    for sheet in artifact
        .blocks
        .iter()
        .filter(|block| matches!(block.kind, DocumentBlockKind::Sheet))
    {
        sections.push(format!("## Sheet: {}", sheet.text));
        let rows = children.get(&sheet.ordinal).cloned().unwrap_or_default();
        let mut rendered_rows = Vec::new();
        let mut max_column = 0_usize;
        for row_ordinal in rows {
            let Some(row) = artifact.blocks.get(row_ordinal as usize) else {
                continue;
            };
            if !matches!(row.kind, DocumentBlockKind::Row) {
                continue;
            }
            let cells = children.get(&row.ordinal).cloned().unwrap_or_default();
            let mut sparse = BTreeMap::new();
            for cell_ordinal in cells {
                let Some(cell) = artifact.blocks.get(cell_ordinal as usize) else {
                    continue;
                };
                if let DocumentLocator::Spreadsheet { column, .. } = cell.locator {
                    let column = usize::from(column);
                    max_column = max_column.max(column);
                    sparse.insert(column, cell.text.clone());
                }
            }
            rendered_rows.push(sparse);
        }
        if rendered_rows.is_empty() {
            continue;
        }
        for (index, row) in rendered_rows.iter().enumerate() {
            let cells = (1..=max_column)
                .map(|column| row.get(&column).cloned().unwrap_or_default())
                .collect::<Vec<_>>();
            sections.push(markdown_row(&cells));
            if index == 0 {
                sections.push(markdown_row(&vec!["---".to_owned(); max_column]));
            }
        }
    }
    Ok(sections.join("\n"))
}

fn child_index(blocks: &[DocumentBlock]) -> BTreeMap<u32, Vec<u32>> {
    let mut children = BTreeMap::<u32, Vec<u32>>::new();
    for block in blocks {
        if let Some(parent) = block.parent {
            children.entry(parent).or_default().push(block.ordinal);
        }
    }
    children
}

fn render_table(
    artifact: &DocumentArtifact,
    table: u32,
    children: &BTreeMap<u32, Vec<u32>>,
    skipped: &mut std::collections::BTreeSet<u32>,
) -> Vec<String> {
    let mut rows = Vec::new();
    for row_ordinal in children.get(&table).into_iter().flatten() {
        skipped.insert(*row_ordinal);
        let mut cells = Vec::new();
        for cell_ordinal in children.get(row_ordinal).into_iter().flatten() {
            skipped.insert(*cell_ordinal);
            if let Some(cell) = artifact.blocks.get(*cell_ordinal as usize) {
                cells.push(escape_markdown_cell(&cell.text));
            }
        }
        if !cells.is_empty() {
            rows.push(cells);
        }
    }
    let Some(header) = rows.first() else {
        return Vec::new();
    };
    let mut rendered = vec![markdown_row(header)];
    rendered.push(markdown_row(&vec!["---".to_owned(); header.len()]));
    rendered.extend(rows.iter().skip(1).map(|row| markdown_row(row)));
    rendered
}

fn escape_markdown_cell(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace(['\r', '\n'], "<br>")
}

fn markdown_row(cells: &[String]) -> String {
    format!("| {} |", cells.join(" | "))
}

fn one_based_u32(index: usize, field: &str) -> Result<u32, String> {
    u32::try_from(index)
        .ok()
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| format!("{field} index overflow"))
}

fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::fs::File;
    use std::io::Write;

    use tempfile::tempdir;
    use zip::CompressionMethod;
    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    use super::*;

    type TestResult = Result<(), Box<dyn Error>>;

    fn write_zip(path: &Path, members: &[(&str, &str)]) -> TestResult {
        let file = File::create(path)?;
        let mut writer = ZipWriter::new(file);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        for (name, contents) in members {
            writer.start_file(*name, options)?;
            writer.write_all(contents.as_bytes())?;
        }
        writer.finish()?;
        Ok(())
    }

    #[test]
    fn docx_preserves_paragraph_table_paragraph_order() -> TestResult {
        let directory = tempdir()?;
        let path = directory.path().join("sample.docx");
        write_zip(
            &path,
            &[
                (
                    "word/styles.xml",
                    r#"<w:styles xmlns:w="urn:w"><w:style w:styleId="Heading1"><w:name w:val="Heading 1"/></w:style></w:styles>"#,
                ),
                (
                    "word/document.xml",
                    r#"<w:document xmlns:w="urn:w"><w:body><w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Title</w:t></w:r></w:p><w:tbl><w:tr><w:tc><w:p><w:r><w:t>Name</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>Value</w:t></w:r></w:p></w:tc></w:tr><w:tr><w:tc><w:p><w:r><w:t>A</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>1</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p><w:r><w:t>After</w:t></w:r></w:p></w:body></w:document>"#,
                ),
            ],
        )?;
        assert_eq!(
            docx_to_markdown(&path)?,
            "# Title\n| Name | Value |\n| --- | --- |\n| A | 1 |\nAfter"
        );
        Ok(())
    }

    #[test]
    fn xlsx_is_sparse_typed_and_bounded() -> TestResult {
        let directory = tempdir()?;
        let path = directory.path().join("sample.xlsx");
        write_zip(
            &path,
            &[
                (
                    "xl/workbook.xml",
                    r#"<workbook xmlns:r="urn:r"><sheets><sheet name="Main" r:id="rId1"/></sheets></workbook>"#,
                ),
                (
                    "xl/_rels/workbook.xml.rels",
                    r#"<Relationships><Relationship Id="rId1" Target="worksheets/sheet1.xml"/></Relationships>"#,
                ),
                (
                    "xl/sharedStrings.xml",
                    r#"<sst><si><t>Name</t></si><si><r><t>Val</t></r><r><t>ue</t></r></si></sst>"#,
                ),
                (
                    "xl/worksheets/sheet1.xml",
                    r#"<worksheet><sheetData><row r="1"><c r="A1" t="s"><v>0</v></c><c r="C1" t="s"><v>1</v></c></row><row r="2"><c r="A2" t="inlineStr"><is><t>Alice</t></is></c><c r="B2" t="b"><v>1</v></c><c r="C2"><f>20+22</f><v>42</v></c></row></sheetData></worksheet>"#,
                ),
            ],
        )?;
        let artifact = decode_document(&path, &fs::read(&path)?)?;
        assert_eq!(
            artifact
                .blocks
                .iter()
                .filter(|b| matches!(b.kind, DocumentBlockKind::Cell))
                .count(),
            5
        );
        assert_eq!(
            render_document_markdown(&artifact)?,
            "## Sheet: Main\n| Name |  | Value |\n| --- | --- | --- |\n| Alice | True | 42 |"
        );
        assert!(
            artifact
                .blocks
                .iter()
                .any(|block| block.metadata.contains_key("formula"))
        );
        Ok(())
    }

    #[test]
    fn rejects_non_zip_and_high_ratio_archives() -> TestResult {
        let directory = tempdir()?;
        let fake = directory.path().join("fake.xlsx");
        fs::write(&fake, b"not a zip")?;
        assert!(validate_office_archive(&fake).is_err());

        let bomb = directory.path().join("bomb.docx");
        let payload = "0".repeat(5 * 1024 * 1024);
        write_zip(&bomb, &[("word/document.xml", &payload)])?;
        assert!(matches!(
            validate_office_archive(&bomb),
            Err(MediaError::Rejected(_))
        ));
        Ok(())
    }

    #[test]
    fn rejects_raw_files_over_cap() -> TestResult {
        let directory = tempdir()?;
        let path = directory.path().join("oversize.pdf");
        let file = File::create(&path)?;
        file.set_len(MEDIA_MAX_RAW_BYTES + 1)?;
        assert!(matches!(
            extract_pdf_text(&path),
            Err(MediaError::Rejected(_))
        ));
        Ok(())
    }

    #[test]
    fn plain_text_uses_utf8_lossy_compatibility() -> TestResult {
        let directory = tempdir()?;
        let path = directory.path().join("notes.txt");
        fs::write(&path, b"hello\xffworld")?;
        assert_eq!(extract_text(&path)?, "hello\u{fffd}world");
        Ok(())
    }
}
