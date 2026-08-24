use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RawFrameworkOrigin {
    Ast,
    Config,
    Convention,
    Heuristic,
}

impl RawFrameworkOrigin {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ast => "ast",
            Self::Config => "config",
            Self::Convention => "convention",
            Self::Heuristic => "heuristic",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RawFrameworkAnchor {
    pub source_file: String,
    pub start_byte: u64,
    pub end_byte: u64,
    pub start_line: u32,
    pub start_column: u32,
    pub end_line: u32,
    pub end_column: u32,
}

impl RawFrameworkAnchor {
    #[must_use]
    pub fn is_valid(&self) -> bool {
        !self.source_file.trim().is_empty()
            && self.start_byte < self.end_byte
            && self.start_line > 0
            && self.end_line > 0
            && (self.start_line < self.end_line
                || (self.start_line == self.end_line && self.start_column <= self.end_column))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RawRouteFact {
    pub framework: String,
    pub operation: String,
    pub raw_path: String,
    pub normalized_path: String,
    pub declaring_scope: String,
    pub anchor: RawFrameworkAnchor,
    pub handler_reference: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub middleware_references: Vec<String>,
    pub origin: RawFrameworkOrigin,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub detail: Map<String, Value>,
}

impl RawRouteFact {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.framework.trim().is_empty() {
            return Err("framework must not be empty");
        }
        if self.operation.trim().is_empty() {
            return Err("operation must not be empty");
        }
        if self.normalized_path.trim().is_empty() {
            return Err("normalized route path must not be empty");
        }
        if self.declaring_scope.trim().is_empty() {
            return Err("declaring scope must not be empty");
        }
        if self.handler_reference.trim().is_empty() {
            return Err("handler reference must not be empty");
        }
        if !self.anchor.is_valid() {
            return Err("route anchor must be a non-empty valid range");
        }
        if matches!(
            self.origin,
            RawFrameworkOrigin::Convention | RawFrameworkOrigin::Heuristic
        ) && self.rule.as_deref().is_none_or(str::is_empty)
        {
            return Err("convention and heuristic routes require a rule");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RawDomainFact {
    pub framework: String,
    pub kind: String,
    pub name: String,
    pub declaring_scope: String,
    pub anchor: RawFrameworkAnchor,
    pub origin: RawFrameworkOrigin,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub detail: Map<String, Value>,
}

/// Exact universal-language annotation evidence consumed by a framework pack.
///
/// The pack records the language fact without deciding project-wide framework
/// meaning. Collection resolution can then correlate composed annotations,
/// interface declarations, inheritance, and same-named types without falling
/// back to terminal labels.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RawFrameworkAnnotationFact {
    pub pack_id: String,
    pub framework: String,
    pub annotation_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotation_qualified_name: Option<String>,
    pub owner_declaration_id: String,
    pub owner_graph_node_id: String,
    pub owner_qualified_name: String,
    pub owner_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_signature: Option<String>,
    pub anchor: RawFrameworkAnchor,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub arguments: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub detail: Map<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "fact", rename_all = "snake_case")]
pub enum RawFrameworkFact {
    Route(RawRouteFact),
    Domain(RawDomainFact),
    Annotation(RawFrameworkAnnotationFact),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameworkLimits {
    pub max_candidates: usize,
    pub max_include_depth: usize,
    pub max_alias_expansions: usize,
    pub max_facts_per_file: usize,
}

impl Default for FrameworkLimits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl FrameworkLimits {
    /// The conservative default budget shared by established framework packs.
    ///
    /// Keeping the value as a named constant lets the pack runtime apply the
    /// same budget without constructing a second, subtly different policy.
    pub const DEFAULT: Self = Self {
        max_candidates: 20,
        max_include_depth: 32,
        max_alias_expansions: 1_000,
        max_facts_per_file: 100_000,
    };

    pub fn check_facts(self, count: usize) -> Result<(), FrameworkLimitError> {
        if count > self.max_facts_per_file {
            return Err(FrameworkLimitError {
                limit: "max_facts_per_file",
                maximum: self.max_facts_per_file,
                observed: count,
            });
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameworkLimitError {
    pub limit: &'static str,
    pub maximum: usize,
    pub observed: usize,
}

impl std::fmt::Display for FrameworkLimitError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "framework limit {} exceeded: observed {}, maximum {}",
            self.limit, self.observed, self.maximum
        )
    }
}

impl std::error::Error for FrameworkLimitError {}
