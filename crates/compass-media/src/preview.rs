//! Deterministic, local-only Office preview snapshots.
//!
//! These previews intentionally do not attempt to be a full Office renderer.
//! They are normalized evidence surfaces that make native text, embedded image
//! candidates, and OCR geometry inspectable without adding LibreOffice,
//! Python, or a network dependency to Compass.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{Arguments, Write as _};
use std::io::Cursor;
use std::path::Path;

use base64::Engine as _;
use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat};
use sha2::{Digest, Sha256};

use crate::document::{
    DOCUMENT_PREVIEW_SCHEMA, DiagnosticSeverity, DocumentArtifact, DocumentBlockKind,
    DocumentDiagnostic, DocumentError, DocumentFormat, DocumentLocator, DocumentPreview,
    DocumentPreviewKind, DocumentPreviewRegion,
};
use crate::limits::{
    DOCUMENT_MAX_PREVIEW_DIMENSION, DOCUMENT_MAX_PREVIEW_SVG_BYTES,
    DOCUMENT_PREVIEW_MAX_INLINE_IMAGE_BYTES, DOCUMENT_PREVIEW_MAX_INLINE_IMAGE_LONG_EDGE,
    DOCUMENT_PREVIEW_MAX_TEXT_BLOCKS, DOCUMENT_PREVIEW_MAX_TEXT_LINES, OCR_MAX_OOXML_IMAGES,
};
use crate::ooxml::{RasterCandidate, raster_candidates};

const DOCX_WIDTH: u32 = 1_200;
const DOCX_HEIGHT: u32 = 1_600;
const SLIDE_WIDTH: u32 = 1_600;
const SLIDE_HEIGHT: u32 = 900;
const SHEET_WIDTH: u32 = 1_600;
const SHEET_HEIGHT: u32 = 1_000;
const DOCX_LINES_PER_PAGE: usize = 24;
const DOCX_MAX_PAGES: usize = 256;
const MAX_THUMBNAILS_PER_PAGE: usize = 2;
const MAX_THUMBNAILS_PER_SLIDE: usize = 3;
const MAX_THUMBNAILS_PER_SHEET: usize = 3;
const TEXT_LINE_CHARS: usize = 88;
const SLIDE_TEXT_LINE_CHARS: usize = 108;
const SHEET_VISIBLE_ROWS: usize = 18;
const SHEET_VISIBLE_COLUMNS: usize = 8;

#[derive(Clone, Debug)]
struct PreviewImage {
    id: String,
    owner: DocumentLocator,
    width: u32,
    height: u32,
    data_uri: String,
}

#[derive(Clone, Debug)]
struct PlacedImage {
    region: DocumentPreviewRegion,
}

/// Attach normalized previews to an Office artifact. Preview failures are
/// non-fatal: native extraction remains usable and receives a diagnostic.
pub(crate) fn attach_office_previews(
    logical_path: &Path,
    bytes: &[u8],
    artifact: &mut DocumentArtifact,
) {
    let extension = logical_path
        .extension()
        .and_then(|value| value.to_str())
        .map_or_else(String::new, |value| value.to_ascii_lowercase());
    let candidates = match raster_candidates(&extension, bytes) {
        Ok(candidates) => candidates,
        Err(error) => {
            add_diagnostic(
                artifact,
                "preview_candidates_unavailable",
                format!("embedded images were not included in the normalized preview: {error}"),
            );
            Vec::new()
        }
    };
    if candidates.len() > OCR_MAX_OOXML_IMAGES {
        add_diagnostic(
            artifact,
            "preview_image_limit",
            format!(
                "{} embedded images were found; only the first {} are eligible for preview",
                candidates.len(),
                OCR_MAX_OOXML_IMAGES
            ),
        );
    }
    let mut images = Vec::new();
    for candidate in candidates.iter().take(OCR_MAX_OOXML_IMAGES) {
        match prepare_thumbnail(candidate) {
            Ok(Some(image)) => images.push(image),
            Ok(None) => add_diagnostic(
                artifact,
                "preview_image_omitted",
                format!(
                    "embedded image {} was too large for the safe preview budget",
                    candidate.id
                ),
            ),
            Err(error) => add_diagnostic(
                artifact,
                "preview_image_omitted",
                format!(
                    "embedded image {} could not be rendered safely: {error}",
                    candidate.id
                ),
            ),
        }
    }

    let result = match artifact.format {
        DocumentFormat::Docx => build_docx_previews(artifact, &images),
        DocumentFormat::Pptx => build_pptx_previews(artifact, &images),
        DocumentFormat::Xlsx => build_xlsx_previews(artifact, &images),
        _ => Ok(Vec::new()),
    };
    match result {
        Ok(previews) => artifact.previews = previews,
        Err(error) => add_diagnostic(
            artifact,
            "preview_generation_failed",
            format!("normalized preview generation was skipped: {error}"),
        ),
    }
}

fn add_diagnostic(artifact: &mut DocumentArtifact, code: &str, message: String) {
    if artifact.diagnostics.len() >= crate::limits::DOCUMENT_MAX_DIAGNOSTICS {
        return;
    }
    let bounded = message.chars().take(4_000).collect::<String>();
    artifact.diagnostics.push(DocumentDiagnostic {
        code: code.to_owned(),
        severity: DiagnosticSeverity::Warning,
        locator: None,
        message: bounded,
    });
}

fn prepare_thumbnail(candidate: &RasterCandidate) -> Result<Option<PreviewImage>, String> {
    let prepared =
        compass_ocr::prepare_raster(&candidate.bytes).map_err(|error| error.to_string())?;
    let long_edge = DOCUMENT_PREVIEW_MAX_INLINE_IMAGE_LONG_EDGE;
    let mut edge = long_edge;
    for _attempt in 0..3 {
        let (width, height) = fit_dimensions(prepared.width, prepared.height, edge)?;
        let resized = image::imageops::resize(&prepared.image, width, height, FilterType::Triangle);
        let mut encoded = Cursor::new(Vec::new());
        DynamicImage::ImageRgb8(resized)
            .write_to(&mut encoded, ImageFormat::Png)
            .map_err(|error| error.to_string())?;
        if encoded.get_ref().len() <= DOCUMENT_PREVIEW_MAX_INLINE_IMAGE_BYTES {
            let data = base64::engine::general_purpose::STANDARD.encode(encoded.get_ref());
            return Ok(Some(PreviewImage {
                id: candidate.id.clone(),
                owner: candidate.owner.clone(),
                width,
                height,
                data_uri: format!("data:image/png;base64,{data}"),
            }));
        }
        edge = (edge.saturating_mul(2) / 3).max(160);
    }
    Ok(None)
}

fn fit_dimensions(width: u32, height: u32, long_edge: u32) -> Result<(u32, u32), String> {
    if width == 0 || height == 0 || long_edge == 0 {
        return Err("image dimensions are empty".to_owned());
    }
    let source_long = width.max(height);
    let scale_numerator = u64::from(long_edge.min(source_long));
    let source_long = u64::from(source_long);
    let scaled_width = (u64::from(width) * scale_numerator / source_long).max(1);
    let scaled_height = (u64::from(height) * scale_numerator / source_long).max(1);
    let width = u32::try_from(scaled_width).map_err(|_| "preview width overflow".to_owned())?;
    let height = u32::try_from(scaled_height).map_err(|_| "preview height overflow".to_owned())?;
    Ok((width, height))
}

fn build_docx_previews(
    artifact: &DocumentArtifact,
    images: &[PreviewImage],
) -> Result<Vec<DocumentPreview>, DocumentError> {
    let mut pages = vec![Vec::<String>::new()];
    let mut blocks = 0_usize;
    for block in artifact
        .blocks
        .iter()
        .filter(|block| !block.text.trim().is_empty())
    {
        if blocks >= DOCUMENT_PREVIEW_MAX_TEXT_BLOCKS {
            break;
        }
        blocks = blocks.saturating_add(1);
        let mut lines = wrap_text(&block.text, TEXT_LINE_CHARS);
        if lines.is_empty() {
            continue;
        }
        let prefix = match block.kind {
            DocumentBlockKind::Heading { .. } | DocumentBlockKind::DocumentTitle => "▸ ",
            DocumentBlockKind::ListItem => "• ",
            _ => "",
        };
        if let Some(first) = lines.first_mut() {
            *first = format!("{prefix}{first}");
        }
        for line in lines {
            let should_split = pages
                .last()
                .is_some_and(|page| page.len() >= DOCX_LINES_PER_PAGE)
                && pages.len() < DOCX_MAX_PAGES;
            if should_split {
                pages.push(Vec::new());
            }
            let page = pages.last_mut().ok_or_else(|| {
                DocumentError::InvalidArtifact("preview page disappeared".to_owned())
            })?;
            if page.len() < DOCUMENT_PREVIEW_MAX_TEXT_LINES {
                page.push(line);
            }
        }
    }
    let page_count = pages.len().max(1);
    let mut previews = Vec::new();
    for (index, lines) in pages.into_iter().enumerate() {
        let page_number = one_based(index, "DOCX preview page")?;
        let mut body = svg_start(
            DOCX_WIDTH,
            DOCX_HEIGHT,
            "#f3f5f8",
            &format!("Page {page_number}"),
        )?;
        append(
            &mut body,
            format_args!(
                "<rect x=\"54\" y=\"54\" width=\"1092\" height=\"1492\" rx=\"12\" fill=\"#ffffff\" stroke=\"#d7dde7\"/><text x=\"88\" y=\"112\" fill=\"#162033\" font-family=\"Arial,sans-serif\" font-size=\"24\" font-weight=\"700\">Page {page_number} · Normalized preview</text>"
            ),
        )?;
        render_lines(&mut body, &lines, 88, 164, 34, "#24324a")?;
        let page_images = images_for_index(images, index, page_count, MAX_THUMBNAILS_PER_PAGE);
        let placed = render_image_row(&mut body, &page_images, 88, 1_140, 1_020, 300, 2)?;
        append(&mut body, format_args!("</svg>"))?;
        previews.push(make_preview(
            DocumentPreviewKind::Page,
            DocumentLocator::Page { page: page_number },
            format!("Page {page_number} · Normalized preview"),
            DOCX_WIDTH,
            DOCX_HEIGHT,
            body,
            placed,
        )?);
    }
    Ok(previews)
}

fn build_pptx_previews(
    artifact: &DocumentArtifact,
    images: &[PreviewImage],
) -> Result<Vec<DocumentPreview>, DocumentError> {
    let mut slides = BTreeMap::<u32, Vec<String>>::new();
    for block in artifact
        .blocks
        .iter()
        .filter(|block| !block.text.trim().is_empty())
    {
        let Some(slide) = slide_number(&block.locator) else {
            continue;
        };
        if slides.entry(slide).or_default().len() < DOCUMENT_PREVIEW_MAX_TEXT_LINES {
            slides
                .entry(slide)
                .or_default()
                .extend(wrap_text(&block.text, SLIDE_TEXT_LINE_CHARS));
        }
    }
    if slides.is_empty() {
        let slide_numbers = artifact
            .blocks
            .iter()
            .filter_map(|block| slide_number(&block.locator))
            .collect::<BTreeSet<_>>();
        for slide in slide_numbers {
            slides.insert(slide, Vec::new());
        }
    }
    let mut previews = Vec::new();
    let slide_count = slides.len().max(1);
    if slides.is_empty() {
        slides.insert(1, Vec::new());
    }
    for (index, (slide_number, lines)) in slides.into_iter().enumerate() {
        let mut body = svg_start(
            SLIDE_WIDTH,
            SLIDE_HEIGHT,
            "#111827",
            &format!("Slide {slide_number}"),
        )?;
        append(
            &mut body,
            format_args!(
                "<rect x=\"36\" y=\"36\" width=\"1528\" height=\"828\" rx=\"16\" fill=\"#f8fafc\"/><text x=\"82\" y=\"104\" fill=\"#162033\" font-family=\"Arial,sans-serif\" font-size=\"26\" font-weight=\"700\">Slide {slide_number} · Normalized preview</text>"
            ),
        )?;
        render_lines(&mut body, &lines, 82, 158, 34, "#24324a")?;
        let slide_images = images_for_slide(
            images,
            slide_number,
            index,
            slide_count,
            MAX_THUMBNAILS_PER_SLIDE,
        );
        let placed = render_image_row(&mut body, &slide_images, 82, 550, 1_436, 230, 3)?;
        append(&mut body, format_args!("</svg>"))?;
        previews.push(make_preview(
            DocumentPreviewKind::Slide,
            DocumentLocator::Slide {
                slide: slide_number,
                shape: 1,
            },
            format!("Slide {slide_number} · Normalized preview"),
            SLIDE_WIDTH,
            SLIDE_HEIGHT,
            body,
            placed,
        )?);
    }
    Ok(previews)
}

fn build_xlsx_previews(
    artifact: &DocumentArtifact,
    images: &[PreviewImage],
) -> Result<Vec<DocumentPreview>, DocumentError> {
    let mut sheets = BTreeMap::<String, BTreeMap<u32, BTreeMap<u16, String>>>::new();
    for block in &artifact.blocks {
        let DocumentLocator::Spreadsheet { sheet, row, column } = &block.locator else {
            continue;
        };
        sheets.entry(sheet.clone()).or_default();
        if matches!(block.kind, DocumentBlockKind::Cell) {
            sheets
                .entry(sheet.clone())
                .or_default()
                .entry(*row)
                .or_default()
                .insert(*column, block.text.clone());
        }
    }
    if sheets.is_empty() {
        sheets.insert("Sheet".to_owned(), BTreeMap::new());
    }
    let sheet_count = sheets.len();
    let mut previews = Vec::new();
    for (index, (name, rows)) in sheets.into_iter().enumerate() {
        let visible_rows = rows
            .keys()
            .take(SHEET_VISIBLE_ROWS)
            .copied()
            .collect::<Vec<_>>();
        let mut columns = BTreeSet::new();
        for row in &visible_rows {
            if let Some(cells) = rows.get(row) {
                columns.extend(cells.keys().copied().take(SHEET_VISIBLE_COLUMNS));
            }
        }
        let column_count = columns.len();
        let visible_columns = columns
            .into_iter()
            .take(SHEET_VISIBLE_COLUMNS)
            .collect::<Vec<_>>();
        let mut body = svg_start(
            SHEET_WIDTH,
            SHEET_HEIGHT,
            "#eef2f7",
            &format!("Sheet {name}"),
        )?;
        append(
            &mut body,
            format_args!(
                "<rect x=\"32\" y=\"32\" width=\"1536\" height=\"936\" rx=\"12\" fill=\"#ffffff\" stroke=\"#d7dde7\"/><text x=\"64\" y=\"84\" fill=\"#162033\" font-family=\"Arial,sans-serif\" font-size=\"24\" font-weight=\"700\">{}</text>",
                escape_xml(&format!("{name} · Sheet snapshot"))
            ),
        )?;
        render_sheet_grid(&mut body, &rows, &visible_rows, &visible_columns)?;
        if rows.len() > visible_rows.len() || column_count > visible_columns.len() {
            append(
                &mut body,
                format_args!(
                    "<text x=\"64\" y=\"760\" fill=\"#667085\" font-family=\"Arial,sans-serif\" font-size=\"16\">Showing a bounded snapshot; additional rows or columns remain available in native evidence.</text>"
                ),
            )?;
        }
        let sheet_images =
            images_for_sheet(images, &name, index, sheet_count, MAX_THUMBNAILS_PER_SHEET);
        let placed = render_image_row(&mut body, &sheet_images, 64, 790, 1_472, 140, 3)?;
        append(&mut body, format_args!("</svg>"))?;
        previews.push(make_preview(
            DocumentPreviewKind::Sheet,
            DocumentLocator::Spreadsheet {
                sheet: name.clone(),
                row: 1,
                column: 1,
            },
            format!("{name} · Sheet snapshot"),
            SHEET_WIDTH,
            SHEET_HEIGHT,
            body,
            placed,
        )?);
    }
    Ok(previews)
}

fn render_sheet_grid(
    svg: &mut String,
    rows: &BTreeMap<u32, BTreeMap<u16, String>>,
    visible_rows: &[u32],
    visible_columns: &[u16],
) -> Result<(), DocumentError> {
    let left = 64_u32;
    let top = 116_u32;
    let row_height = 32_u32;
    let column_width = 176_u32;
    append(
        svg,
        format_args!(
            "<rect x=\"{left}\" y=\"{top}\" width=\"{column_width}\" height=\"{row_height}\" fill=\"#e8edf4\" stroke=\"#cbd5e1\"/>"
        ),
    )?;
    for (column_index, column) in visible_columns.iter().enumerate() {
        let x = left
            .saturating_add(column_width.saturating_mul((column_index as u32).saturating_add(1)));
        append(
            svg,
            format_args!(
                "<rect x=\"{x}\" y=\"{top}\" width=\"{column_width}\" height=\"{row_height}\" fill=\"#e8edf4\" stroke=\"#cbd5e1\"/><text x=\"{}\" y=\"{}\" fill=\"#344054\" font-family=\"Arial,sans-serif\" font-size=\"14\" font-weight=\"700\">{}</text>",
                x.saturating_add(8),
                top.saturating_add(21),
                column_label(*column)
            ),
        )?;
    }
    for (row_index, row) in visible_rows.iter().enumerate() {
        let y = top.saturating_add(row_height.saturating_mul((row_index as u32).saturating_add(1)));
        append(
            svg,
            format_args!(
                "<rect x=\"{left}\" y=\"{y}\" width=\"{column_width}\" height=\"{row_height}\" fill=\"#f8fafc\" stroke=\"#d7dde7\"/><text x=\"{}\" y=\"{}\" fill=\"#667085\" font-family=\"Arial,sans-serif\" font-size=\"13\">{row}</text>",
                left.saturating_add(8),
                y.saturating_add(21)
            ),
        )?;
        for (column_index, column) in visible_columns.iter().enumerate() {
            let x = left.saturating_add(
                column_width.saturating_mul((column_index as u32).saturating_add(1)),
            );
            let value = rows
                .get(row)
                .and_then(|cells| cells.get(column))
                .map_or("", String::as_str);
            let value = wrap_text(value, 20).into_iter().next().unwrap_or_default();
            append(
                svg,
                format_args!(
                    "<rect x=\"{x}\" y=\"{y}\" width=\"{column_width}\" height=\"{row_height}\" fill=\"#ffffff\" stroke=\"#d7dde7\"/><text x=\"{}\" y=\"{}\" fill=\"#344054\" font-family=\"Arial,sans-serif\" font-size=\"13\">{}</text>",
                    x.saturating_add(8),
                    y.saturating_add(21),
                    escape_xml(&value)
                ),
            )?;
        }
    }
    Ok(())
}

fn render_lines(
    svg: &mut String,
    lines: &[String],
    x: u32,
    y: u32,
    line_height: u32,
    color: &str,
) -> Result<(), DocumentError> {
    for (index, line) in lines
        .iter()
        .take(DOCUMENT_PREVIEW_MAX_TEXT_LINES)
        .enumerate()
    {
        let y = y.saturating_add(line_height.saturating_mul(index as u32));
        append(
            svg,
            format_args!(
                "<text x=\"{x}\" y=\"{y}\" fill=\"{color}\" font-family=\"Arial,sans-serif\" font-size=\"18\">{}</text>",
                escape_xml(line)
            ),
        )?;
    }
    Ok(())
}

fn render_image_row(
    svg: &mut String,
    images: &[&PreviewImage],
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    slots: usize,
) -> Result<Vec<PlacedImage>, DocumentError> {
    if images.is_empty() || slots == 0 {
        return Ok(Vec::new());
    }
    let gap = 16_u32;
    let slot_width =
        width.saturating_sub(gap.saturating_mul((slots.saturating_sub(1)) as u32)) / slots as u32;
    let mut placed = Vec::new();
    for (index, image) in images.iter().take(slots).enumerate() {
        let slot_x =
            x.saturating_add((slot_width.saturating_add(gap)).saturating_mul(index as u32));
        let (display_width, display_height) =
            fit_inside(image.width, image.height, slot_width, height);
        let image_x = slot_x.saturating_add(slot_width.saturating_sub(display_width) / 2);
        let image_y = y.saturating_add(height.saturating_sub(display_height) / 2);
        append(
            svg,
            format_args!(
                "<rect x=\"{slot_x}\" y=\"{y}\" width=\"{slot_width}\" height=\"{height}\" rx=\"8\" fill=\"#f8fafc\" stroke=\"#cbd5e1\"/><image x=\"{image_x}\" y=\"{image_y}\" width=\"{display_width}\" height=\"{display_height}\" href=\"{}\"/>",
                image.data_uri
            ),
        )?;
        placed.push(PlacedImage {
            region: DocumentPreviewRegion {
                candidate_id: image.id.clone(),
                x: image_x,
                y: image_y,
                width: display_width,
                height: display_height,
            },
        });
    }
    Ok(placed)
}

fn make_preview(
    kind: DocumentPreviewKind,
    locator: DocumentLocator,
    label: String,
    width: u32,
    height: u32,
    svg: String,
    placed: Vec<PlacedImage>,
) -> Result<DocumentPreview, DocumentError> {
    if width > DOCUMENT_MAX_PREVIEW_DIMENSION || height > DOCUMENT_MAX_PREVIEW_DIMENSION {
        return Err(DocumentError::InvalidArtifact(
            "preview canvas exceeds the safety limit".to_owned(),
        ));
    }
    if svg.len() > DOCUMENT_MAX_PREVIEW_SVG_BYTES {
        return Err(DocumentError::InvalidArtifact(
            "preview SVG exceeds the safety limit".to_owned(),
        ));
    }
    let regions = placed
        .into_iter()
        .map(|item| item.region)
        .collect::<Vec<_>>();
    let digest = format!("sha256:{:x}", Sha256::digest(svg.as_bytes()));
    let preview = DocumentPreview {
        schema: DOCUMENT_PREVIEW_SCHEMA.to_owned(),
        kind,
        locator,
        label,
        width,
        height,
        svg,
        regions,
        digest,
    };
    preview.validate()?;
    Ok(preview)
}

fn svg_start(
    width: u32,
    height: u32,
    background: &str,
    title: &str,
) -> Result<String, DocumentError> {
    let mut svg = String::new();
    append(
        &mut svg,
        format_args!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {width} {height}\" width=\"{width}\" height=\"{height}\" role=\"img\"><title>{}</title><rect width=\"100%\" height=\"100%\" fill=\"{background}\"/>",
            escape_xml(title)
        ),
    )?;
    Ok(svg)
}

fn append(output: &mut String, arguments: Arguments<'_>) -> Result<(), DocumentError> {
    output
        .write_fmt(arguments)
        .map_err(|_| DocumentError::InvalidArtifact("could not build preview SVG".to_owned()))
}

fn wrap_text(text: &str, max_chars: usize) -> Vec<String> {
    let max_chars = max_chars.max(1);
    let mut lines = Vec::new();
    for paragraph in text.lines().take(DOCUMENT_PREVIEW_MAX_TEXT_LINES) {
        let mut current = String::new();
        for word in paragraph
            .split_whitespace()
            .flat_map(|word| wrapped_word_parts(word, max_chars))
        {
            if current.is_empty() {
                current.push_str(&word);
            } else if current
                .chars()
                .count()
                .saturating_add(word.chars().count())
                .saturating_add(1)
                <= max_chars
            {
                current.push(' ');
                current.push_str(&word);
            } else {
                lines.push(current);
                current = word;
            }
            if lines.len() >= DOCUMENT_PREVIEW_MAX_TEXT_LINES {
                break;
            }
        }
        if !current.is_empty() && lines.len() < DOCUMENT_PREVIEW_MAX_TEXT_LINES {
            lines.push(current);
        }
    }
    lines
}

fn wrapped_word_parts(word: &str, max_chars: usize) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    for character in word.chars() {
        current.push(character);
        if current.chars().count() >= max_chars {
            parts.push(std::mem::take(&mut current));
            if parts.len() >= DOCUMENT_PREVIEW_MAX_TEXT_LINES {
                break;
            }
        }
    }
    if !current.is_empty() && parts.len() < DOCUMENT_PREVIEW_MAX_TEXT_LINES {
        parts.push(current);
    }
    if parts.is_empty() {
        parts.push(String::new());
    }
    parts
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn fit_inside(
    source_width: u32,
    source_height: u32,
    max_width: u32,
    max_height: u32,
) -> (u32, u32) {
    let width_ratio = f64::from(max_width) / f64::from(source_width.max(1));
    let height_ratio = f64::from(max_height) / f64::from(source_height.max(1));
    let scale = width_ratio.min(height_ratio).min(1.0);
    (
        (f64::from(source_width) * scale).round().max(1.0) as u32,
        (f64::from(source_height) * scale).round().max(1.0) as u32,
    )
}

fn images_for_index(
    images: &[PreviewImage],
    index: usize,
    count: usize,
    limit: usize,
) -> Vec<&PreviewImage> {
    images
        .iter()
        .enumerate()
        .filter(|(image_index, _)| image_index % count.max(1) == index)
        .map(|(_, image)| image)
        .take(limit)
        .collect()
}

fn images_for_slide(
    images: &[PreviewImage],
    slide: u32,
    index: usize,
    count: usize,
    limit: usize,
) -> Vec<&PreviewImage> {
    let selected = images
        .iter()
        .filter(|image| matches!(&image.owner, DocumentLocator::Slide { slide: value, .. } if *value == slide))
        .take(limit)
        .collect::<Vec<_>>();
    if selected.is_empty() {
        images_for_index(images, index, count, limit)
    } else {
        selected
    }
}

fn images_for_sheet<'a>(
    images: &'a [PreviewImage],
    sheet: &str,
    index: usize,
    count: usize,
    limit: usize,
) -> Vec<&'a PreviewImage> {
    let selected = images
        .iter()
        .filter(|image| matches!(&image.owner, DocumentLocator::Spreadsheet { sheet: value, .. } if value == sheet))
        .take(limit)
        .collect::<Vec<_>>();
    if selected.is_empty() {
        images_for_index(images, index, count, limit)
    } else {
        selected
    }
}

fn slide_number(locator: &DocumentLocator) -> Option<u32> {
    match locator {
        DocumentLocator::Slide { slide, .. } => Some(*slide),
        _ => None,
    }
}

fn column_label(column: u16) -> String {
    let mut value = u32::from(column);
    let mut output = String::new();
    while value > 0 {
        let remainder = (value - 1) % 26;
        output.insert(
            0,
            char::from_u32(u32::from(b'A') + remainder).unwrap_or('A'),
        );
        value = (value - 1) / 26;
    }
    if output.is_empty() {
        "A".to_owned()
    } else {
        output
    }
}

fn one_based(index: usize, label: &str) -> Result<u32, DocumentError> {
    u32::try_from(index)
        .ok()
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| DocumentError::Rejected(format!("{label} index overflow")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnail_dimensions_are_bounded_and_proportional() -> Result<(), Box<dyn std::error::Error>>
    {
        assert_eq!(fit_dimensions(2_000, 1_000, 720)?, (720, 360));
        assert_eq!(fit_dimensions(1_000, 2_000, 720)?, (360, 720));
        assert!(fit_dimensions(0, 1, 720).is_err());
        Ok(())
    }

    #[test]
    fn generated_svg_escapes_untrusted_text() -> Result<(), Box<dyn std::error::Error>> {
        let mut svg = svg_start(400, 300, "#fff", "<unsafe>")?;
        render_lines(
            &mut svg,
            &["<script>alert(1)</script>".to_owned()],
            10,
            20,
            20,
            "#000",
        )?;
        append(&mut svg, format_args!("</svg>"))?;
        assert!(svg.contains("&lt;unsafe&gt;"));
        assert!(svg.contains("&lt;script&gt;"));
        assert!(!svg.contains("<script"));
        Ok(())
    }
}
