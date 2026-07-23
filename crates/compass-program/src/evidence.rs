use std::collections::BTreeMap;

use compass_ir::{
    Capability, Coverage, CoverageState, EvidenceRecord, ModuleIr, ProviderDescriptor,
    SourceAnchor, SymbolId, canonical_json_bytes, hex_sha256,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Definition,
    Reference,
    Import,
    Read,
    Write,
    Implementation,
    TypeDefinition,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FactKind {
    Symbol {
        symbol: SymbolId,
        roles: Vec<Role>,
    },
    CallResolution {
        target: SymbolId,
    },
    TypeResolution {
        spelling: String,
        target: SymbolId,
    },
    Relationship {
        source: SymbolId,
        target: SymbolId,
        roles: Vec<Role>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EvidenceFact {
    pub evidence_id: String,
    pub capability: Capability,
    pub anchor: SourceAnchor,
    pub kind: FactKind,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EvidenceBatch {
    pub descriptor: ProviderDescriptor,
    #[serde(default)]
    pub evidence: Vec<EvidenceRecord>,
    #[serde(default)]
    pub modules: Vec<ModuleIr>,
    #[serde(default)]
    pub facts: Vec<EvidenceFact>,
    #[serde(default)]
    pub coverage: BTreeMap<String, Coverage>,
}

impl EvidenceBatch {
    pub fn canonicalized(&self) -> Self {
        let mut batch = self.clone();
        batch.evidence.sort();
        batch.evidence.dedup();
        batch.modules.sort_by(|left, right| {
            left.source_file
                .as_bytes()
                .cmp(right.source_file.as_bytes())
        });
        batch.facts.sort_by(|left, right| {
            left.anchor
                .cmp(&right.anchor)
                .then_with(|| left.evidence_id.as_bytes().cmp(right.evidence_id.as_bytes()))
        });
        batch.facts.dedup();
        for coverage in batch.coverage.values_mut() {
            for state in coverage.values_mut() {
                match state {
                    CoverageState::Complete => {}
                    CoverageState::Partial { reasons }
                    | CoverageState::Unavailable { reasons } => {
                        reasons.sort();
                        reasons.dedup();
                    }
                }
            }
        }
        batch
    }

    pub fn digest(&self) -> Result<String, compass_ir::IrError> {
        Ok(hex_sha256(&canonical_json_bytes(&self.canonicalized())?))
    }
}

pub fn coverage_with(items: impl IntoIterator<Item = (Capability, CoverageState)>) -> Coverage {
    items.into_iter().collect()
}

pub fn evidence_id(
    provider_id: &str,
    capability: &Capability,
    source_file: Option<&str>,
    anchor: Option<&SourceAnchor>,
    fact_kind: &str,
    fact_payload: &str,
) -> String {
    let value = (
        provider_id,
        capability,
        source_file.unwrap_or_default(),
        anchor,
        fact_kind,
        fact_payload,
    );
    canonical_json_bytes(&value)
        .map_or_else(|_| hex_sha256(b"invalid-evidence"), |bytes| hex_sha256(&bytes))
}

pub fn evidence_record(
    provider_id: &str,
    source_file: Option<&str>,
    capability: Capability,
    detail: impl Into<String>,
    anchor: Option<&SourceAnchor>,
    fact_kind: &str,
    fact_payload: &str,
) -> EvidenceRecord {
    let detail = detail.into();
    EvidenceRecord {
        id: evidence_id(
            provider_id,
            &capability,
            source_file,
            anchor,
            fact_kind,
            fact_payload,
        ),
        provider_id: provider_id.to_owned(),
        source_file: source_file.map(str::to_owned),
        capability,
        detail,
    }
}
