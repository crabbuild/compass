//! Bounded Open Packaging Convention reader and native OOXML decoders.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read};

use roxmltree::{Document, Node};
use serde::{Deserialize, Serialize};
use zip::ZipArchive;

use crate::document::{
    DiagnosticSeverity, DocumentArtifact, DocumentBlockKind, DocumentDiagnostic, DocumentError,
    DocumentFormat, DocumentLink, DocumentLinkKind, DocumentLocator,
};
use crate::limits::{
    DOCUMENT_MAX_DEPTH, DOCUMENT_MAX_TEXT_CHARS, OFFICE_MAX_COMPRESSION_RATIO,
    OFFICE_MAX_DECOMPRESSED_BYTES, OFFICE_MAX_MEMBERS, OFFICE_MEMBER_MAX_BYTES, XLSX_MAX_CELLS,
    XLSX_MAX_COLUMNS, XLSX_MAX_ROWS,
};

const XML_MAX_EVENTS: usize = 1_000_000;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RasterCandidate {
    pub id: String,
    pub owner: DocumentLocator,
    pub part: String,
    pub media_type: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
struct Relationship {
    target: String,
    kind: String,
    external: bool,
}

pub(crate) struct Package {
    parts: BTreeMap<String, Vec<u8>>,
}

impl Package {
    pub(crate) fn open(bytes: &[u8]) -> Result<Self, DocumentError> {
        let mut archive = ZipArchive::new(Cursor::new(bytes))
            .map_err(|error| DocumentError::Parse(format!("invalid ZIP container: {error}")))?;
        if archive.len() > OFFICE_MAX_MEMBERS {
            return Err(DocumentError::Rejected(
                "office member count exceeds safety limit".to_owned(),
            ));
        }
        let mut compressed = 0_u64;
        let mut declared = 0_u64;
        let mut names = BTreeSet::new();
        for index in 0..archive.len() {
            let member = archive
                .by_index_raw(index)
                .map_err(|error| DocumentError::Parse(error.to_string()))?;
            compressed = compressed
                .checked_add(member.compressed_size())
                .ok_or_else(|| DocumentError::Rejected("compressed size overflow".to_owned()))?;
            declared = declared
                .checked_add(member.size())
                .ok_or_else(|| DocumentError::Rejected("declared size overflow".to_owned()))?;
            if member.size() > OFFICE_MEMBER_MAX_BYTES {
                return Err(DocumentError::Rejected(format!(
                    "office member {} exceeds safety limit",
                    member.name()
                )));
            }
            if !names.insert(normalize_part_name(member.name())?) {
                return Err(DocumentError::Rejected(
                    "office package contains duplicate normalized part names".to_owned(),
                ));
            }
        }
        if declared > OFFICE_MAX_DECOMPRESSED_BYTES {
            return Err(DocumentError::Rejected(format!(
                "declared office payload is {declared} bytes"
            )));
        }
        let ratio_limit = compressed
            .max(1)
            .saturating_mul(OFFICE_MAX_COMPRESSION_RATIO);
        if declared > ratio_limit {
            return Err(DocumentError::Rejected(
                "office compression ratio exceeds safety limit".to_owned(),
            ));
        }
        let mut actual = 0_u64;
        let mut parts = BTreeMap::new();
        for index in 0..archive.len() {
            let member = archive
                .by_index(index)
                .map_err(|error| DocumentError::Parse(error.to_string()))?;
            let name = normalize_part_name(member.name())?;
            let member_capacity = usize::try_from(member.size()).map_err(|_| {
                DocumentError::Rejected("office member cannot fit in memory".to_owned())
            })?;
            let mut contents = Vec::with_capacity(member_capacity);
            member
                .take(OFFICE_MEMBER_MAX_BYTES.saturating_add(1))
                .read_to_end(&mut contents)
                .map_err(|error| DocumentError::Parse(error.to_string()))?;
            actual = actual
                .checked_add(contents.len() as u64)
                .ok_or_else(|| DocumentError::Rejected("actual size overflow".to_owned()))?;
            if actual > OFFICE_MAX_DECOMPRESSED_BYTES
                || contents.len() as u64 > OFFICE_MEMBER_MAX_BYTES
            {
                return Err(DocumentError::Rejected(
                    "decompressed office payload exceeds safety limit".to_owned(),
                ));
            }
            parts.insert(name, contents);
        }
        Ok(Self { parts })
    }

    fn required_text(&self, name: &str) -> Result<&str, DocumentError> {
        let bytes = self.parts.get(name).ok_or_else(|| {
            DocumentError::Parse(format!("office package is missing required part {name}"))
        })?;
        parse_xml_text(name, bytes)
    }

    fn optional_text(&self, name: &str) -> Result<Option<&str>, DocumentError> {
        self.parts
            .get(name)
            .map(|bytes| parse_xml_text(name, bytes))
            .transpose()
    }

    fn relationships(
        &self,
        source_part: &str,
    ) -> Result<BTreeMap<String, Relationship>, DocumentError> {
        let relationship_part = relationship_part_name(source_part)?;
        let Some(xml) = self.optional_text(&relationship_part)? else {
            return Ok(BTreeMap::new());
        };
        let document = parse_xml(&relationship_part, xml)?;
        let mut relationships = BTreeMap::new();
        for node in document
            .descendants()
            .filter(|node| local_name(*node) == "Relationship")
        {
            let Some(id) = attribute_local(node, "Id") else {
                continue;
            };
            let target = attribute_local(node, "Target").unwrap_or_default();
            let external = attribute_local(node, "TargetMode") == Some("External");
            let resolved = if external {
                bounded(target, "external relationship target")?.to_owned()
            } else {
                resolve_target(source_part, target)?
            };
            let relationship = Relationship {
                target: resolved,
                kind: attribute_local(node, "Type").unwrap_or_default().to_owned(),
                external,
            };
            if relationships.insert(id.to_owned(), relationship).is_some() {
                return Err(DocumentError::Parse(format!(
                    "duplicate relationship ID {id:?} in {relationship_part}"
                )));
            }
        }
        Ok(relationships)
    }
}

pub fn decode_docx(bytes: &[u8]) -> Result<DocumentArtifact, DocumentError> {
    let package = Package::open(bytes)?;
    let styles = package
        .optional_text("word/styles.xml")?
        .map(parse_docx_styles)
        .transpose()?
        .unwrap_or_default();
    let document = parse_xml(
        "word/document.xml",
        package.required_text("word/document.xml")?,
    )?;
    let body = document
        .descendants()
        .find(|node| local_name(*node) == "body")
        .ok_or_else(|| DocumentError::Parse("DOCX document has no body".to_owned()))?;
    let relationships = package.relationships("word/document.xml")?;
    let mut artifact = DocumentArtifact::new(DocumentFormat::Docx);
    let mut occurrence = 0_u32;
    for child in body.children().filter(Node::is_element) {
        occurrence = occurrence
            .checked_add(1)
            .ok_or_else(|| DocumentError::Rejected("DOCX occurrence overflow".to_owned()))?;
        match local_name(child) {
            "p" => emit_docx_paragraph(
                child,
                None,
                occurrence,
                &styles,
                &relationships,
                &mut artifact,
            )?,
            "tbl" => emit_docx_table(child, occurrence, &mut artifact)?,
            "sectPr" => {}
            other => mark_unsupported(
                &mut artifact,
                "docx_unsupported_body_element",
                format!("DOCX body element {other} was not decoded"),
                DocumentLocator::Package {
                    part: "word/document.xml".to_owned(),
                    path: format!("body/*[{occurrence}]"),
                },
            ),
        }
    }
    emit_optional_docx_part(&package, "word/footnotes.xml", "footnote", &mut artifact)?;
    emit_optional_docx_part(&package, "word/endnotes.xml", "endnote", &mut artifact)?;
    emit_optional_docx_part(&package, "word/comments.xml", "comment", &mut artifact)?;
    record_ooxml_images(&package, "word/", &mut artifact);
    artifact.validate()?;
    Ok(artifact)
}

pub fn decode_xlsx(bytes: &[u8]) -> Result<DocumentArtifact, DocumentError> {
    let package = Package::open(bytes)?;
    let workbook = parse_xml("xl/workbook.xml", package.required_text("xl/workbook.xml")?)?;
    let relationships = package.relationships("xl/workbook.xml")?;
    let shared_strings = package
        .optional_text("xl/sharedStrings.xml")?
        .map(parse_shared_strings)
        .transpose()?
        .unwrap_or_default();
    let mut artifact = DocumentArtifact::new(DocumentFormat::Xlsx);
    let mut total_cells = 0_usize;
    for sheet in workbook
        .descendants()
        .filter(|node| local_name(*node) == "sheet")
    {
        let name = bounded(
            attribute_local(sheet, "name").unwrap_or("Sheet"),
            "sheet name",
        )?;
        let relation_id = attribute_local(sheet, "id").unwrap_or_default();
        let relationship = relationships.get(relation_id).ok_or_else(|| {
            DocumentError::Parse(format!("worksheet relationship {relation_id:?} is missing"))
        })?;
        if relationship.external {
            return Err(DocumentError::Parse(
                "worksheet relationship cannot be external".to_owned(),
            ));
        }
        let sheet_block = artifact.push_block(
            None,
            DocumentBlockKind::Sheet,
            name.to_owned(),
            DocumentLocator::Spreadsheet {
                sheet: name.to_owned(),
                row: 1,
                column: 1,
            },
        )?;
        let worksheet = parse_xml(
            &relationship.target,
            package.required_text(&relationship.target)?,
        )?;
        for (row_occurrence, row) in worksheet
            .descendants()
            .filter(|node| local_name(*node) == "row")
            .enumerate()
        {
            let fallback_row = one_based(row_occurrence, "row")?;
            let row_number = attribute_local(row, "r")
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or(fallback_row);
            if row_number == 0 || row_number > XLSX_MAX_ROWS {
                return Err(DocumentError::Rejected(format!(
                    "worksheet row {row_number} exceeds the supported bound"
                )));
            }
            let row_block = artifact.push_block(
                Some(sheet_block),
                DocumentBlockKind::Row,
                String::new(),
                DocumentLocator::Spreadsheet {
                    sheet: name.to_owned(),
                    row: row_number,
                    column: 1,
                },
            )?;
            let mut fallback_column = 1_usize;
            for cell in row.children().filter(|node| local_name(*node) == "c") {
                total_cells = total_cells
                    .checked_add(1)
                    .ok_or_else(|| DocumentError::Rejected("cell count overflow".to_owned()))?;
                if total_cells > XLSX_MAX_CELLS {
                    return Err(DocumentError::Rejected(
                        "worksheet cell count exceeds safety limit".to_owned(),
                    ));
                }
                let (reference_row, column) = attribute_local(cell, "r")
                    .map(parse_cell_reference)
                    .transpose()?
                    .unwrap_or((row_number, fallback_column));
                if reference_row != row_number {
                    return Err(DocumentError::Parse(format!(
                        "cell row {reference_row} does not match containing row {row_number}"
                    )));
                }
                fallback_column = column
                    .checked_add(1)
                    .ok_or_else(|| DocumentError::Rejected("column overflow".to_owned()))?;
                let (value, value_kind) = parse_cell_value(cell, &shared_strings)?;
                let formula = cell
                    .descendants()
                    .find(|node| local_name(*node) == "f")
                    .and_then(|node| node.text());
                if value.is_empty() && formula.is_none() {
                    continue;
                }
                let column_u16 = u16::try_from(column)
                    .map_err(|_| DocumentError::Rejected("column exceeds u16".to_owned()))?;
                let ordinal = artifact.push_block(
                    Some(row_block),
                    DocumentBlockKind::Cell,
                    value,
                    DocumentLocator::Spreadsheet {
                        sheet: name.to_owned(),
                        row: row_number,
                        column: column_u16,
                    },
                )?;
                let block = artifact.blocks.get_mut(ordinal as usize).ok_or_else(|| {
                    DocumentError::InvalidArtifact("new cell block disappeared".to_owned())
                })?;
                block
                    .metadata
                    .insert("value_kind".to_owned(), serde_json::json!(value_kind));
                if let Some(formula) = formula {
                    block.metadata.insert(
                        "formula".to_owned(),
                        serde_json::json!(bounded(formula, "formula")?),
                    );
                }
            }
        }
    }
    record_ooxml_images(&package, "xl/", &mut artifact);
    artifact.validate()?;
    Ok(artifact)
}

pub fn decode_pptx(bytes: &[u8]) -> Result<DocumentArtifact, DocumentError> {
    let package = Package::open(bytes)?;
    let presentation = parse_xml(
        "ppt/presentation.xml",
        package.required_text("ppt/presentation.xml")?,
    )?;
    let relationships = package.relationships("ppt/presentation.xml")?;
    let mut artifact = DocumentArtifact::new(DocumentFormat::Pptx);
    let mut slide_number = 0_u32;
    for slide_id in presentation
        .descendants()
        .filter(|node| local_name(*node) == "sldId")
    {
        slide_number = slide_number
            .checked_add(1)
            .ok_or_else(|| DocumentError::Rejected("slide index overflow".to_owned()))?;
        let relation_id = attribute_local(slide_id, "id").unwrap_or_default();
        let relationship = relationships.get(relation_id).ok_or_else(|| {
            DocumentError::Parse(format!("slide relationship {relation_id:?} is missing"))
        })?;
        if relationship.external {
            return Err(DocumentError::Parse(
                "slide relationship cannot be external".to_owned(),
            ));
        }
        let slide_block = artifact.push_block(
            None,
            DocumentBlockKind::Slide,
            String::new(),
            DocumentLocator::Slide {
                slide: slide_number,
                shape: 1,
            },
        )?;
        let slide = parse_xml(
            &relationship.target,
            package.required_text(&relationship.target)?,
        )?;
        let slide_relationships = package.relationships(&relationship.target)?;
        let shape_tree = slide
            .descendants()
            .find(|node| local_name(*node) == "spTree")
            .unwrap_or(slide.root_element());
        let mut shape_number = 1_u32;
        for shape in shape_tree.children().filter(Node::is_element) {
            let name = local_name(shape);
            if !matches!(name, "sp" | "graphicFrame" | "pic" | "cxnSp") {
                continue;
            }
            shape_number = shape_number
                .checked_add(1)
                .ok_or_else(|| DocumentError::Rejected("shape index overflow".to_owned()))?;
            if name == "graphicFrame" && shape.descendants().any(|node| local_name(node) == "tbl") {
                emit_pptx_table(
                    shape,
                    slide_number,
                    shape_number,
                    slide_block,
                    &mut artifact,
                )?;
                continue;
            }
            let text = collect_text(shape);
            if !text.is_empty() {
                let is_title = shape.descendants().any(|node| {
                    local_name(node) == "ph"
                        && matches!(attribute_local(node, "type"), Some("title" | "ctrTitle"))
                });
                let kind = if is_title {
                    DocumentBlockKind::Heading { level: 1 }
                } else {
                    DocumentBlockKind::Paragraph
                };
                let ordinal = artifact.push_block(
                    Some(slide_block),
                    kind,
                    text,
                    DocumentLocator::Slide {
                        slide: slide_number,
                        shape: shape_number,
                    },
                )?;
                if let Some(c_nv_pr) = shape
                    .descendants()
                    .find(|node| local_name(*node) == "cNvPr")
                {
                    let block = artifact.blocks.get_mut(ordinal as usize).ok_or_else(|| {
                        DocumentError::InvalidArtifact("new shape block disappeared".to_owned())
                    })?;
                    for (key, attribute) in [
                        ("shape_name", "name"),
                        ("alt_text", "descr"),
                        ("title", "title"),
                    ] {
                        if let Some(value) = attribute_local(c_nv_pr, attribute) {
                            block
                                .metadata
                                .insert(key.to_owned(), serde_json::json!(bounded(value, key)?));
                        }
                    }
                }
            }
            emit_shape_links(
                shape,
                slide_number,
                shape_number,
                slide_block,
                &slide_relationships,
                &mut artifact,
            );
        }
        emit_slide_notes(
            &package,
            &relationship.target,
            slide_number,
            slide_block,
            &slide_relationships,
            &mut artifact,
        )?;
        if slide.descendants().any(|node| {
            matches!(
                local_name(node),
                "chart" | "diagram" | "oleObj" | "video" | "audio"
            )
        }) {
            mark_unsupported(
                &mut artifact,
                "pptx_visual_object_not_decoded",
                "A chart, diagram, OLE object, or media item was retained as unsupported evidence"
                    .to_owned(),
                DocumentLocator::Slide {
                    slide: slide_number,
                    shape: 1,
                },
            );
        }
    }
    record_ooxml_images(&package, "ppt/", &mut artifact);
    artifact.validate()?;
    Ok(artifact)
}

pub fn raster_candidates(
    logical_extension: &str,
    bytes: &[u8],
) -> Result<Vec<RasterCandidate>, DocumentError> {
    let package = Package::open(bytes)?;
    let mut candidates = Vec::new();
    match logical_extension {
        "docx" => docx_raster_candidates(&package, &mut candidates)?,
        "pptx" => pptx_raster_candidates(&package, &mut candidates)?,
        "xlsx" => xlsx_raster_candidates(&package, &mut candidates)?,
        _ => {}
    }
    Ok(candidates)
}

fn push_raster_candidate(
    package: &Package,
    relationship: &Relationship,
    owner: DocumentLocator,
    candidates: &mut Vec<RasterCandidate>,
) -> Result<(), DocumentError> {
    if relationship.external || !relationship.kind.ends_with("/image") {
        return Ok(());
    }
    let Some(media_type) = media_type(&relationship.target) else {
        return Ok(());
    };
    let contents = package.parts.get(&relationship.target).ok_or_else(|| {
        DocumentError::Parse(format!(
            "embedded image relationship target {} is missing",
            relationship.target
        ))
    })?;
    let occurrence = one_based(candidates.len(), "image occurrence")?;
    candidates.push(RasterCandidate {
        id: format!("ooxml-image-{occurrence}"),
        owner,
        part: relationship.target.clone(),
        media_type: media_type.to_owned(),
        bytes: contents.clone(),
    });
    Ok(())
}

fn docx_raster_candidates(
    package: &Package,
    candidates: &mut Vec<RasterCandidate>,
) -> Result<(), DocumentError> {
    let source_part = "word/document.xml";
    let document = parse_xml(source_part, package.required_text(source_part)?)?;
    let relationships = package.relationships(source_part)?;
    let body = document
        .descendants()
        .find(|node| local_name(*node) == "body")
        .ok_or_else(|| DocumentError::Parse("DOCX document has no body".to_owned()))?;
    for (child_index, child) in body.children().filter(Node::is_element).enumerate() {
        let mut image_index = 0_usize;
        for blip in child
            .descendants()
            .filter(|node| local_name(*node) == "blip")
        {
            let Some(relation_id) = attribute_local(blip, "embed") else {
                continue;
            };
            let relationship = relationships.get(relation_id).ok_or_else(|| {
                DocumentError::Parse(format!("image relationship {relation_id:?} is missing"))
            })?;
            image_index = image_index.saturating_add(1);
            push_raster_candidate(
                package,
                relationship,
                DocumentLocator::Package {
                    part: source_part.to_owned(),
                    path: format!(
                        "body/*[{}]/image[{image_index}]",
                        one_based(child_index, "DOCX body")?
                    ),
                },
                candidates,
            )?;
        }
    }
    Ok(())
}

fn pptx_raster_candidates(
    package: &Package,
    candidates: &mut Vec<RasterCandidate>,
) -> Result<(), DocumentError> {
    let presentation = parse_xml(
        "ppt/presentation.xml",
        package.required_text("ppt/presentation.xml")?,
    )?;
    let presentation_relationships = package.relationships("ppt/presentation.xml")?;
    for (slide_index, slide_id) in presentation
        .descendants()
        .filter(|node| local_name(*node) == "sldId")
        .enumerate()
    {
        let slide_number = one_based(slide_index, "slide")?;
        let relation_id = attribute_local(slide_id, "id").unwrap_or_default();
        let slide_relationship = presentation_relationships.get(relation_id).ok_or_else(|| {
            DocumentError::Parse(format!("slide relationship {relation_id:?} is missing"))
        })?;
        if slide_relationship.external {
            continue;
        }
        let slide = parse_xml(
            &slide_relationship.target,
            package.required_text(&slide_relationship.target)?,
        )?;
        let relationships = package.relationships(&slide_relationship.target)?;
        let shape_tree = slide
            .descendants()
            .find(|node| local_name(*node) == "spTree")
            .unwrap_or(slide.root_element());
        let mut shape_number = 1_u32;
        for shape in shape_tree.children().filter(Node::is_element) {
            if !matches!(local_name(shape), "sp" | "graphicFrame" | "pic" | "cxnSp") {
                continue;
            }
            shape_number = shape_number
                .checked_add(1)
                .ok_or_else(|| DocumentError::Rejected("shape index overflow".to_owned()))?;
            for blip in shape
                .descendants()
                .filter(|node| local_name(*node) == "blip")
            {
                let Some(relation_id) = attribute_local(blip, "embed") else {
                    continue;
                };
                let relationship = relationships.get(relation_id).ok_or_else(|| {
                    DocumentError::Parse(format!(
                        "slide image relationship {relation_id:?} is missing"
                    ))
                })?;
                push_raster_candidate(
                    package,
                    relationship,
                    DocumentLocator::Slide {
                        slide: slide_number,
                        shape: shape_number,
                    },
                    candidates,
                )?;
            }
        }
    }
    Ok(())
}

fn xlsx_raster_candidates(
    package: &Package,
    candidates: &mut Vec<RasterCandidate>,
) -> Result<(), DocumentError> {
    let workbook = parse_xml("xl/workbook.xml", package.required_text("xl/workbook.xml")?)?;
    let workbook_relationships = package.relationships("xl/workbook.xml")?;
    for sheet in workbook
        .descendants()
        .filter(|node| local_name(*node) == "sheet")
    {
        let name = bounded(
            attribute_local(sheet, "name").unwrap_or("Sheet"),
            "sheet name",
        )?;
        let relation_id = attribute_local(sheet, "id").unwrap_or_default();
        let sheet_relationship = workbook_relationships.get(relation_id).ok_or_else(|| {
            DocumentError::Parse(format!("worksheet relationship {relation_id:?} is missing"))
        })?;
        if sheet_relationship.external {
            continue;
        }
        let worksheet = parse_xml(
            &sheet_relationship.target,
            package.required_text(&sheet_relationship.target)?,
        )?;
        let sheet_relationships = package.relationships(&sheet_relationship.target)?;
        for drawing in worksheet
            .descendants()
            .filter(|node| local_name(*node) == "drawing")
        {
            let Some(drawing_id) = attribute_local(drawing, "id") else {
                continue;
            };
            let drawing_relationship = sheet_relationships.get(drawing_id).ok_or_else(|| {
                DocumentError::Parse(format!("drawing relationship {drawing_id:?} is missing"))
            })?;
            if drawing_relationship.external {
                continue;
            }
            let drawing_document = parse_xml(
                &drawing_relationship.target,
                package.required_text(&drawing_relationship.target)?,
            )?;
            let image_relationships = package.relationships(&drawing_relationship.target)?;
            for anchor in drawing_document
                .root_element()
                .children()
                .filter(Node::is_element)
            {
                if !matches!(
                    local_name(anchor),
                    "oneCellAnchor" | "twoCellAnchor" | "absoluteAnchor"
                ) {
                    continue;
                }
                let from = anchor.children().find(|node| local_name(*node) == "from");
                let row = from
                    .and_then(|node| node.children().find(|child| local_name(*child) == "row"))
                    .and_then(|node| node.text())
                    .and_then(|value| value.parse::<u32>().ok())
                    .and_then(|value| value.checked_add(1))
                    .unwrap_or(1);
                let column = from
                    .and_then(|node| node.children().find(|child| local_name(*child) == "col"))
                    .and_then(|node| node.text())
                    .and_then(|value| value.parse::<u16>().ok())
                    .and_then(|value| value.checked_add(1))
                    .unwrap_or(1);
                for blip in anchor
                    .descendants()
                    .filter(|node| local_name(*node) == "blip")
                {
                    let Some(image_id) = attribute_local(blip, "embed") else {
                        continue;
                    };
                    let relationship = image_relationships.get(image_id).ok_or_else(|| {
                        DocumentError::Parse(format!(
                            "drawing image relationship {image_id:?} is missing"
                        ))
                    })?;
                    push_raster_candidate(
                        package,
                        relationship,
                        DocumentLocator::Spreadsheet {
                            sheet: name.to_owned(),
                            row,
                            column,
                        },
                        candidates,
                    )?;
                }
            }
        }
    }
    Ok(())
}

fn emit_docx_paragraph(
    node: Node<'_, '_>,
    parent: Option<u32>,
    occurrence: u32,
    styles: &BTreeMap<String, String>,
    relationships: &BTreeMap<String, Relationship>,
    artifact: &mut DocumentArtifact,
) -> Result<(), DocumentError> {
    let text = collect_docx_text(node);
    let style_id = node
        .descendants()
        .find(|descendant| local_name(*descendant) == "pStyle")
        .and_then(|style| attribute_local(style, "val"))
        .unwrap_or_default();
    let style_name = styles.get(style_id).map_or(style_id, String::as_str);
    let normalized_style = style_name.to_ascii_lowercase();
    let kind = if let Some(level) = heading_level(&normalized_style) {
        DocumentBlockKind::Heading { level }
    } else if node
        .descendants()
        .any(|descendant| local_name(descendant) == "numPr")
        || normalized_style.starts_with("list")
    {
        DocumentBlockKind::ListItem
    } else {
        DocumentBlockKind::Paragraph
    };
    let ordinal = artifact.push_block(
        parent,
        kind,
        text,
        DocumentLocator::Package {
            part: "word/document.xml".to_owned(),
            path: format!("body/p[{occurrence}]"),
        },
    )?;
    for hyperlink in node
        .descendants()
        .filter(|node| local_name(*node) == "hyperlink")
    {
        let Some(relation_id) = attribute_local(hyperlink, "id") else {
            continue;
        };
        let Some(relationship) = relationships.get(relation_id) else {
            continue;
        };
        artifact.links.push(DocumentLink {
            source_block: ordinal,
            destination: relationship.target.clone(),
            label: Some(collect_docx_text(hyperlink)),
            relationship: DocumentLinkKind::Hyperlink,
            locator: DocumentLocator::Package {
                part: "word/document.xml".to_owned(),
                path: format!("body/p[{occurrence}]/hyperlink"),
            },
            external: relationship.external,
        });
    }
    Ok(())
}

fn emit_docx_table(
    table: Node<'_, '_>,
    occurrence: u32,
    artifact: &mut DocumentArtifact,
) -> Result<(), DocumentError> {
    let table_block = artifact.push_block(
        None,
        DocumentBlockKind::Table,
        String::new(),
        DocumentLocator::Package {
            part: "word/document.xml".to_owned(),
            path: format!("body/tbl[{occurrence}]"),
        },
    )?;
    for (row_index, row) in table
        .children()
        .filter(|node| local_name(*node) == "tr")
        .enumerate()
    {
        let row_number = one_based(row_index, "DOCX row")?;
        let row_block = artifact.push_block(
            Some(table_block),
            DocumentBlockKind::Row,
            String::new(),
            DocumentLocator::Package {
                part: "word/document.xml".to_owned(),
                path: format!("body/tbl[{occurrence}]/tr[{row_number}]"),
            },
        )?;
        for (cell_index, cell) in row
            .children()
            .filter(|node| local_name(*node) == "tc")
            .enumerate()
        {
            let cell_number = one_based(cell_index, "DOCX cell")?;
            artifact.push_block(
                Some(row_block),
                DocumentBlockKind::Cell,
                cell.children()
                    .filter(|node| local_name(*node) == "p")
                    .map(collect_docx_text)
                    .collect::<Vec<_>>()
                    .join("\n"),
                DocumentLocator::Package {
                    part: "word/document.xml".to_owned(),
                    path: format!("body/tbl[{occurrence}]/tr[{row_number}]/tc[{cell_number}]"),
                },
            )?;
        }
    }
    Ok(())
}

fn emit_optional_docx_part(
    package: &Package,
    part: &str,
    element_name: &str,
    artifact: &mut DocumentArtifact,
) -> Result<(), DocumentError> {
    let Some(xml) = package.optional_text(part)? else {
        return Ok(());
    };
    let document = parse_xml(part, xml)?;
    for (index, node) in document
        .descendants()
        .filter(|node| local_name(*node) == element_name)
        .enumerate()
    {
        let text = collect_docx_text(node);
        if text.is_empty() {
            continue;
        }
        let occurrence = one_based(index, "DOCX note")?;
        artifact.push_block(
            None,
            DocumentBlockKind::Note,
            text,
            DocumentLocator::Package {
                part: part.to_owned(),
                path: format!("{element_name}[{occurrence}]"),
            },
        )?;
    }
    Ok(())
}

fn parse_docx_styles(xml: &str) -> Result<BTreeMap<String, String>, DocumentError> {
    let document = parse_xml("word/styles.xml", xml)?;
    let mut styles = BTreeMap::new();
    for style in document
        .descendants()
        .filter(|node| local_name(*node) == "style")
    {
        let Some(id) = attribute_local(style, "styleId") else {
            continue;
        };
        let name = style
            .children()
            .find(|node| local_name(*node) == "name")
            .and_then(|node| attribute_local(node, "val"))
            .unwrap_or(id);
        styles.insert(id.to_owned(), name.to_owned());
    }
    Ok(styles)
}

fn parse_shared_strings(xml: &str) -> Result<Vec<String>, DocumentError> {
    let document = parse_xml("xl/sharedStrings.xml", xml)?;
    let values = document
        .descendants()
        .filter(|node| local_name(*node) == "si")
        .map(collect_text)
        .collect::<Vec<_>>();
    let characters = values.iter().try_fold(0_usize, |total, value| {
        total
            .checked_add(value.chars().count())
            .ok_or_else(|| DocumentError::Rejected("shared string size overflow".to_owned()))
    })?;
    if characters > DOCUMENT_MAX_TEXT_CHARS {
        return Err(DocumentError::Rejected(
            "shared strings exceed document text limit".to_owned(),
        ));
    }
    Ok(values)
}

fn parse_cell_value(
    cell: Node<'_, '_>,
    shared_strings: &[String],
) -> Result<(String, &'static str), DocumentError> {
    let kind = attribute_local(cell, "t").unwrap_or_default();
    let raw = cell
        .descendants()
        .find(|node| local_name(*node) == "v")
        .and_then(|node| node.text())
        .unwrap_or_default();
    match kind {
        "s" => {
            let index = raw.parse::<usize>().map_err(|_| {
                DocumentError::Parse(format!("invalid shared string index {raw:?}"))
            })?;
            Ok((
                shared_strings
                    .get(index)
                    .ok_or_else(|| {
                        DocumentError::Parse(format!("shared string index {index} is missing"))
                    })?
                    .clone(),
                "shared_string",
            ))
        }
        "inlineStr" => Ok((collect_text(cell), "inline_string")),
        "b" => match raw {
            "1" => Ok(("True".to_owned(), "boolean")),
            "0" => Ok(("False".to_owned(), "boolean")),
            _ => Err(DocumentError::Parse(format!(
                "invalid spreadsheet boolean {raw:?}"
            ))),
        },
        "e" => Ok((raw.to_owned(), "error")),
        "d" => Ok((raw.to_owned(), "iso_date")),
        "str" => Ok((raw.to_owned(), "formula_string")),
        "" | "n" => Ok((raw.to_owned(), "number")),
        other => Err(DocumentError::Parse(format!(
            "unsupported spreadsheet cell type {other:?}"
        ))),
    }
}

fn parse_cell_reference(reference: &str) -> Result<(u32, usize), DocumentError> {
    let letters = reference
        .bytes()
        .take_while(u8::is_ascii_alphabetic)
        .collect::<Vec<_>>();
    if letters.is_empty() || letters.len() > 3 {
        return Err(DocumentError::Rejected(format!(
            "invalid spreadsheet cell reference {reference:?}"
        )));
    }
    let mut column = 0_usize;
    for &byte in &letters {
        let digit = usize::from(byte.to_ascii_uppercase() - b'A' + 1);
        column = column
            .checked_mul(26)
            .and_then(|value| value.checked_add(digit))
            .ok_or_else(|| DocumentError::Rejected("spreadsheet column overflow".to_owned()))?;
    }
    if column == 0 || column > XLSX_MAX_COLUMNS {
        return Err(DocumentError::Rejected(format!(
            "spreadsheet column {column} exceeds the supported bound"
        )));
    }
    let row = reference
        .get(letters.len()..)
        .unwrap_or_default()
        .parse::<u32>()
        .map_err(|_| {
            DocumentError::Rejected(format!("invalid spreadsheet row in {reference:?}"))
        })?;
    if row == 0 || row > XLSX_MAX_ROWS {
        return Err(DocumentError::Rejected(format!(
            "spreadsheet row {row} exceeds the supported bound"
        )));
    }
    Ok((row, column))
}

fn emit_pptx_table(
    shape: Node<'_, '_>,
    slide: u32,
    shape_number: u32,
    slide_block: u32,
    artifact: &mut DocumentArtifact,
) -> Result<(), DocumentError> {
    let table = shape
        .descendants()
        .find(|node| local_name(*node) == "tbl")
        .ok_or_else(|| DocumentError::Parse("PPTX table disappeared".to_owned()))?;
    let table_block = artifact.push_block(
        Some(slide_block),
        DocumentBlockKind::Table,
        String::new(),
        DocumentLocator::Slide {
            slide,
            shape: shape_number,
        },
    )?;
    for (row_index, row) in table
        .children()
        .filter(|node| local_name(*node) == "tr")
        .enumerate()
    {
        let row_shape = shape_number
            .checked_add(one_based(row_index, "PPTX row")?)
            .ok_or_else(|| DocumentError::Rejected("PPTX shape overflow".to_owned()))?;
        let row_block = artifact.push_block(
            Some(table_block),
            DocumentBlockKind::Row,
            String::new(),
            DocumentLocator::Slide {
                slide,
                shape: row_shape,
            },
        )?;
        for (cell_index, cell) in row
            .children()
            .filter(|node| local_name(*node) == "tc")
            .enumerate()
        {
            let cell_shape = row_shape
                .checked_add(one_based(cell_index, "PPTX cell")?)
                .ok_or_else(|| DocumentError::Rejected("PPTX cell locator overflow".to_owned()))?;
            artifact.push_block(
                Some(row_block),
                DocumentBlockKind::Cell,
                collect_text(cell),
                DocumentLocator::Slide {
                    slide,
                    shape: cell_shape,
                },
            )?;
        }
    }
    Ok(())
}

fn emit_shape_links(
    shape: Node<'_, '_>,
    slide: u32,
    shape_number: u32,
    source_block: u32,
    relationships: &BTreeMap<String, Relationship>,
    artifact: &mut DocumentArtifact,
) {
    for hyperlink in shape
        .descendants()
        .filter(|node| matches!(local_name(*node), "hlinkClick" | "hlinkHover"))
    {
        let Some(relation_id) = attribute_local(hyperlink, "id") else {
            continue;
        };
        let Some(relationship) = relationships.get(relation_id) else {
            continue;
        };
        artifact.links.push(DocumentLink {
            source_block,
            destination: relationship.target.clone(),
            label: None,
            relationship: DocumentLinkKind::Hyperlink,
            locator: DocumentLocator::Slide {
                slide,
                shape: shape_number,
            },
            external: relationship.external,
        });
    }
}

fn emit_slide_notes(
    package: &Package,
    slide_part: &str,
    slide: u32,
    slide_block: u32,
    relationships: &BTreeMap<String, Relationship>,
    artifact: &mut DocumentArtifact,
) -> Result<(), DocumentError> {
    let notes = relationships
        .values()
        .find(|relationship| relationship.kind.ends_with("/notesSlide") && !relationship.external);
    let Some(notes) = notes else {
        return Ok(());
    };
    let document = parse_xml(&notes.target, package.required_text(&notes.target)?)?;
    let text = collect_text(document.root_element());
    if !text.is_empty() {
        artifact.push_block(
            Some(slide_block),
            DocumentBlockKind::Note,
            text,
            DocumentLocator::Package {
                part: notes.target.clone(),
                path: format!("notes-for:{slide_part}:{slide}"),
            },
        )?;
    }
    Ok(())
}

fn record_ooxml_images(package: &Package, prefix: &str, artifact: &mut DocumentArtifact) {
    let count = package
        .parts
        .keys()
        .filter(|name| name.starts_with(prefix) && name.contains("/media/"))
        .count();
    if count > 0 {
        artifact
            .metadata
            .insert("embedded_image_count".to_owned(), serde_json::json!(count));
        artifact.diagnostics.push(DocumentDiagnostic {
            code: "embedded_images_available_for_ocr".to_owned(),
            severity: DiagnosticSeverity::Info,
            locator: None,
            message: format!("{count} embedded image part(s) are available for selective OCR"),
        });
    }
}

fn mark_unsupported(
    artifact: &mut DocumentArtifact,
    code: &str,
    message: String,
    locator: DocumentLocator,
) {
    artifact.complete = false;
    artifact.diagnostics.push(DocumentDiagnostic {
        code: code.to_owned(),
        severity: DiagnosticSeverity::Warning,
        locator: Some(locator),
        message,
    });
}

fn parse_xml_text<'a>(name: &str, bytes: &'a [u8]) -> Result<&'a str, DocumentError> {
    if bytes.iter().filter(|byte| **byte == b'<').count() > XML_MAX_EVENTS {
        return Err(DocumentError::Rejected(format!(
            "XML part {name} exceeds the event limit"
        )));
    }
    std::str::from_utf8(bytes)
        .map_err(|error| DocumentError::Parse(format!("XML part {name} is not UTF-8: {error}")))
}

fn parse_xml<'a>(name: &str, xml: &'a str) -> Result<Document<'a>, DocumentError> {
    let document = Document::parse(xml)
        .map_err(|error| DocumentError::Parse(format!("invalid XML part {name}: {error}")))?;
    if document
        .descendants()
        .any(|node| node.ancestors().take(DOCUMENT_MAX_DEPTH + 1).count() > DOCUMENT_MAX_DEPTH)
    {
        return Err(DocumentError::Rejected(format!(
            "XML part {name} exceeds the nesting limit"
        )));
    }
    Ok(document)
}

fn normalize_part_name(name: &str) -> Result<String, DocumentError> {
    let replaced = name.replace('\\', "/");
    if replaced.starts_with('/') || replaced.contains('\0') {
        return Err(DocumentError::Rejected(
            "office part name is absolute or invalid".to_owned(),
        ));
    }
    let mut components = Vec::new();
    for component in replaced.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                if components.pop().is_none() {
                    return Err(DocumentError::Rejected(
                        "office part escapes package root".to_owned(),
                    ));
                }
            }
            value => components.push(value),
        }
    }
    if components.is_empty() {
        return Err(DocumentError::Rejected(
            "office part name is empty".to_owned(),
        ));
    }
    Ok(components.join("/"))
}

fn resolve_target(source_part: &str, target: &str) -> Result<String, DocumentError> {
    if target.starts_with('/') {
        return normalize_part_name(target.trim_start_matches('/'));
    }
    let directory = source_part
        .rsplit_once('/')
        .map_or("", |(directory, _)| directory);
    normalize_part_name(&format!("{directory}/{target}"))
}

fn relationship_part_name(source_part: &str) -> Result<String, DocumentError> {
    let (directory, file) = source_part
        .rsplit_once('/')
        .map_or(("", source_part), |(directory, file)| (directory, file));
    normalize_part_name(&format!("{directory}/_rels/{file}.rels"))
}

fn collect_text(node: Node<'_, '_>) -> String {
    node.descendants()
        .filter(|descendant| local_name(*descendant) == "t")
        .filter_map(|descendant| descendant.text())
        .collect::<String>()
}

fn collect_docx_text(node: Node<'_, '_>) -> String {
    let mut text = String::new();
    for descendant in node.descendants().filter(Node::is_element) {
        match local_name(descendant) {
            "t" | "delText" | "instrText" => {
                if let Some(value) = descendant.text() {
                    text.push_str(value);
                }
            }
            "tab" => text.push('\t'),
            "br" | "cr" => text.push('\n'),
            _ => {}
        }
    }
    text
}

fn heading_level(style: &str) -> Option<u8> {
    let suffix = style.strip_prefix("heading")?.trim();
    suffix
        .parse::<u8>()
        .ok()
        .filter(|level| (1..=6).contains(level))
}

fn local_name<'input>(node: Node<'_, 'input>) -> &'input str {
    if node.is_element() {
        node.tag_name().name()
    } else {
        ""
    }
}

fn attribute_local<'a>(node: Node<'a, 'a>, name: &str) -> Option<&'a str> {
    node.attributes()
        .find(|attribute| attribute.name() == name)
        .map(|attribute| attribute.value())
}

fn bounded<'a>(value: &'a str, field: &str) -> Result<&'a str, DocumentError> {
    if value.len() > crate::limits::DOCUMENT_MAX_FIELD_BYTES || value.contains('\0') {
        return Err(DocumentError::Rejected(format!(
            "{field} exceeds its bound"
        )));
    }
    Ok(value)
}

fn one_based(index: usize, field: &str) -> Result<u32, DocumentError> {
    u32::try_from(index)
        .ok()
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| DocumentError::Rejected(format!("{field} index overflow")))
}

fn media_type(part: &str) -> Option<&'static str> {
    match part.rsplit('.').next()?.to_ascii_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "tif" | "tiff" => Some("image/tiff"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write as _};

    use zip::write::SimpleFileOptions;

    use super::*;

    #[test]
    fn rejects_escaping_targets_and_invalid_sparse_coordinates() {
        assert!(resolve_target("word/document.xml", "../../escape.xml").is_err());
        assert_eq!(
            parse_cell_reference("XFD100000").ok(),
            Some((100_000, 16_384))
        );
        assert!(parse_cell_reference("XFE1").is_err());
        assert!(parse_cell_reference("ZZZZ1").is_err());
        assert_eq!(media_type("word/media/photo.PNG"), Some("image/png"));
        assert_eq!(media_type("word/media/vector.svg"), None);
    }

    #[test]
    fn normalizes_safe_targets() {
        assert_eq!(
            resolve_target("ppt/presentation.xml", "slides/slide2.xml").ok(),
            Some("ppt/slides/slide2.xml".to_owned())
        );
        assert_eq!(
            relationship_part_name("ppt/slides/slide1.xml").ok(),
            Some("ppt/slides/_rels/slide1.xml.rels".to_owned())
        );
    }

    #[test]
    fn docx_candidates_follow_only_internal_image_relationship_occurrences()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
        archive.start_file("word/document.xml", SimpleFileOptions::default())?;
        archive.write_all(br#"<w:document xmlns:w="urn:w" xmlns:a="urn:a" xmlns:r="urn:r"><w:body><w:p><a:blip r:embed="image"/></w:p><w:p><a:blip r:embed="image"/><a:blip r:link="external"/></w:p></w:body></w:document>"#)?;
        archive.start_file("word/_rels/document.xml.rels", SimpleFileOptions::default())?;
        archive.write_all(br#"<Relationships xmlns="urn:rels"><Relationship Id="image" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/shared.png"/><Relationship Id="external" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="https://example.invalid/image.png" TargetMode="External"/></Relationships>"#)?;
        archive.start_file("word/media/shared.png", SimpleFileOptions::default())?;
        archive.write_all(b"fixture")?;
        archive.start_file("word/media/unreferenced.png", SimpleFileOptions::default())?;
        archive.write_all(b"must not be selected")?;
        let bytes = archive.finish()?.into_inner();

        let candidates = raster_candidates("docx", &bytes)?;
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].bytes, candidates[1].bytes);
        assert_eq!(candidates[0].part, "word/media/shared.png");
        assert_ne!(candidates[0].owner, candidates[1].owner);
        assert!(matches!(
            &candidates[0].owner,
            DocumentLocator::Package { part, path }
                if part == "word/document.xml" && path == "body/*[1]/image[1]"
        ));
        assert!(matches!(
            &candidates[1].owner,
            DocumentLocator::Package { path, .. } if path == "body/*[2]/image[1]"
        ));
        Ok(())
    }

    #[test]
    fn pptx_uses_presentation_relationship_order_and_shape_locators()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
        archive.start_file("ppt/presentation.xml", SimpleFileOptions::default())?;
        archive.write_all(br#"<p:presentation xmlns:p="urn:p" xmlns:r="urn:r"><p:sldIdLst><p:sldId r:id="second"/><p:sldId r:id="first"/></p:sldIdLst></p:presentation>"#)?;
        archive.start_file(
            "ppt/_rels/presentation.xml.rels",
            SimpleFileOptions::default(),
        )?;
        archive.write_all(br#"<Relationships xmlns="urn:rels"><Relationship Id="first" Type="urn/slide" Target="slides/slide1.xml"/><Relationship Id="second" Type="urn/slide" Target="slides/slide2.xml"/></Relationships>"#)?;
        for (name, text) in [("slide1.xml", "First"), ("slide2.xml", "Second")] {
            archive.start_file(format!("ppt/slides/{name}"), SimpleFileOptions::default())?;
            let image = if name == "slide2.xml" {
                r#"<p:pic><p:blipFill><a:blip r:embed="image"/></p:blipFill></p:pic>"#
            } else {
                ""
            };
            archive.write_all(format!(r#"<p:sld xmlns:p="urn:p" xmlns:a="urn:a" xmlns:r="urn:r"><p:spTree><p:sp><p:txBody><a:p><a:r><a:t>{text}</a:t></a:r></a:p></p:txBody></p:sp>{image}</p:spTree></p:sld>"#).as_bytes())?;
        }
        archive.start_file(
            "ppt/slides/_rels/slide2.xml.rels",
            SimpleFileOptions::default(),
        )?;
        archive.write_all(br#"<Relationships xmlns="urn:rels"><Relationship Id="image" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/image.png"/></Relationships>"#)?;
        archive.start_file("ppt/media/image.png", SimpleFileOptions::default())?;
        archive.write_all(b"fixture")?;
        let bytes = archive.finish()?.into_inner();
        let artifact = decode_pptx(&bytes)?;
        let text = artifact
            .blocks
            .iter()
            .filter(|block| !block.text.is_empty())
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>();
        assert_eq!(text, ["Second", "First"]);
        assert!(matches!(
            artifact
                .blocks
                .iter()
                .find(|block| block.text == "Second")
                .map(|block| &block.locator),
            Some(DocumentLocator::Slide { slide: 1, shape: 2 })
        ));
        let candidates = raster_candidates("pptx", &bytes)?;
        assert_eq!(candidates.len(), 1);
        assert!(matches!(
            candidates[0].owner,
            DocumentLocator::Slide { slide: 1, shape: 3 }
        ));
        Ok(())
    }

    #[test]
    fn xlsx_image_candidates_retain_sheet_anchor_coordinates()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
        archive.start_file("xl/workbook.xml", SimpleFileOptions::default())?;
        archive.write_all(br#"<workbook xmlns:r="urn:r"><sheets><sheet name="Data" r:id="sheet"/></sheets></workbook>"#)?;
        archive.start_file("xl/_rels/workbook.xml.rels", SimpleFileOptions::default())?;
        archive.write_all(br#"<Relationships><Relationship Id="sheet" Type="urn/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#)?;
        archive.start_file("xl/worksheets/sheet1.xml", SimpleFileOptions::default())?;
        archive.write_all(
            br#"<worksheet xmlns:r="urn:r"><sheetData/><drawing r:id="drawing"/></worksheet>"#,
        )?;
        archive.start_file(
            "xl/worksheets/_rels/sheet1.xml.rels",
            SimpleFileOptions::default(),
        )?;
        archive.write_all(br#"<Relationships><Relationship Id="drawing" Type="urn/drawing" Target="../drawings/drawing1.xml"/></Relationships>"#)?;
        archive.start_file("xl/drawings/drawing1.xml", SimpleFileOptions::default())?;
        archive.write_all(br#"<xdr:wsDr xmlns:xdr="urn:xdr" xmlns:a="urn:a" xmlns:r="urn:r"><xdr:twoCellAnchor><xdr:from><xdr:col>2</xdr:col><xdr:row>4</xdr:row></xdr:from><xdr:pic><a:blip r:embed="image"/></xdr:pic></xdr:twoCellAnchor></xdr:wsDr>"#)?;
        archive.start_file(
            "xl/drawings/_rels/drawing1.xml.rels",
            SimpleFileOptions::default(),
        )?;
        archive.write_all(br#"<Relationships><Relationship Id="image" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/image.png"/></Relationships>"#)?;
        archive.start_file("xl/media/image.png", SimpleFileOptions::default())?;
        archive.write_all(b"fixture")?;

        let candidates = raster_candidates("xlsx", &archive.finish()?.into_inner())?;
        assert_eq!(candidates.len(), 1);
        assert!(matches!(
            &candidates[0].owner,
            DocumentLocator::Spreadsheet { sheet, row: 5, column: 3 } if sheet == "Data"
        ));
        Ok(())
    }
}
