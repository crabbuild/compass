use std::io::{SeekFrom, Write};

use protobuf::{CodedInputStream, Message};
use scip::types::{Document, Metadata};
use sha2::{Digest, Sha256};

use crate::{ArtifactLimits, ArtifactReader, ProviderError};

pub(crate) fn verify_reader(
    reader: &mut dyn ArtifactReader,
    expected_len: u64,
    expected_digest: &str,
    limits: ArtifactLimits,
) -> Result<(), ProviderError> {
    let actual_len = reader.seek(SeekFrom::End(0))?;
    if actual_len != expected_len {
        return Err(ProviderError::InvalidInput(format!(
            "artifact length changed: expected {expected_len}, got {actual_len}"
        )));
    }
    if actual_len > limits.max_artifact_bytes {
        return Err(ProviderError::ResourceLimit(format!(
            "artifact is {actual_len} bytes; maximum is {}",
            limits.max_artifact_bytes
        )));
    }
    reader.seek(SeekFrom::Start(0))?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut remaining = actual_len;
    while remaining != 0 {
        let limit = usize::try_from(remaining.min(buffer.len() as u64))
            .map_err(|_| ProviderError::ResourceLimit("artifact length overflow".to_owned()))?;
        let count = reader.read(&mut buffer[..limit])?;
        if count == 0 {
            return Err(ProviderError::MalformedArtifact(
                "artifact ended during digest verification".to_owned(),
            ));
        }
        digest
            .write_all(&buffer[..count])
            .map_err(ProviderError::Io)?;
        remaining = remaining.saturating_sub(count as u64);
    }
    let actual_digest = hex_digest(digest.finalize().as_slice());
    if actual_digest != expected_digest {
        return Err(ProviderError::InvalidInput(
            "artifact content digest mismatch".to_owned(),
        ));
    }
    reader.seek(SeekFrom::Start(0))?;
    Ok(())
}

pub(crate) fn read_metadata(
    reader: &mut dyn ArtifactReader,
    limits: ArtifactLimits,
) -> Result<Metadata, ProviderError> {
    reader.seek(SeekFrom::Start(0))?;
    let mut input = CodedInputStream::new(reader);
    let mut metadata = None;
    while let Some(tag) = input.read_raw_tag_or_eof().map_err(protobuf_error)? {
        match tag {
            10 => {
                let bytes =
                    read_message_bytes(&mut input, limits.max_metadata_bytes, "SCIP metadata")?;
                if metadata.is_some() {
                    return Err(ProviderError::MalformedArtifact(
                        "SCIP index has duplicate metadata".to_owned(),
                    ));
                }
                metadata = Some(Metadata::parse_from_bytes(&bytes).map_err(protobuf_error)?);
            }
            18 | 26 => skip_length_delimited(&mut input, limits.max_document_bytes)?,
            _ => skip_unknown(tag, &mut input)?,
        }
    }
    metadata.ok_or_else(|| ProviderError::MalformedArtifact("SCIP metadata is missing".to_owned()))
}

pub(crate) fn visit_documents(
    reader: &mut dyn ArtifactReader,
    limits: ArtifactLimits,
    mut visit: impl FnMut(Document) -> Result<(), ProviderError>,
) -> Result<(), ProviderError> {
    reader.seek(SeekFrom::Start(0))?;
    let mut input = CodedInputStream::new(reader);
    let mut records = 0_u64;
    while let Some(tag) = input.read_raw_tag_or_eof().map_err(protobuf_error)? {
        match tag {
            18 => {
                let bytes =
                    read_message_bytes(&mut input, limits.max_document_bytes, "SCIP document")?;
                let document = Document::parse_from_bytes(&bytes).map_err(protobuf_error)?;
                let document_records = u64::try_from(document.occurrences.len())
                    .unwrap_or(u64::MAX)
                    .saturating_add(u64::try_from(document.symbols.len()).unwrap_or(u64::MAX));
                records = records.checked_add(document_records).ok_or_else(|| {
                    ProviderError::ResourceLimit("SCIP record count overflow".to_owned())
                })?;
                if records > limits.max_records {
                    return Err(ProviderError::ResourceLimit(format!(
                        "SCIP record count exceeds {}",
                        limits.max_records
                    )));
                }
                visit(document)?;
            }
            10 => skip_length_delimited(&mut input, limits.max_metadata_bytes)?,
            26 => {
                records = records.checked_add(1).ok_or_else(|| {
                    ProviderError::ResourceLimit("SCIP record count overflow".to_owned())
                })?;
                if records > limits.max_records {
                    return Err(ProviderError::ResourceLimit(format!(
                        "SCIP record count exceeds {}",
                        limits.max_records
                    )));
                }
                skip_length_delimited(&mut input, limits.max_document_bytes)?;
            }
            _ => skip_unknown(tag, &mut input)?,
        }
    }
    Ok(())
}

fn read_message_bytes(
    input: &mut CodedInputStream<'_>,
    max_bytes: u64,
    label: &str,
) -> Result<Vec<u8>, ProviderError> {
    let len = input.read_raw_varint32().map_err(protobuf_error)?;
    if u64::from(len) > max_bytes {
        return Err(ProviderError::ResourceLimit(format!(
            "{label} is {len} bytes; maximum is {max_bytes}"
        )));
    }
    input.read_raw_bytes(len).map_err(protobuf_error)
}

fn skip_length_delimited(
    input: &mut CodedInputStream<'_>,
    max_bytes: u64,
) -> Result<(), ProviderError> {
    let len = input.read_raw_varint32().map_err(protobuf_error)?;
    if u64::from(len) > max_bytes {
        return Err(ProviderError::ResourceLimit(format!(
            "SCIP message is {len} bytes; maximum is {max_bytes}"
        )));
    }
    input.skip_raw_bytes(len).map_err(protobuf_error)
}

fn skip_unknown(tag: u32, input: &mut CodedInputStream<'_>) -> Result<(), ProviderError> {
    match tag & 7 {
        0 => {
            input.read_raw_varint64().map_err(protobuf_error)?;
        }
        1 => input.skip_raw_bytes(8).map_err(protobuf_error)?,
        2 => {
            let len = input.read_raw_varint32().map_err(protobuf_error)?;
            input.skip_raw_bytes(len).map_err(protobuf_error)?;
        }
        5 => input.skip_raw_bytes(4).map_err(protobuf_error)?,
        wire => {
            return Err(ProviderError::MalformedArtifact(format!(
                "unsupported protobuf wire type {wire}"
            )));
        }
    }
    Ok(())
}

fn protobuf_error(error: protobuf::Error) -> ProviderError {
    ProviderError::MalformedArtifact(error.to_string())
}

fn hex_digest(bytes: &[u8]) -> String {
    use std::fmt::Write;

    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(output, "{byte:02x}");
    }
    output
}
