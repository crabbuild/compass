use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{EvidenceManifest, PrIntelligenceError, PullRequestReport};

pub fn canonical_json_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, PrIntelligenceError> {
    let mut value = serde_json::to_value(value)?;
    sort_value(&mut value);
    Ok(serde_json::to_vec(&value)?)
}

pub fn report_digest(report: &PullRequestReport) -> Result<String, PrIntelligenceError> {
    let mut value = serde_json::to_value(report)?;
    let object = value.as_object_mut().ok_or_else(|| {
        PrIntelligenceError::InvalidEvidence("report must encode as an object".to_owned())
    })?;
    object.remove("report_digest");
    sort_value(&mut value);
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(&value)?)
    ))
}

pub fn evidence_manifest_digest(
    manifest: &EvidenceManifest,
) -> Result<String, PrIntelligenceError> {
    let mut value = serde_json::to_value(manifest)?;
    let object = value.as_object_mut().ok_or_else(|| {
        PrIntelligenceError::InvalidEvidence("manifest must encode as an object".to_owned())
    })?;
    object.remove("digest");
    sort_value(&mut value);
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(&value)?)
    ))
}

fn sort_value(value: &mut Value) {
    match value {
        Value::Object(object) => {
            let old = std::mem::take(object);
            let mut entries = old.into_iter().collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
            for (key, mut child) in entries {
                sort_value(&mut child);
                object.insert(key, child);
            }
        }
        Value::Array(values) => {
            for child in values {
                sort_value(child);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}
