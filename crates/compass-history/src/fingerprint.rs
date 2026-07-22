use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};

use crate::{HistoryError, canonical_json_bytes};

/// SHA-256 identity for every meaning-affecting extraction input.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ExtractionFingerprint([u8; 32]);

impl ExtractionFingerprint {
    /// Return strict lowercase hexadecimal text.
    #[must_use]
    pub fn as_hex(&self) -> String {
        hex(&self.0)
    }
}

impl fmt::Display for ExtractionFingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.as_hex())
    }
}

impl FromStr for ExtractionFingerprint {
    type Err = HistoryError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        parse_digest(value).map(Self)
    }
}

impl Serialize for ExtractionFingerprint {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.as_hex())
    }
}

impl<'de> Deserialize<'de> for ExtractionFingerprint {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

/// Normalized, non-secret options available before an exact checkout exists.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct BuildProfile {
    values: BTreeMap<String, String>,
}

impl BuildProfile {
    /// Insert one normalized non-secret option.
    pub fn insert(&mut self, key: &str, value: &str) -> Result<(), HistoryError> {
        let key = checked_key(key)?;
        self.values.insert(key, value.to_owned());
        Ok(())
    }

    /// Return canonical profile bytes.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, HistoryError> {
        canonical_map_bytes(&self.values)
    }

    /// Return the profile's binary SHA-256 digest.
    pub fn digest(&self) -> Result<[u8; 32], HistoryError> {
        Ok(Sha256::digest(self.canonical_bytes()?).into())
    }
}

/// Inputs resolved from a build profile and the exact target commit.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ExtractionFingerprintInput {
    values: BTreeMap<String, String>,
}

impl ExtractionFingerprintInput {
    /// Start an input set with the mandatory binary and graph-schema versions.
    #[must_use]
    pub fn new(compass_version: &str, graph_schema: &str) -> Self {
        let mut values = BTreeMap::new();
        values.insert("compass_version".to_owned(), compass_version.to_owned());
        values.insert("graph_schema".to_owned(), graph_schema.to_owned());
        Self { values }
    }

    /// Insert one meaning-affecting, non-secret input.
    pub fn insert(&mut self, key: &str, value: &str) -> Result<(), HistoryError> {
        let key = checked_key(key)?;
        self.values.insert(key, value.to_owned());
        Ok(())
    }

    /// Return canonical bytes used as the digest input.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, HistoryError> {
        canonical_map_bytes(&self.values)
    }

    /// Compute the extraction fingerprint.
    pub fn digest(&self) -> Result<ExtractionFingerprint, HistoryError> {
        Ok(ExtractionFingerprint(
            Sha256::digest(self.canonical_bytes()?).into(),
        ))
    }
}

fn canonical_map_bytes(values: &BTreeMap<String, String>) -> Result<Vec<u8>, HistoryError> {
    let value =
        serde_json::to_value(values).map_err(|error| HistoryError::Canonical(error.to_string()))?;
    canonical_json_bytes(&value)
}

fn checked_key(key: &str) -> Result<String, HistoryError> {
    let normalized = key.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return Err(HistoryError::InvalidFingerprint(
            "field name cannot be empty".to_owned(),
        ));
    }
    if ["key", "secret", "token", "password", "credential"]
        .iter()
        .any(|needle| normalized.contains(needle))
    {
        return Err(HistoryError::FingerprintSecretKey(key.to_owned()));
    }
    Ok(normalized)
}

fn parse_digest(value: &str) -> Result<[u8; 32], HistoryError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(HistoryError::InvalidFingerprint(value.to_owned()));
    }
    let mut bytes = [0u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        bytes[index] = (nibble(pair[0]) << 4) | nibble(pair[1]);
    }
    Ok(bytes)
}

fn nibble(value: u8) -> u8 {
    match value {
        b'0'..=b'9' => value - b'0',
        b'a'..=b'f' => value - b'a' + 10,
        _ => 0,
    }
}

fn hex(bytes: &[u8]) -> String {
    use fmt::Write;

    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(output, "{byte:02x}");
    }
    output
}
