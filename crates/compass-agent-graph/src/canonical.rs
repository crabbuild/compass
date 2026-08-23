use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use serde_json::{Map, Value};
use sha2::{Digest as _, Sha256};

use crate::{AgentGraphError, AgentGraphErrorCode};

/// A strict lowercase SHA-256 digest used by all agent-graph identities.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Digest(String);

impl Digest {
    pub fn parse(value: impl Into<String>) -> Result<Self, AgentGraphError> {
        let value = value.into();
        if value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::InvalidIdentifier,
                "digest must be exactly 64 lowercase hexadecimal characters",
            ));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn of_bytes(domain: &str, value: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(domain.as_bytes());
        hasher.update([0]);
        hasher.update(value);
        Self(format!("{:x}", hasher.finalize()))
    }

    #[must_use]
    pub fn raw_bytes(value: &[u8]) -> Self {
        Self(format!("{:x}", Sha256::digest(value)))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Serialize for Digest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Digest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(de::Error::custom)
    }
}

/// Serialize with recursively sorted object keys and no insignificant whitespace.
pub fn canonical_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, AgentGraphError> {
    let value = serde_json::to_value(value).map_err(|error| {
        AgentGraphError::new(
            AgentGraphErrorCode::InvalidInput,
            format!("value cannot be represented as canonical JSON: {error}"),
        )
    })?;
    let normalized = canonicalize(value);
    serde_json::to_vec(&normalized).map_err(|error| {
        AgentGraphError::new(
            AgentGraphErrorCode::InvalidInput,
            format!("canonical JSON encoding failed: {error}"),
        )
    })
}

pub fn canonical_digest<T: Serialize>(domain: &str, value: &T) -> Result<Digest, AgentGraphError> {
    Ok(Digest::of_bytes(domain, &canonical_bytes(value)?))
}

fn canonicalize(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(canonicalize).collect()),
        Value::Object(values) => {
            let mut keys = values.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            let mut ordered = Map::new();
            for key in keys {
                if let Some(value) = values.get(&key) {
                    ordered.insert(key, canonicalize(value.clone()));
                }
            }
            Value::Object(ordered)
        }
        other => other,
    }
}
