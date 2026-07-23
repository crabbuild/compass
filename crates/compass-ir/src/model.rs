use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub type EvidenceId = String;
pub type SymbolId = String;
pub type Coverage = BTreeMap<Capability, CoverageState>;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Syntax,
    Artifact,
    Project,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Syntax,
    SymbolIdentity,
    Definitions,
    References,
    Types,
    CallResolution,
    ControlFlow,
    DataFlow,
    Effects,
    Contracts,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum CoverageState {
    Complete,
    Partial { reasons: Vec<String> },
    Unavailable { reasons: Vec<String> },
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct ProviderDescriptor {
    pub id: String,
    pub kind: ProviderKind,
    pub version: String,
    pub scope: String,
    pub input_digest: String,
    pub configuration_digest: String,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct EvidenceRecord {
    pub id: EvidenceId,
    pub provider_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_file: Option<String>,
    pub capability: Capability,
    pub detail: String,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct SourceAnchor {
    pub source_file: String,
    pub start_byte: u64,
    pub end_byte: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProgramBundle {
    pub schema: String,
    pub providers: Vec<ProviderDescriptor>,
    pub evidence: Vec<EvidenceRecord>,
    pub modules: Vec<ModuleIr>,
}

impl Default for ProgramBundle {
    fn default() -> Self {
        Self {
            schema: crate::PROGRAM_SCHEMA.to_owned(),
            providers: Vec::new(),
            evidence: Vec::new(),
            modules: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ModuleIr {
    pub source_file: String,
    pub language: String,
    pub source_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_node_id: Option<String>,
    pub functions: Vec<FunctionIr>,
    #[serde(default)]
    pub coverage: Coverage,
    #[serde(default)]
    pub evidence: Vec<EvidenceId>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FunctionIr {
    pub symbol_id: SymbolId,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_node_id: Option<String>,
    pub signature_digest: String,
    pub body_digest: String,
    pub anchor: SourceAnchor,
    #[serde(default)]
    pub parameters: Vec<ParameterIr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub return_type: Option<TypeRef>,
    pub blocks: Vec<BasicBlock>,
    #[serde(default)]
    pub coverage: Coverage,
    #[serde(default)]
    pub evidence: Vec<EvidenceId>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ParameterIr {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_ref: Option<TypeRef>,
    pub anchor: SourceAnchor,
    #[serde(default)]
    pub evidence: Vec<EvidenceId>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TypeRef {
    pub spelling: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_symbol: Option<SymbolId>,
    #[serde(default)]
    pub evidence: Vec<EvidenceId>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BasicBlock {
    pub id: u32,
    pub operations: Vec<Operation>,
    pub terminator: Terminator,
    #[serde(default)]
    pub evidence: Vec<EvidenceId>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Operation {
    pub ordinal: u32,
    pub anchor: SourceAnchor,
    #[serde(default)]
    pub evidence: Vec<EvidenceId>,
    pub kind: OperationKind,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OperationKind {
    Call {
        callee: String,
        callee_anchor: SourceAnchor,
        #[serde(default)]
        resolved_symbols: Vec<SymbolId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        receiver_type: Option<TypeRef>,
    },
    Read {
        path: String,
    },
    Write {
        path: String,
    },
    Await,
    Throw {
        value: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Terminator {
    Return {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<String>,
    },
    Goto {
        target: u32,
    },
    Branch {
        condition: String,
        then_target: u32,
        else_target: u32,
    },
    Unreachable,
}
