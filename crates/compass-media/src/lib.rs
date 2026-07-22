//! Bounded, pure-Rust text extraction for semantic media inputs.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use oxidize_pdf::parser::{PdfDocument, PdfReader};
use roxmltree::{Document, Node};
use zip::ZipArchive;

pub const MEDIA_MAX_RAW_BYTES: u64 = 50 * 1024 * 1024;
pub const OFFICE_MAX_DECOMPRESSED_BYTES: u64 = 512 * 1024 * 1024;
pub const OFFICE_MAX_COMPRESSION_RATIO: u64 = 200;
const OFFICE_MEMBER_MAX_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("could not access {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("media rejected: {0}")]
    Rejected(String),
    #[error("media parse failed: {0}")]
    Parse(String),
}

/// Extract text from the formats accepted by Graphify's semantic path.
pub fn extract_text(path: &Path) -> Result<String, MediaError> {
    enforce_raw_size(path)?;
    match extension(path).as_str() {
        "pdf" => extract_pdf_text(path),
        "docx" => docx_to_markdown(path),
        "xlsx" => xlsx_to_markdown(path),
        _ => fs::read(path)
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .map_err(|source| MediaError::Io {
                path: path.to_path_buf(),
                source,
            }),
    }
}

/// Compatibility surface for callers where malformed media is a skipped,
/// empty source rather than a fatal corpus error.
#[must_use]
pub fn extract_text_compat(path: &Path) -> String {
    extract_text(path).unwrap_or_default()
}

pub fn extract_pdf_text(path: &Path) -> Result<String, MediaError> {
    enforce_raw_size(path)?;
    let owned = path.to_path_buf();
    std::panic::catch_unwind(move || {
        let reader = PdfReader::open(&owned).map_err(|error| error.to_string())?;
        let document = PdfDocument::new(reader);
        let pages = document.extract_text().map_err(|error| error.to_string())?;
        Ok::<_, String>(
            pages
                .into_iter()
                .map(|page| page.text)
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join("\n"),
        )
    })
    .map_err(|_| MediaError::Parse("PDF parser panicked".to_owned()))?
    .map_err(MediaError::Parse)
}

pub fn docx_to_markdown(path: &Path) -> Result<String, MediaError> {
    validate_office_archive(path)?;
    let styles = read_zip_member(path, "word/styles.xml")
        .ok()
        .and_then(|xml| parse_docx_styles(&xml).ok())
        .unwrap_or_default();
    let document_xml = read_zip_member(path, "word/document.xml")?;
    let document = Document::parse(&document_xml)
        .map_err(|error| MediaError::Parse(format!("invalid DOCX document XML: {error}")))?;
    let Some(body) = document
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "body")
    else {
        return Ok(String::new());
    };
    let mut paragraphs = Vec::new();
    let mut tables = Vec::new();
    for child in body.children().filter(Node::is_element) {
        match child.tag_name().name() {
            "p" => paragraphs.push(render_docx_paragraph(child, &styles)),
            "tbl" => tables.push(render_docx_table(child, &styles)),
            _ => {}
        }
    }
    let mut lines = paragraphs;
    for table in tables {
        if table.is_empty() {
            continue;
        }
        lines.push(markdown_row(&table[0]));
        lines.push(markdown_row(
            &table[0]
                .iter()
                .map(|_| "---".to_owned())
                .collect::<Vec<_>>(),
        ));
        lines.extend(table.iter().skip(1).map(|row| markdown_row(row)));
    }
    Ok(lines.join("\n"))
}

pub fn xlsx_to_markdown(path: &Path) -> Result<String, MediaError> {
    validate_office_archive(path)?;
    let workbook_xml = read_zip_member(path, "xl/workbook.xml")?;
    let relationships_xml = read_zip_member(path, "xl/_rels/workbook.xml.rels")?;
    let shared_strings = read_zip_member(path, "xl/sharedStrings.xml")
        .ok()
        .and_then(|xml| parse_shared_strings(&xml).ok())
        .unwrap_or_default();
    let relationships = parse_workbook_relationships(&relationships_xml)?;
    let workbook = Document::parse(&workbook_xml)
        .map_err(|error| MediaError::Parse(format!("invalid XLSX workbook XML: {error}")))?;
    let mut sections = Vec::new();
    for sheet in workbook
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "sheet")
    {
        let name = attribute_local(sheet, "name").unwrap_or_default();
        let relation = attribute_local(sheet, "id").unwrap_or_default();
        let Some(target) = relationships.get(relation) else {
            continue;
        };
        let member = normalize_xlsx_target(target);
        let Ok(sheet_xml) = read_zip_member(path, &member) else {
            continue;
        };
        let rows = parse_xlsx_rows(&sheet_xml, &shared_strings)?;
        if rows.is_empty() {
            continue;
        }
        sections.push(format!("## Sheet: {name}"));
        sections.push(markdown_row(&rows[0]));
        sections.push(markdown_row(
            &rows[0].iter().map(|_| "---".to_owned()).collect::<Vec<_>>(),
        ));
        sections.extend(rows.iter().skip(1).map(|row| markdown_row(row)));
    }
    Ok(sections.join("\n"))
}

pub fn validate_office_archive(path: &Path) -> Result<(), MediaError> {
    enforce_raw_size(path)?;
    let file = File::open(path).map_err(|source| MediaError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut archive = ZipArchive::new(file)
        .map_err(|error| MediaError::Parse(format!("invalid ZIP container: {error}")))?;
    let mut compressed = 0_u64;
    let mut declared = 0_u64;
    for index in 0..archive.len() {
        let member = archive
            .by_index_raw(index)
            .map_err(|error| MediaError::Parse(error.to_string()))?;
        compressed = compressed.saturating_add(member.compressed_size());
        declared = declared.saturating_add(member.size());
    }
    if declared > OFFICE_MAX_DECOMPRESSED_BYTES {
        return Err(MediaError::Rejected(format!(
            "declared office payload is {declared} bytes"
        )));
    }
    if declared
        > compressed
            .max(1)
            .saturating_mul(OFFICE_MAX_COMPRESSION_RATIO)
    {
        return Err(MediaError::Rejected(
            "office compression ratio exceeds safety limit".to_owned(),
        ));
    }
    let mut actual = 0_u64;
    let mut buffer = [0_u8; 1024 * 1024];
    for index in 0..archive.len() {
        let mut member = archive
            .by_index(index)
            .map_err(|error| MediaError::Parse(error.to_string()))?;
        loop {
            let read = member
                .read(&mut buffer)
                .map_err(|error| MediaError::Parse(error.to_string()))?;
            if read == 0 {
                break;
            }
            actual = actual.saturating_add(read as u64);
            if actual > OFFICE_MAX_DECOMPRESSED_BYTES {
                return Err(MediaError::Rejected(
                    "decompressed office payload exceeds safety limit".to_owned(),
                ));
            }
        }
    }
    Ok(())
}

fn enforce_raw_size(path: &Path) -> Result<(), MediaError> {
    let metadata = fs::metadata(path).map_err(|source| MediaError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.len() > MEDIA_MAX_RAW_BYTES {
        return Err(MediaError::Rejected(format!(
            "{} is {} bytes; maximum is {MEDIA_MAX_RAW_BYTES}",
            path.display(),
            metadata.len()
        )));
    }
    Ok(())
}

fn read_zip_member(path: &Path, name: &str) -> Result<String, MediaError> {
    let file = File::open(path).map_err(|source| MediaError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut archive = ZipArchive::new(file)
        .map_err(|error| MediaError::Parse(format!("invalid ZIP container: {error}")))?;
    let member = archive
        .by_name(name)
        .map_err(|error| MediaError::Parse(format!("missing {name}: {error}")))?;
    if member.size() > OFFICE_MEMBER_MAX_BYTES {
        return Err(MediaError::Rejected(format!(
            "office member {name} exceeds safety limit"
        )));
    }
    let mut bytes = Vec::with_capacity(usize::try_from(member.size()).unwrap_or(0));
    member
        .take(OFFICE_MEMBER_MAX_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| MediaError::Parse(error.to_string()))?;
    if bytes.len() as u64 > OFFICE_MEMBER_MAX_BYTES {
        return Err(MediaError::Rejected(format!(
            "office member {name} exceeds safety limit"
        )));
    }
    String::from_utf8(bytes)
        .map_err(|error| MediaError::Parse(format!("office member {name} is not UTF-8: {error}")))
}

fn parse_docx_styles(xml: &str) -> Result<HashMap<String, String>, MediaError> {
    let document = Document::parse(xml)
        .map_err(|error| MediaError::Parse(format!("invalid DOCX styles XML: {error}")))?;
    let mut styles = HashMap::new();
    for style in document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "style")
    {
        let Some(id) = attribute_local(style, "styleId") else {
            continue;
        };
        let name = style
            .children()
            .find(|node| node.is_element() && node.tag_name().name() == "name")
            .and_then(|node| attribute_local(node, "val"))
            .unwrap_or(id);
        styles.insert(id.to_owned(), name.to_owned());
    }
    Ok(styles)
}

fn render_docx_paragraph(node: Node<'_, '_>, styles: &HashMap<String, String>) -> String {
    let text = node
        .descendants()
        .filter(|descendant| descendant.is_element() && descendant.tag_name().name() == "t")
        .filter_map(|descendant| descendant.text())
        .collect::<String>()
        .trim()
        .to_owned();
    if text.is_empty() {
        return String::new();
    }
    let style_id = node
        .descendants()
        .find(|descendant| descendant.is_element() && descendant.tag_name().name() == "pStyle")
        .and_then(|style| attribute_local(style, "val"))
        .unwrap_or_default();
    let style_name = styles.get(style_id).map_or(style_id, String::as_str);
    let normalized_style = style_name.to_ascii_lowercase();
    if normalized_style.starts_with("heading 1") || normalized_style == "heading1" {
        format!("# {text}")
    } else if normalized_style.starts_with("heading 2") || normalized_style == "heading2" {
        format!("## {text}")
    } else if normalized_style.starts_with("heading 3") || normalized_style == "heading3" {
        format!("### {text}")
    } else if normalized_style.starts_with("list") {
        format!("- {text}")
    } else {
        text
    }
}

fn render_docx_table(table: Node<'_, '_>, styles: &HashMap<String, String>) -> Vec<Vec<String>> {
    table
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "tr")
        .map(|row| {
            row.children()
                .filter(|node| node.is_element() && node.tag_name().name() == "tc")
                .map(|cell| {
                    cell.children()
                        .filter(|node| node.is_element() && node.tag_name().name() == "p")
                        .map(|paragraph| render_docx_paragraph(paragraph, styles))
                        .collect::<Vec<_>>()
                        .join("\n")
                        .trim()
                        .to_owned()
                })
                .collect()
        })
        .collect()
}

fn parse_shared_strings(xml: &str) -> Result<Vec<String>, MediaError> {
    let document = Document::parse(xml)
        .map_err(|error| MediaError::Parse(format!("invalid shared strings XML: {error}")))?;
    Ok(document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "si")
        .map(|item| {
            item.descendants()
                .filter(|node| node.is_element() && node.tag_name().name() == "t")
                .filter_map(|node| node.text())
                .collect::<String>()
        })
        .collect())
}

fn parse_workbook_relationships(xml: &str) -> Result<HashMap<String, String>, MediaError> {
    let document = Document::parse(xml)
        .map_err(|error| MediaError::Parse(format!("invalid workbook relationships: {error}")))?;
    Ok(document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "Relationship")
        .filter_map(|node| {
            Some((
                attribute_local(node, "Id")?.to_owned(),
                attribute_local(node, "Target")?.to_owned(),
            ))
        })
        .collect())
}

fn parse_xlsx_rows(xml: &str, shared_strings: &[String]) -> Result<Vec<Vec<String>>, MediaError> {
    let document = Document::parse(xml)
        .map_err(|error| MediaError::Parse(format!("invalid worksheet XML: {error}")))?;
    let mut rows = Vec::new();
    for row in document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "row")
    {
        let mut values = Vec::<String>::new();
        for cell in row
            .children()
            .filter(|node| node.is_element() && node.tag_name().name() == "c")
        {
            let column = attribute_local(cell, "r")
                .map(excel_column_index)
                .unwrap_or(values.len());
            if values.len() <= column {
                values.resize(column + 1, String::new());
            }
            let cell_type = attribute_local(cell, "t").unwrap_or_default();
            let raw = cell
                .descendants()
                .find(|node| node.is_element() && node.tag_name().name() == "v")
                .and_then(|node| node.text())
                .unwrap_or_default();
            values[column] = match cell_type {
                "s" => raw
                    .parse::<usize>()
                    .ok()
                    .and_then(|index| shared_strings.get(index))
                    .cloned()
                    .unwrap_or_default(),
                "inlineStr" => cell
                    .descendants()
                    .filter(|node| node.is_element() && node.tag_name().name() == "t")
                    .filter_map(|node| node.text())
                    .collect::<String>(),
                "b" => match raw {
                    "1" => "True".to_owned(),
                    "0" => "False".to_owned(),
                    _ => raw.to_owned(),
                },
                _ => raw.to_owned(),
            };
        }
        if values.iter().any(|value| !value.is_empty()) {
            rows.push(values);
        }
    }
    Ok(rows)
}

fn normalize_xlsx_target(target: &str) -> String {
    if let Some(absolute) = target.strip_prefix('/') {
        absolute.to_owned()
    } else if target.starts_with("xl/") {
        target.to_owned()
    } else {
        format!("xl/{target}")
    }
}

fn excel_column_index(reference: &str) -> usize {
    reference
        .bytes()
        .take_while(u8::is_ascii_alphabetic)
        .fold(0_usize, |value, byte| {
            value
                .saturating_mul(26)
                .saturating_add(usize::from(byte.to_ascii_uppercase() - b'A' + 1))
        })
        .saturating_sub(1)
}

fn attribute_local<'a>(node: Node<'a, 'a>, name: &str) -> Option<&'a str> {
    node.attributes()
        .find(|attribute| attribute.name() == name)
        .map(|attribute| attribute.value())
}

fn markdown_row(cells: &[String]) -> String {
    format!("| {} |", cells.join(" | "))
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
    fn converts_docx_paragraphs_styles_and_tables_like_python() -> TestResult {
        let directory = tempdir()?;
        let path = directory.path().join("sample.docx");
        write_zip(
            &path,
            &[
                (
                    "word/styles.xml",
                    r#"<w:styles xmlns:w="urn:w"><w:style w:styleId="Heading1"><w:name w:val="Heading 1"/></w:style><w:style w:styleId="ListBullet"><w:name w:val="List Bullet"/></w:style></w:styles>"#,
                ),
                (
                    "word/document.xml",
                    r#"<w:document xmlns:w="urn:w"><w:body><w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Title</w:t></w:r></w:p><w:p/><w:p><w:pPr><w:pStyle w:val="ListBullet"/></w:pPr><w:r><w:t>Item</w:t></w:r></w:p><w:tbl><w:tr><w:tc><w:p><w:r><w:t>Name</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>Value</w:t></w:r></w:p></w:tc></w:tr><w:tr><w:tc><w:p><w:r><w:t>A</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>1</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:body></w:document>"#,
                ),
            ],
        )?;

        assert_eq!(
            docx_to_markdown(&path)?,
            "# Title\n\n- Item\n| Name | Value |\n| --- | --- |\n| A | 1 |"
        );
        Ok(())
    }

    #[test]
    fn converts_xlsx_shared_inline_boolean_and_sparse_cells() -> TestResult {
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
                    r#"<worksheet><sheetData><row><c r="A1" t="s"><v>0</v></c><c r="C1" t="s"><v>1</v></c></row><row><c r="A2" t="inlineStr"><is><t>Alice</t></is></c><c r="B2" t="b"><v>1</v></c><c r="C2"><v>42</v></c></row></sheetData></worksheet>"#,
                ),
            ],
        )?;

        assert_eq!(
            xlsx_to_markdown(&path)?,
            "## Sheet: Main\n| Name |  | Value |\n| --- | --- | --- |\n| Alice | True | 42 |"
        );
        Ok(())
    }

    #[test]
    fn rejects_non_zip_office_documents() -> TestResult {
        let directory = tempdir()?;
        let path = directory.path().join("fake.xlsx");
        fs::write(&path, b"not a zip")?;

        assert!(validate_office_archive(&path).is_err());
        assert_eq!(extract_text_compat(&path), "");
        Ok(())
    }

    #[test]
    fn rejects_high_ratio_office_archives() -> TestResult {
        let directory = tempdir()?;
        let path = directory.path().join("bomb.docx");
        let payload = "0".repeat(5 * 1024 * 1024);
        write_zip(&path, &[("word/document.xml", &payload)])?;

        assert!(matches!(
            validate_office_archive(&path),
            Err(MediaError::Rejected(_))
        ));
        assert_eq!(docx_to_markdown(&path).unwrap_or_default(), "");
        Ok(())
    }

    #[test]
    fn rejects_raw_files_over_the_cap_without_reading_them() -> TestResult {
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
