use std::collections::{BTreeMap, BTreeSet};

use base64::Engine;
use compass_ir::{
    Capability, Coverage, CoverageState, ProviderDescriptor, ProviderKind, SourceAnchor,
    canonical_json_bytes, hex_sha256,
};
use protobuf::Message;
use scip::types::{
    Document, Metadata, Occurrence, PositionEncoding, Relationship, SymbolInformation, SymbolRole,
    TextEncoding, occurrence,
};
use serde::{Deserialize, Serialize};

use crate::manifest::{manifest_digest, validate_manifest};
use crate::scip_stream::{read_metadata, verify_reader, visit_documents};
use crate::{
    ArtifactInput, ArtifactProvider, ArtifactReader, EvidenceBatch, EvidenceFact, FactKind,
    ProviderError, Role, evidence_record, normalize_source_path,
};

pub const SCIP_PROVIDER_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DecodedScipDocument {
    pub path: String,
    pub protobuf_base64: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DecodedScipArtifact {
    pub metadata_protobuf_base64: String,
    pub documents: Vec<DecodedScipDocument>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct OfficialScipProvider;

impl OfficialScipProvider {
    pub fn decode_artifact(
        &self,
        input: ArtifactInput<'_>,
        reader: &mut dyn ArtifactReader,
    ) -> Result<DecodedScipArtifact, ProviderError> {
        verify_reader(reader, input.byte_len, input.input_digest, input.limits)?;
        if let Some(manifest) = input.manifest {
            validate_manifest(manifest, input.input_digest)?;
        }
        let metadata = read_metadata(reader, input.limits)?;
        validate_metadata(&metadata)?;
        let metadata_protobuf_base64 = base64::engine::general_purpose::STANDARD
            .encode(metadata.write_to_bytes().map_err(protobuf_error)?);
        let mut paths = BTreeSet::new();
        let mut documents = Vec::new();
        visit_documents(reader, input.limits, |document| {
            let path = normalize_source_path(&document.relative_path)?;
            if !paths.insert(path.clone()) {
                return Err(ProviderError::InvalidInput(format!(
                    "duplicate normalized SCIP document path {path}"
                )));
            }
            documents.push(DecodedScipDocument {
                path,
                protobuf_base64: base64::engine::general_purpose::STANDARD
                    .encode(document.write_to_bytes().map_err(protobuf_error)?),
            });
            Ok(())
        })?;
        documents.sort_by(|left, right| left.path.as_bytes().cmp(right.path.as_bytes()));
        Ok(DecodedScipArtifact {
            metadata_protobuf_base64,
            documents,
        })
    }

    pub fn normalize_decoded(
        &self,
        input: ArtifactInput<'_>,
        decoded: &DecodedScipArtifact,
    ) -> Result<EvidenceBatch, ProviderError> {
        if let Some(manifest) = input.manifest {
            validate_manifest(manifest, input.input_digest)?;
        }
        let metadata_bytes = base64::engine::general_purpose::STANDARD
            .decode(&decoded.metadata_protobuf_base64)
            .map_err(|error| ProviderError::MalformedArtifact(error.to_string()))?;
        if metadata_bytes.len() as u64 > input.limits.max_metadata_bytes {
            return Err(ProviderError::ResourceLimit(
                "cached SCIP metadata exceeds configured limit".to_owned(),
            ));
        }
        let metadata = Metadata::parse_from_bytes(&metadata_bytes).map_err(protobuf_error)?;
        validate_metadata(&metadata)?;
        let mut batch = EvidenceBatch {
            descriptor: self.descriptor(&input),
            evidence: Vec::new(),
            modules: Vec::new(),
            facts: Vec::new(),
            coverage: BTreeMap::new(),
        };
        add_tool_evidence(&metadata, &mut batch);
        let mut paths = BTreeSet::new();
        for cached in &decoded.documents {
            let path = normalize_source_path(&cached.path)?;
            if !paths.insert(path.clone()) {
                return Err(ProviderError::InvalidInput(format!(
                    "duplicate normalized SCIP document path {path}"
                )));
            }
            let document_bytes = base64::engine::general_purpose::STANDARD
                .decode(&cached.protobuf_base64)
                .map_err(|error| ProviderError::MalformedArtifact(error.to_string()))?;
            if document_bytes.len() as u64 > input.limits.max_document_bytes {
                return Err(ProviderError::ResourceLimit(format!(
                    "cached SCIP document {path} exceeds configured limit"
                )));
            }
            let document = Document::parse_from_bytes(&document_bytes).map_err(protobuf_error)?;
            if normalize_source_path(&document.relative_path)? != path {
                return Err(ProviderError::MalformedArtifact(format!(
                    "cached SCIP document path mismatch for {path}"
                )));
            }
            let freshness = freshness(&input, &path)?;
            if freshness == Freshness::Stale {
                batch
                    .coverage
                    .insert(path, coverage_for_document(Freshness::Stale));
                continue;
            }
            normalize_document(&input, &metadata, &document, &path, freshness, &mut batch)?;
        }
        Ok(batch.canonicalized())
    }
}

impl ArtifactProvider for OfficialScipProvider {
    fn descriptor(&self, input: &ArtifactInput<'_>) -> ProviderDescriptor {
        let manifest = manifest_digest(input.manifest);
        let bound_sources = input
            .manifest
            .map(|manifest| {
                manifest
                    .documents
                    .keys()
                    .filter_map(|path| {
                        let path = normalize_source_path(path).ok()?;
                        Some((path.clone(), input.source_digests.get(&path).cloned()))
                    })
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_default();
        let configuration_digest = canonical_json_bytes(&(&manifest, &bound_sources)).map_or_else(
            |_| hex_sha256(b"invalid-scip-configuration"),
            |bytes| hex_sha256(&bytes),
        );
        ProviderDescriptor {
            id: format!("scip:{}", input.input_digest),
            kind: ProviderKind::Artifact,
            version: format!("scip/{SCIP_PROVIDER_VERSION}"),
            scope: "repository".to_owned(),
            input_digest: input.input_digest.to_owned(),
            configuration_digest,
        }
    }

    fn analyze_artifact(
        &self,
        input: ArtifactInput<'_>,
        reader: &mut dyn ArtifactReader,
    ) -> Result<EvidenceBatch, ProviderError> {
        let decoded = self.decode_artifact(input, reader)?;
        self.normalize_decoded(input, &decoded)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Freshness {
    Verified,
    Unverified,
    Stale,
}

fn freshness(input: &ArtifactInput<'_>, path: &str) -> Result<Freshness, ProviderError> {
    let Some(manifest) = input.manifest else {
        return Ok(Freshness::Unverified);
    };
    let expected = manifest
        .documents
        .iter()
        .find_map(|(manifest_path, digest)| {
            normalize_source_path(manifest_path)
                .ok()
                .filter(|manifest_path| manifest_path == path)
                .map(|_| digest)
        });
    let Some(expected) = expected else {
        return Ok(Freshness::Unverified);
    };
    Ok(match input.source_digests.get(path) {
        Some(actual) if actual == expected => Freshness::Verified,
        _ => Freshness::Stale,
    })
}

fn normalize_document(
    input: &ArtifactInput<'_>,
    metadata: &Metadata,
    document: &Document,
    path: &str,
    freshness: Freshness,
    batch: &mut EvidenceBatch,
) -> Result<(), ProviderError> {
    let source = match freshness {
        Freshness::Verified => input
            .source_texts
            .get(path)
            .map(Vec::as_slice)
            .or_else(|| (!document.text.is_empty()).then_some(document.text.as_bytes())),
        Freshness::Unverified => (!document.text.is_empty()).then_some(document.text.as_bytes()),
        Freshness::Stale => None,
    };
    let Some(source) = source else {
        let mut coverage = coverage_for_document(freshness);
        add_coverage_reason(
            &mut coverage,
            Capability::References,
            "source_text_unavailable",
        );
        batch.coverage.insert(path.to_owned(), coverage);
        return Ok(());
    };
    let encoding = position_encoding(document, metadata)?;
    let mut definitions = BTreeMap::<String, SourceAnchor>::new();
    for occurrence in &document.occurrences {
        if occurrence.symbol.is_empty() {
            continue;
        }
        let anchor = occurrence_anchor(occurrence, path, source, encoding)?;
        let roles = occurrence_roles(occurrence.symbol_roles);
        if roles.contains(&Role::Definition) {
            definitions
                .entry(occurrence.symbol.clone())
                .or_insert_with(|| anchor.clone());
        }
        add_occurrence_facts(batch, path, occurrence, anchor, roles);
    }
    for symbol in &document.symbols {
        add_relationship_facts(batch, path, symbol, &definitions);
    }
    batch
        .coverage
        .insert(path.to_owned(), coverage_for_document(freshness));
    Ok(())
}

fn add_occurrence_facts(
    batch: &mut EvidenceBatch,
    path: &str,
    occurrence: &Occurrence,
    anchor: SourceAnchor,
    roles: Vec<Role>,
) {
    let capability = if roles.contains(&Role::Definition) {
        Capability::Definitions
    } else {
        Capability::References
    };
    let record = evidence_record(
        &batch.descriptor.id,
        Some(path),
        capability.clone(),
        format!(
            "SCIP symbol {} roles {}",
            occurrence.symbol,
            role_names(&roles)
        ),
        Some(&anchor),
        "symbol",
        &occurrence.symbol,
    );
    batch.evidence.push(record.clone());
    batch.facts.push(EvidenceFact {
        evidence_id: record.id,
        capability,
        anchor: anchor.clone(),
        kind: FactKind::Symbol {
            symbol: occurrence.symbol.clone(),
            roles: roles.clone(),
        },
    });
    if !roles.contains(&Role::Definition) {
        let resolution = evidence_record(
            &batch.descriptor.id,
            Some(path),
            Capability::CallResolution,
            format!("SCIP reference target {}", occurrence.symbol),
            Some(&anchor),
            "call_resolution",
            &occurrence.symbol,
        );
        batch.evidence.push(resolution.clone());
        batch.facts.push(EvidenceFact {
            evidence_id: resolution.id,
            capability: Capability::CallResolution,
            anchor,
            kind: FactKind::CallResolution {
                target: occurrence.symbol.clone(),
            },
        });
    }
}

fn add_relationship_facts(
    batch: &mut EvidenceBatch,
    path: &str,
    symbol: &SymbolInformation,
    definitions: &BTreeMap<String, SourceAnchor>,
) {
    let Some(anchor) = definitions.get(&symbol.symbol) else {
        return;
    };
    for relationship in &symbol.relationships {
        let roles = relationship_roles(relationship);
        if roles.is_empty() || relationship.symbol.is_empty() {
            continue;
        }
        let capability = if roles.contains(&Role::TypeDefinition) {
            Capability::Types
        } else if roles.contains(&Role::Implementation) {
            Capability::Definitions
        } else {
            Capability::References
        };
        let payload = format!(
            "{}\0{}\0{}",
            symbol.symbol,
            relationship.symbol,
            role_names(&roles)
        );
        let record = evidence_record(
            &batch.descriptor.id,
            Some(path),
            capability.clone(),
            format!(
                "SCIP relationship {} -> {} ({})",
                symbol.symbol,
                relationship.symbol,
                role_names(&roles)
            ),
            Some(anchor),
            "relationship",
            &payload,
        );
        batch.evidence.push(record.clone());
        batch.facts.push(EvidenceFact {
            evidence_id: record.id,
            capability,
            anchor: anchor.clone(),
            kind: FactKind::Relationship {
                source: symbol.symbol.clone(),
                target: relationship.symbol.clone(),
                roles,
            },
        });
    }
}

fn add_tool_evidence(metadata: &Metadata, batch: &mut EvidenceBatch) {
    let Some(tool) = metadata.tool_info.as_ref() else {
        return;
    };
    let detail = format!("SCIP producer {} {}", tool.name, tool.version);
    let record = evidence_record(
        &batch.descriptor.id,
        None,
        Capability::SymbolIdentity,
        detail,
        None,
        "tool",
        &format!("{}\0{}", tool.name, tool.version),
    );
    batch.evidence.push(record);
}

fn validate_metadata(metadata: &Metadata) -> Result<(), ProviderError> {
    match metadata.text_document_encoding.enum_value() {
        Ok(TextEncoding::UTF8 | TextEncoding::UTF16 | TextEncoding::UnspecifiedTextEncoding) => {
            Ok(())
        }
        Err(value) => Err(ProviderError::UnsupportedArtifact(format!(
            "unknown SCIP text encoding {value}"
        ))),
    }
}

fn position_encoding(
    document: &Document,
    metadata: &Metadata,
) -> Result<PositionEncoding, ProviderError> {
    match document.position_encoding.enum_value() {
        Ok(PositionEncoding::UTF8CodeUnitOffsetFromLineStart) => {
            Ok(PositionEncoding::UTF8CodeUnitOffsetFromLineStart)
        }
        Ok(PositionEncoding::UTF16CodeUnitOffsetFromLineStart) => {
            Ok(PositionEncoding::UTF16CodeUnitOffsetFromLineStart)
        }
        Ok(PositionEncoding::UTF32CodeUnitOffsetFromLineStart) => {
            Ok(PositionEncoding::UTF32CodeUnitOffsetFromLineStart)
        }
        Ok(PositionEncoding::UnspecifiedPositionEncoding) => {
            match metadata.text_document_encoding.enum_value() {
                Ok(TextEncoding::UTF16) => Ok(PositionEncoding::UTF16CodeUnitOffsetFromLineStart),
                Ok(TextEncoding::UTF8 | TextEncoding::UnspecifiedTextEncoding) => {
                    Ok(PositionEncoding::UTF8CodeUnitOffsetFromLineStart)
                }
                Err(value) => Err(ProviderError::UnsupportedArtifact(format!(
                    "unknown SCIP text encoding {value}"
                ))),
            }
        }
        Err(value) => Err(ProviderError::UnsupportedArtifact(format!(
            "unknown SCIP position encoding {value}"
        ))),
    }
}

fn occurrence_anchor(
    occurrence: &Occurrence,
    path: &str,
    source: &[u8],
    encoding: PositionEncoding,
) -> Result<SourceAnchor, ProviderError> {
    let (start_line, start_character, end_line, end_character) =
        match occurrence.typed_range.as_ref() {
            Some(occurrence::Typed_range::SingleLineRange(range)) => (
                range.line,
                range.start_character,
                range.line,
                range.end_character,
            ),
            Some(occurrence::Typed_range::MultiLineRange(range)) => (
                range.start_line,
                range.start_character,
                range.end_line,
                range.end_character,
            ),
            Some(_) => {
                return Err(ProviderError::UnsupportedArtifact(
                    "unsupported SCIP typed range".to_owned(),
                ));
            }
            None => match occurrence.range.as_slice() {
                [line, start, end] => (*line, *start, *line, *end),
                [start_line, start, end_line, end] => (*start_line, *start, *end_line, *end),
                _ => {
                    return Err(ProviderError::MalformedArtifact(format!(
                        "invalid SCIP occurrence range in {path}"
                    )));
                }
            },
        };
    let start = position_to_byte(source, start_line, start_character, encoding)?;
    let end = position_to_byte(source, end_line, end_character, encoding)?;
    if start > end {
        return Err(ProviderError::MalformedArtifact(format!(
            "reversed SCIP occurrence range in {path}"
        )));
    }
    Ok(SourceAnchor {
        source_file: path.to_owned(),
        start_byte: start,
        end_byte: end,
    })
}

fn position_to_byte(
    source: &[u8],
    line: i32,
    character: i32,
    encoding: PositionEncoding,
) -> Result<u64, ProviderError> {
    let line = usize::try_from(line)
        .map_err(|_| ProviderError::MalformedArtifact("negative SCIP line".to_owned()))?;
    let character = usize::try_from(character)
        .map_err(|_| ProviderError::MalformedArtifact("negative SCIP character".to_owned()))?;
    let (line_start, line_bytes) = source_line(source, line)?;
    let offset = match encoding {
        PositionEncoding::UTF8CodeUnitOffsetFromLineStart => {
            if character > line_bytes.len()
                || std::str::from_utf8(line_bytes)
                    .ok()
                    .is_none_or(|line| !line.is_char_boundary(character))
            {
                return Err(ProviderError::MalformedArtifact(
                    "SCIP UTF-8 character is not a valid boundary".to_owned(),
                ));
            }
            character
        }
        PositionEncoding::UTF16CodeUnitOffsetFromLineStart => {
            encoded_character_offset(line_bytes, character, true)?
        }
        PositionEncoding::UTF32CodeUnitOffsetFromLineStart => {
            encoded_character_offset(line_bytes, character, false)?
        }
        PositionEncoding::UnspecifiedPositionEncoding => {
            return Err(ProviderError::UnsupportedArtifact(
                "unspecified SCIP position encoding".to_owned(),
            ));
        }
    };
    u64::try_from(line_start.saturating_add(offset))
        .map_err(|_| ProviderError::ResourceLimit("source position overflow".to_owned()))
}

fn source_line(source: &[u8], requested: usize) -> Result<(usize, &[u8]), ProviderError> {
    let mut start = 0;
    for line in 0..=requested {
        let end = source[start..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(source.len(), |relative| start + relative);
        if line == requested {
            let content_end = end.saturating_sub(usize::from(
                end > start && source.get(end - 1) == Some(&b'\r'),
            ));
            return Ok((start, &source[start..content_end]));
        }
        if end == source.len() {
            break;
        }
        start = end + 1;
    }
    Err(ProviderError::MalformedArtifact(format!(
        "SCIP line {requested} is outside source"
    )))
}

fn encoded_character_offset(
    line: &[u8],
    requested: usize,
    utf16: bool,
) -> Result<usize, ProviderError> {
    let text = std::str::from_utf8(line)
        .map_err(|_| ProviderError::MalformedArtifact("source is not valid UTF-8".to_owned()))?;
    let mut units = 0;
    for (byte, character) in text.char_indices() {
        if units == requested {
            return Ok(byte);
        }
        units += if utf16 { character.len_utf16() } else { 1 };
        if units > requested {
            return Err(ProviderError::MalformedArtifact(
                "SCIP character splits an encoded code point".to_owned(),
            ));
        }
    }
    if units == requested {
        Ok(line.len())
    } else {
        Err(ProviderError::MalformedArtifact(
            "SCIP character is outside source line".to_owned(),
        ))
    }
}

fn occurrence_roles(bits: i32) -> Vec<Role> {
    let mut roles = Vec::new();
    for (value, role) in [
        (SymbolRole::Definition as i32, Role::Definition),
        (SymbolRole::Import as i32, Role::Import),
        (SymbolRole::WriteAccess as i32, Role::Write),
        (SymbolRole::ReadAccess as i32, Role::Read),
    ] {
        if bits & value != 0 {
            roles.push(role);
        }
    }
    if roles.is_empty() {
        roles.push(Role::Reference);
    }
    roles
}

fn relationship_roles(relationship: &Relationship) -> Vec<Role> {
    let mut roles = Vec::new();
    if relationship.is_reference {
        roles.push(Role::Reference);
    }
    if relationship.is_implementation {
        roles.push(Role::Implementation);
    }
    if relationship.is_type_definition {
        roles.push(Role::TypeDefinition);
    }
    if relationship.is_definition {
        roles.push(Role::Definition);
    }
    roles
}

fn role_names(roles: &[Role]) -> String {
    roles
        .iter()
        .map(|role| format!("{role:?}").to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join(",")
}

fn coverage_for_document(freshness: Freshness) -> Coverage {
    let freshness_reason = match freshness {
        Freshness::Verified => None,
        Freshness::Unverified => Some("artifact_revision_unverified"),
        Freshness::Stale => Some("stale_artifact_document"),
    };
    let mut coverage = Coverage::new();
    for capability in [
        Capability::SymbolIdentity,
        Capability::Definitions,
        Capability::References,
        Capability::CallResolution,
    ] {
        let mut reasons = vec!["scip_index_scope".to_owned()];
        if let Some(reason) = freshness_reason {
            reasons.push(reason.to_owned());
        }
        coverage.insert(capability, CoverageState::Partial { reasons });
    }
    let mut type_reasons = vec!["scip_type_definitions_only".to_owned()];
    if let Some(reason) = freshness_reason {
        type_reasons.push(reason.to_owned());
    }
    coverage.insert(
        Capability::Types,
        CoverageState::Partial {
            reasons: type_reasons,
        },
    );
    coverage
}

fn add_coverage_reason(coverage: &mut Coverage, capability: Capability, reason: &str) {
    coverage
        .entry(capability)
        .and_modify(|state| match state {
            CoverageState::Complete => {
                *state = CoverageState::Partial {
                    reasons: vec![reason.to_owned()],
                };
            }
            CoverageState::Partial { reasons }
            | CoverageState::Indeterminate { reasons }
            | CoverageState::Failed { reasons }
            | CoverageState::Unavailable { reasons } => {
                reasons.push(reason.to_owned());
            }
        })
        .or_insert_with(|| CoverageState::Partial {
            reasons: vec![reason.to_owned()],
        });
}

fn protobuf_error(error: protobuf::Error) -> ProviderError {
    ProviderError::MalformedArtifact(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::{PositionEncoding, position_to_byte};

    #[test]
    fn converts_utf8_utf16_and_utf32_positions() -> Result<(), crate::ProviderError> {
        let source = "a😀z\n".as_bytes();
        assert_eq!(
            position_to_byte(
                source,
                0,
                5,
                PositionEncoding::UTF8CodeUnitOffsetFromLineStart
            )?,
            5
        );
        assert_eq!(
            position_to_byte(
                source,
                0,
                3,
                PositionEncoding::UTF16CodeUnitOffsetFromLineStart
            )?,
            5
        );
        assert_eq!(
            position_to_byte(
                source,
                0,
                2,
                PositionEncoding::UTF32CodeUnitOffsetFromLineStart
            )?,
            5
        );
        Ok(())
    }
}
