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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stages: Vec<RawRouteStageFact>,
    pub origin: RawFrameworkOrigin,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub detail: Map<String, Value>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RawRouteStageRole {
    Middleware,
    Dependency,
    Security,
    Layout,
    Template,
    Loading,
    Default,
    ErrorBoundary,
    NotFound,
    Boundary,
    Loader,
    Action,
    Handler,
    DataLoader,
    RouteComponent,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RawRouteStageFact {
    pub role: RawRouteStageRole,
    pub position: u32,
    pub reference: String,
    pub anchor: RawFrameworkAnchor,
    pub origin: RawFrameworkOrigin,
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
        if self
            .stages
            .iter()
            .any(|stage| stage.reference.trim().is_empty() || !stage.anchor.is_valid())
        {
            return Err("route stages require non-empty references and valid anchors");
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

/// A project-neutral role assertion. `subject_reference` is only an identity
/// hint from the language evidence; it is never a resolved graph target.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RawFrameworkRoleFact {
    pub pack_id: String,
    pub framework: String,
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_reference: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    pub anchor: RawFrameworkAnchor,
    pub origin: RawFrameworkOrigin,
    pub evidence_class: String,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub detail: Map<String, Value>,
}

/// A project-neutral relation assertion. The target field is deliberately
/// named a hint: project-wide resolution must still prove the target or keep
/// the relation unresolved/ambiguous.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RawFrameworkRelationFact {
    pub pack_id: String,
    pub framework: String,
    pub relation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_reference: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    pub anchor: RawFrameworkAnchor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_anchor: Option<RawFrameworkAnchor>,
    pub origin: RawFrameworkOrigin,
    pub evidence_class: String,
    pub ambiguity_policy: String,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub detail: Map<String, Value>,
}

/// A statically recovered configuration field selected by a framework pack.
/// The value is intentionally opaque JSON and must remain bounded by the pack
/// limits; dynamic or recovery-overlapping values use `complete = false`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RawFrameworkConfigurationFact {
    pub pack_id: String,
    pub framework: String,
    pub config_id: String,
    pub field: String,
    pub anchor: RawFrameworkAnchor,
    pub ordinal: u32,
    pub complete: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    pub origin: RawFrameworkOrigin,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub detail: Map<String, Value>,
}

/// An ordered, unresolved file-set pattern declaration. Matching and target
/// selection remain owned by `compass-files`/`compass-resolve`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RawFrameworkFileSetFact {
    pub pack_id: String,
    pub framework: String,
    pub owner_reference: String,
    pub patterns: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub negative_patterns: Vec<String>,
    pub anchor: RawFrameworkAnchor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_scope: Option<String>,
    pub eager: bool,
    pub lazy: bool,
    pub import_mode: bool,
    pub query_mode: bool,
    pub origin: RawFrameworkOrigin,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub detail: Map<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "fact", rename_all = "snake_case")]
pub enum RawFrameworkFact {
    Route(RawRouteFact),
    Domain(RawDomainFact),
    Annotation(RawFrameworkAnnotationFact),
    Role(RawFrameworkRoleFact),
    Relation(RawFrameworkRelationFact),
    Configuration(RawFrameworkConfigurationFact),
    FileSet(RawFrameworkFileSetFact),
}

impl RawFrameworkFact {
    #[must_use]
    pub fn anchor(&self) -> &RawFrameworkAnchor {
        match self {
            Self::Route(fact) => &fact.anchor,
            Self::Domain(fact) => &fact.anchor,
            Self::Annotation(fact) => &fact.anchor,
            Self::Role(fact) => &fact.anchor,
            Self::Relation(fact) => &fact.anchor,
            Self::Configuration(fact) => &fact.anchor,
            Self::FileSet(fact) => &fact.anchor,
        }
    }

    #[must_use]
    pub fn framework(&self) -> &str {
        match self {
            Self::Route(fact) => &fact.framework,
            Self::Domain(fact) => &fact.framework,
            Self::Annotation(fact) => &fact.framework,
            Self::Role(fact) => &fact.framework,
            Self::Relation(fact) => &fact.framework,
            Self::Configuration(fact) => &fact.framework,
            Self::FileSet(fact) => &fact.framework,
        }
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if !self.anchor().is_valid() {
            return Err("framework fact anchor must be a non-empty valid range");
        }
        match self {
            Self::Route(fact) => fact.validate(),
            Self::Domain(fact) => {
                if fact.framework.trim().is_empty() || fact.kind.trim().is_empty() {
                    Err("domain framework and kind must not be empty")
                } else {
                    Ok(())
                }
            }
            Self::Annotation(fact) => {
                if fact.pack_id.trim().is_empty()
                    || fact.framework.trim().is_empty()
                    || fact.annotation_name.trim().is_empty()
                    || fact.owner_declaration_id.trim().is_empty()
                {
                    Err("annotation framework identity must not be empty")
                } else {
                    Ok(())
                }
            }
            Self::Role(fact) => fact.validate(),
            Self::Relation(fact) => fact.validate(),
            Self::Configuration(fact) => fact.validate(),
            Self::FileSet(fact) => fact.validate(),
        }
    }
}

impl RawFrameworkRoleFact {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.pack_id.trim().is_empty()
            || self.framework.trim().is_empty()
            || self.role.trim().is_empty()
            || self.evidence_class.trim().is_empty()
        {
            return Err("role identity and evidence class must not be empty");
        }
        if self.subject_reference.as_deref().is_some_and(str::is_empty) {
            return Err("role subject reference must not be empty");
        }
        Ok(())
    }
}

impl RawFrameworkRelationFact {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.pack_id.trim().is_empty()
            || self.framework.trim().is_empty()
            || self.relation.trim().is_empty()
            || self.evidence_class.trim().is_empty()
            || self.ambiguity_policy.trim().is_empty()
        {
            return Err("relation identity and resolution policy must not be empty");
        }
        if self.source_reference.as_deref().is_some_and(str::is_empty)
            || self.target_hint.as_deref().is_some_and(str::is_empty)
        {
            return Err("relation references must be absent or non-empty");
        }
        if let Some(target_anchor) = self.target_anchor.as_ref()
            && !target_anchor.is_valid()
        {
            return Err("relation target anchor must be valid");
        }
        Ok(())
    }
}

impl RawFrameworkConfigurationFact {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.pack_id.trim().is_empty()
            || self.framework.trim().is_empty()
            || self.config_id.trim().is_empty()
            || self.field.trim().is_empty()
        {
            return Err("configuration identity must not be empty");
        }
        if self.complete && self.value.is_none() {
            return Err("complete configuration facts require a value");
        }
        Ok(())
    }
}

impl RawFrameworkFileSetFact {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.pack_id.trim().is_empty()
            || self.framework.trim().is_empty()
            || self.owner_reference.trim().is_empty()
            || self.patterns.is_empty()
            || self
                .patterns
                .iter()
                .any(|pattern| pattern.trim().is_empty())
            || self
                .negative_patterns
                .iter()
                .any(|pattern| pattern.trim().is_empty())
        {
            return Err("file-set identity and patterns must not be empty");
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameworkLimits {
    pub max_candidates: usize,
    pub max_include_depth: usize,
    pub max_alias_expansions: usize,
    pub max_facts_per_file: usize,
    pub max_source_bytes: usize,
    pub max_config_bytes: usize,
    pub max_syntax_nodes: usize,
    pub max_syntax_depth: usize,
    pub max_retained_literal_bytes: usize,
    pub max_role_facts: usize,
    pub max_relation_facts: usize,
    pub max_diagnostics: usize,
    pub max_route_nodes: usize,
    pub max_route_stages: usize,
    pub max_glob_patterns: usize,
    pub max_glob_matches_per_pattern: usize,
    pub max_file_set_edges: usize,
    pub max_regex_pattern_length: usize,
    pub max_regex_complexity: usize,
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
        max_source_bytes: 8 * 1024 * 1024,
        max_config_bytes: 2 * 1024 * 1024,
        max_syntax_nodes: 100_000,
        max_syntax_depth: 256,
        max_retained_literal_bytes: 64 * 1024,
        max_role_facts: 100_000,
        max_relation_facts: 100_000,
        max_diagnostics: 10_000,
        max_route_nodes: 50_000,
        max_route_stages: 100_000,
        max_glob_patterns: 2_048,
        max_glob_matches_per_pattern: 100_000,
        max_file_set_edges: 1_000_000,
        max_regex_pattern_length: 1_024,
        max_regex_complexity: 1_024,
    };

    pub fn check_facts(self, count: usize) -> Result<(), FrameworkLimitError> {
        self.check("max_facts_per_file", self.max_facts_per_file, count)
    }

    pub fn check_source_bytes(self, count: usize) -> Result<(), FrameworkLimitError> {
        self.check("max_source_bytes", self.max_source_bytes, count)
    }

    pub fn check_config_bytes(self, count: usize) -> Result<(), FrameworkLimitError> {
        self.check("max_config_bytes", self.max_config_bytes, count)
    }

    pub fn check_syntax_nodes(self, count: usize) -> Result<(), FrameworkLimitError> {
        self.check("max_syntax_nodes", self.max_syntax_nodes, count)
    }

    pub fn check_syntax_depth(self, depth: usize) -> Result<(), FrameworkLimitError> {
        self.check("max_syntax_depth", self.max_syntax_depth, depth)
    }

    pub fn check_retained_literal_bytes(self, count: usize) -> Result<(), FrameworkLimitError> {
        self.check(
            "max_retained_literal_bytes",
            self.max_retained_literal_bytes,
            count,
        )
    }

    pub fn check_role_facts(self, count: usize) -> Result<(), FrameworkLimitError> {
        self.check("max_role_facts", self.max_role_facts, count)
    }

    pub fn check_relation_facts(self, count: usize) -> Result<(), FrameworkLimitError> {
        self.check("max_relation_facts", self.max_relation_facts, count)
    }

    pub fn check_diagnostics(self, count: usize) -> Result<(), FrameworkLimitError> {
        self.check("max_diagnostics", self.max_diagnostics, count)
    }

    pub fn check_route_nodes(self, count: usize) -> Result<(), FrameworkLimitError> {
        self.check("max_route_nodes", self.max_route_nodes, count)
    }

    pub fn check_route_stages(self, count: usize) -> Result<(), FrameworkLimitError> {
        self.check("max_route_stages", self.max_route_stages, count)
    }

    pub fn check_glob_patterns(self, count: usize) -> Result<(), FrameworkLimitError> {
        self.check("max_glob_patterns", self.max_glob_patterns, count)
    }

    pub fn check_glob_matches_per_pattern(self, count: usize) -> Result<(), FrameworkLimitError> {
        self.check(
            "max_glob_matches_per_pattern",
            self.max_glob_matches_per_pattern,
            count,
        )
    }

    pub fn check_file_set_edges(self, count: usize) -> Result<(), FrameworkLimitError> {
        self.check("max_file_set_edges", self.max_file_set_edges, count)
    }

    pub fn check_regex_pattern_length(self, count: usize) -> Result<(), FrameworkLimitError> {
        self.check(
            "max_regex_pattern_length",
            self.max_regex_pattern_length,
            count,
        )
    }

    pub fn check_regex_complexity(self, count: usize) -> Result<(), FrameworkLimitError> {
        self.check("max_regex_complexity", self.max_regex_complexity, count)
    }

    fn check(
        self,
        limit: &'static str,
        maximum: usize,
        observed: usize,
    ) -> Result<(), FrameworkLimitError> {
        (observed <= maximum)
            .then_some(())
            .ok_or(FrameworkLimitError {
                limit,
                maximum,
                observed,
            })
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
