use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::provenance::{Provenance, ResolutionState, SourceAnchor};

pub const CODE_GRAPH_SCHEMA_V1: &str = "compass.graph/1";

/// The closed structural and enterprise node vocabulary for `compass.graph/1`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    File,
    Module,
    Package,
    Namespace,
    Class,
    Struct,
    Interface,
    Trait,
    Protocol,
    Enum,
    EnumMember,
    TypeAlias,
    Function,
    Method,
    Constructor,
    Property,
    Field,
    Variable,
    Constant,
    Parameter,
    Import,
    Export,
    Macro,
    Annotation,
    Route,
    Component,
    Event,
    Message,
    Topic,
    Queue,
    Job,
    Resource,
    Schema,
    Query,
    Migration,
    ConfigKey,
    Database,
    DatabaseSchema,
    DatabaseTable,
    DatabaseView,
    DatabaseColumn,
    DatabaseIndex,
    DatabaseConstraint,
    DatabaseProcedure,
    DatabaseTrigger,
}

impl NodeKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Module => "module",
            Self::Package => "package",
            Self::Namespace => "namespace",
            Self::Class => "class",
            Self::Struct => "struct",
            Self::Interface => "interface",
            Self::Trait => "trait",
            Self::Protocol => "protocol",
            Self::Enum => "enum",
            Self::EnumMember => "enum_member",
            Self::TypeAlias => "type_alias",
            Self::Function => "function",
            Self::Method => "method",
            Self::Constructor => "constructor",
            Self::Property => "property",
            Self::Field => "field",
            Self::Variable => "variable",
            Self::Constant => "constant",
            Self::Parameter => "parameter",
            Self::Import => "import",
            Self::Export => "export",
            Self::Macro => "macro",
            Self::Annotation => "annotation",
            Self::Route => "route",
            Self::Component => "component",
            Self::Event => "event",
            Self::Message => "message",
            Self::Topic => "topic",
            Self::Queue => "queue",
            Self::Job => "job",
            Self::Resource => "resource",
            Self::Schema => "schema",
            Self::Query => "query",
            Self::Migration => "migration",
            Self::ConfigKey => "config_key",
            Self::Database => "database",
            Self::DatabaseSchema => "database_schema",
            Self::DatabaseTable => "database_table",
            Self::DatabaseView => "database_view",
            Self::DatabaseColumn => "database_column",
            Self::DatabaseIndex => "database_index",
            Self::DatabaseConstraint => "database_constraint",
            Self::DatabaseProcedure => "database_procedure",
            Self::DatabaseTrigger => "database_trigger",
        }
    }

    #[must_use]
    pub const fn is_callable(self) -> bool {
        matches!(
            self,
            Self::Function | Self::Method | Self::Constructor | Self::DatabaseProcedure
        )
    }

    #[must_use]
    pub const fn is_constructible(self) -> bool {
        matches!(
            self,
            Self::Class | Self::Struct | Self::Enum | Self::Component | Self::DatabaseProcedure
        )
    }

    #[must_use]
    pub const fn is_type(self) -> bool {
        matches!(
            self,
            Self::Class
                | Self::Struct
                | Self::Interface
                | Self::Trait
                | Self::Protocol
                | Self::Enum
                | Self::TypeAlias
        )
    }

    #[must_use]
    pub const fn is_container(self) -> bool {
        matches!(
            self,
            Self::File
                | Self::Module
                | Self::Package
                | Self::Namespace
                | Self::Class
                | Self::Struct
                | Self::Interface
                | Self::Trait
                | Self::Protocol
                | Self::Enum
                | Self::Component
                | Self::Schema
                | Self::Database
                | Self::DatabaseSchema
                | Self::DatabaseTable
                | Self::DatabaseView
        )
    }
}

/// Semantic roles that enrich, but never replace, a node's structural kind.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeRole {
    Controller,
    RouteHandler,
    Middleware,
    Service,
    Resolver,
    Consumer,
    Producer,
    Subscriber,
    Repository,
    Model,
    Test,
    Fixture,
    Generated,
}

/// The closed relationship vocabulary for `compass.graph/1`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    Contains,
    Calls,
    Imports,
    Exports,
    Extends,
    Implements,
    References,
    TypeOf,
    Returns,
    Instantiates,
    Overrides,
    Decorates,
    RoutesTo,
    Reads,
    Writes,
    Aliases,
    Registers,
    Handles,
    Publishes,
    Subscribes,
    Produces,
    Consumes,
    Schedules,
    Triggers,
    Tests,
    DependsOn,
    Documents,
    MapsTo,
}

impl EdgeKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Contains => "contains",
            Self::Calls => "calls",
            Self::Imports => "imports",
            Self::Exports => "exports",
            Self::Extends => "extends",
            Self::Implements => "implements",
            Self::References => "references",
            Self::TypeOf => "type_of",
            Self::Returns => "returns",
            Self::Instantiates => "instantiates",
            Self::Overrides => "overrides",
            Self::Decorates => "decorates",
            Self::RoutesTo => "routes_to",
            Self::Reads => "reads",
            Self::Writes => "writes",
            Self::Aliases => "aliases",
            Self::Registers => "registers",
            Self::Handles => "handles",
            Self::Publishes => "publishes",
            Self::Subscribes => "subscribes",
            Self::Produces => "produces",
            Self::Consumes => "consumes",
            Self::Schedules => "schedules",
            Self::Triggers => "triggers",
            Self::Tests => "tests",
            Self::DependsOn => "depends_on",
            Self::Documents => "documents",
            Self::MapsTo => "maps_to",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    Document,
    Paper,
    Image,
    Concept,
    Rationale,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionStatus {
    Extracted,
    Partial,
    Unsupported,
    Excluded,
    ParseFailure,
    Generated,
    Binary,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageStatus {
    Complete,
    Partial,
    Unsupported,
    Excluded,
    Failed,
    Indeterminate,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GraphDiagnostic {
    pub severity: DiagnosticSeverity,
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<SourceAnchor>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CoverageRecord {
    pub capability: String,
    pub producer: String,
    pub status: CoverageStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<SourceAnchor>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileRecord {
    pub id: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    pub content_digest: String,
    pub byte_size: u64,
    pub generated: bool,
    pub extraction_status: ExtractionStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extractor_versions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub coverage: Vec<CoverageRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<GraphDiagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BuildMetadata {
    pub builder_version: String,
    pub schema_fingerprint: String,
    pub source_tree_digest: String,
    pub configuration_digest: String,
    pub generation_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GraphMetadata {
    pub schema: String,
    pub build: BuildMetadata,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<FileRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub coverage: Vec<CoverageRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<GraphDiagnostic>,
}

impl GraphMetadata {
    #[must_use]
    pub fn v1(build: BuildMetadata) -> Self {
        Self {
            schema: CODE_GRAPH_SCHEMA_V1.to_owned(),
            build,
            files: Vec::new(),
            coverage: Vec::new(),
            diagnostics: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommunityMetadata {
    pub id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileNodeDetails {
    pub content_digest: String,
    pub byte_size: u64,
    pub generated: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SymbolNodeDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modifiers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overload_discriminator: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declaring_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub implementation_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_digest: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportExportNodeDetails {
    pub specifier: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imported_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_name: Option<String>,
    #[serde(default)]
    pub type_only: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouteNodeDetails {
    pub operation: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_path: Option<String>,
    pub declaring_scope: String,
    pub resolution: ResolutionState,
    #[serde(default)]
    pub middleware_count: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ComponentNodeDetails {
    pub component_type: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceNodeDetails {
    pub resource_kind: ResourceKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MessagingNodeDetails {
    pub transport: String,
    pub subject: String,
    pub declaring_scope: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JobNodeDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SchemaNodeDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dialect: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logical_database: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueryNodeDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dialect: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_digest: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigNodeDetails {
    pub format: String,
    pub key_path: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DatabaseNodeDetails {
    pub logical_database: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_schema: Option<String>,
}

/// Closed, category-specific node payloads.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum NodeDetails {
    File(FileNodeDetails),
    Symbol(SymbolNodeDetails),
    ImportExport(ImportExportNodeDetails),
    Route(RouteNodeDetails),
    Component(ComponentNodeDetails),
    Resource(ResourceNodeDetails),
    Messaging(MessagingNodeDetails),
    Job(JobNodeDetails),
    Schema(SchemaNodeDetails),
    Query(QueryNodeDetails),
    Config(ConfigNodeDetails),
    Database(DatabaseNodeDetails),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NodeRecord {
    pub id: String,
    pub kind: NodeKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<NodeRole>,
    pub name: String,
    pub qualified_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub framework: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceAnchor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<NodeDetails>,
    pub evidence: Vec<Provenance>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub coverage: Vec<CoverageRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<GraphDiagnostic>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub community: Option<CommunityMetadata>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallDispatch {
    Static,
    Virtual,
    Dynamic,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallEdgeDetails {
    pub dispatch: CallDispatch,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receiver_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argument_count: Option<u32>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteStage {
    Middleware,
    Handler,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouteEdgeDetails {
    pub stage: RouteStage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MessagingEdgeDetails {
    pub transport: String,
    pub subject: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScheduleEdgeDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MappingEdgeDetails {
    pub mapping_kind: String,
}

/// Closed, category-specific relationship payloads.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum EdgeDetails {
    Call(CallEdgeDetails),
    Route(RouteEdgeDetails),
    Messaging(MessagingEdgeDetails),
    Schedule(ScheduleEdgeDetails),
    Mapping(MappingEdgeDetails),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EdgeRecord {
    pub id: String,
    pub key: String,
    pub source: String,
    pub target: String,
    pub kind: EdgeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relationship_site: Option<SourceAnchor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<EdgeDetails>,
    pub evidence: Vec<Provenance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub deferred: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<GraphDiagnostic>,
}

impl EdgeRecord {
    #[must_use]
    pub fn has_networkx_identity(&self) -> bool {
        self.id == self.key
    }

    #[must_use]
    pub const fn relation(&self) -> &'static str {
        self.kind.as_str()
    }

    #[must_use]
    pub fn source_file(&self) -> Option<&str> {
        self.relationship_site
            .as_ref()
            .map(|anchor| anchor.file.as_str())
    }

    #[must_use]
    pub fn semantic_source(&self) -> &str {
        &self.source
    }

    #[must_use]
    pub fn semantic_target(&self) -> &str {
        &self.target
    }

    #[must_use]
    pub fn number(&self, key: &str) -> Option<f64> {
        self.property(key).and_then(|value| value.as_f64())
    }

    #[must_use]
    pub fn boolean(&self, key: &str) -> Option<bool> {
        self.property(key).and_then(|value| value.as_bool())
    }

    #[must_use]
    pub fn property(&self, key: &str) -> Option<Value> {
        match key {
            "id" | "key" => Some(Value::String(self.id.clone())),
            "source" | "_src" => Some(Value::String(self.source.clone())),
            "target" | "_tgt" => Some(Value::String(self.target.clone())),
            "kind" | "relation" => Some(Value::String(self.kind.as_str().to_owned())),
            "source_file" => self
                .relationship_site
                .as_ref()
                .map(|anchor| Value::String(anchor.file.clone())),
            "source_location" => self.relationship_site.as_ref().map(|anchor| {
                Value::String(format!(
                    "L{}:{}-L{}:{}",
                    anchor.start_line, anchor.start_column, anchor.end_line, anchor.end_column
                ))
            }),
            "confidence" => self
                .evidence
                .first()
                .map(|evidence| Value::String(evidence.confidence.legacy_str().to_owned())),
            "confidence_score" => self
                .evidence
                .iter()
                .find_map(|evidence| evidence.score)
                .map(Value::from),
            "_origin" => self
                .evidence
                .first()
                .map(|evidence| Value::String(evidence.origin.as_str().to_owned())),
            "weight" => self.weight.map(Value::from),
            "context" => self.context.clone().map(Value::String),
            "deferred" => self.deferred.then_some(Value::Bool(true)),
            _ => None,
        }
    }

    #[must_use]
    pub fn string(&self, key: &str) -> String {
        self.property(key)
            .as_ref()
            .and_then(value_as_python_string)
            .unwrap_or_default()
    }

    pub fn properties(&self) -> EdgePropertyProjection<'_> {
        EdgePropertyProjection {
            edge: self,
            position: 0,
        }
    }
}

impl NodeRecord {
    #[must_use]
    pub fn label(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn source_file(&self) -> Option<&str> {
        self.source.as_ref().map(|anchor| anchor.file.as_str())
    }

    #[must_use]
    pub fn language_name(&self) -> Option<&str> {
        self.language.as_deref()
    }

    #[must_use]
    pub const fn kind_name(&self) -> &'static str {
        self.kind.as_str()
    }

    #[must_use]
    pub fn digest(&self, key: &str) -> Option<&str> {
        let details = self.symbol_details()?;
        match key {
            "signature_hash" => details
                .signature_digest
                .as_deref()
                .or(details.overload_discriminator.as_deref()),
            "implementation_hash" => details.implementation_digest.as_deref(),
            "source_hash" => details.source_digest.as_deref(),
            _ => None,
        }
    }

    #[must_use]
    pub fn unsigned(&self, key: &str) -> Option<u64> {
        self.property(key).and_then(|value| value.as_u64())
    }

    #[must_use]
    pub fn property(&self, key: &str) -> Option<Value> {
        match key {
            "id" => Some(Value::String(self.id.clone())),
            "label" | "name" => Some(Value::String(self.name.clone())),
            "qualified_name" | "qualifiedName" => Some(Value::String(self.qualified_name.clone())),
            "kind" | "type" | "symbol_kind" | "node_type" => {
                Some(Value::String(self.kind.as_str().to_owned()))
            }
            "file_type" => Some(Value::String(
                if self.kind == NodeKind::Resource {
                    self.details
                        .as_ref()
                        .and_then(|details| match details {
                            NodeDetails::Resource(details) => {
                                Some(resource_kind_str(details.resource_kind))
                            }
                            _ => None,
                        })
                        .unwrap_or("document")
                } else {
                    "code"
                }
                .to_owned(),
            )),
            "language" => self.language.clone().map(Value::String),
            "framework" => self.framework.clone().map(Value::String),
            "source_file" => self
                .source
                .as_ref()
                .map(|anchor| Value::String(anchor.file.clone())),
            "source_location" => self.source.as_ref().map(|anchor| {
                Value::String(format!(
                    "L{}:{}-L{}:{}",
                    anchor.start_line, anchor.start_column, anchor.end_line, anchor.end_column
                ))
            }),
            "line_start" => self
                .source
                .as_ref()
                .map(|anchor| Value::from(anchor.start_line)),
            "line_end" => self
                .source
                .as_ref()
                .map(|anchor| Value::from(anchor.end_line)),
            "community" => self
                .community
                .as_ref()
                .map(|community| Value::from(community.id)),
            "community_name" => self
                .community
                .as_ref()
                .and_then(|community| community.label.clone())
                .map(Value::String),
            "signature" => self.symbol_details().and_then(|details| {
                details
                    .signature
                    .as_ref()
                    .map(|value| Value::String(value.clone()))
            }),
            "signature_hash" => self.symbol_details().and_then(|details| {
                details
                    .signature_digest
                    .as_ref()
                    .or(details.overload_discriminator.as_ref())
                    .map(|value| Value::String(value.clone()))
            }),
            "implementation_hash" => self.symbol_details().and_then(|details| {
                details
                    .implementation_digest
                    .as_ref()
                    .map(|value| Value::String(value.clone()))
            }),
            "source_hash" => self.symbol_details().and_then(|details| {
                details
                    .source_digest
                    .as_ref()
                    .map(|value| Value::String(value.clone()))
            }),
            "_origin" => self
                .evidence
                .first()
                .map(|evidence| Value::String(evidence.origin.as_str().to_owned())),
            "confidence" => self
                .evidence
                .first()
                .map(|evidence| Value::String(evidence.confidence.legacy_str().to_owned())),
            "roles" => Some(Value::Array(
                self.roles
                    .iter()
                    .map(|role| {
                        serde_json::to_value(role).unwrap_or_else(|_| Value::String(String::new()))
                    })
                    .collect(),
            )),
            _ => None,
        }
    }

    #[must_use]
    pub fn string(&self, key: &str) -> String {
        self.property(key)
            .as_ref()
            .and_then(value_as_python_string)
            .unwrap_or_default()
    }

    pub fn properties(&self) -> NodePropertyProjection<'_> {
        NodePropertyProjection {
            node: self,
            position: 0,
        }
    }

    fn symbol_details(&self) -> Option<&SymbolNodeDetails> {
        match self.details.as_ref()? {
            NodeDetails::Symbol(details) => Some(details),
            _ => None,
        }
    }
}

const NODE_PROPERTY_KEYS: &[&str] = &[
    "id",
    "label",
    "qualified_name",
    "kind",
    "roles",
    "file_type",
    "language",
    "framework",
    "source_file",
    "source_location",
    "line_start",
    "line_end",
    "signature",
    "signature_hash",
    "implementation_hash",
    "source_hash",
    "community",
    "community_name",
    "_origin",
    "confidence",
];

const EDGE_PROPERTY_KEYS: &[&str] = &[
    "id",
    "key",
    "source",
    "target",
    "kind",
    "relation",
    "source_file",
    "source_location",
    "confidence",
    "confidence_score",
    "_origin",
    "weight",
    "context",
    "deferred",
];

pub struct NodePropertyProjection<'a> {
    node: &'a NodeRecord,
    position: usize,
}

impl Iterator for NodePropertyProjection<'_> {
    type Item = (&'static str, Value);

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(&key) = NODE_PROPERTY_KEYS.get(self.position) {
            self.position += 1;
            if let Some(value) = self.node.property(key) {
                return Some((key, value));
            }
        }
        None
    }
}

pub struct EdgePropertyProjection<'a> {
    edge: &'a EdgeRecord,
    position: usize,
}

impl Iterator for EdgePropertyProjection<'_> {
    type Item = (&'static str, Value);

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(&key) = EDGE_PROPERTY_KEYS.get(self.position) {
            self.position += 1;
            if let Some(value) = self.edge.property(key) {
                return Some((key, value));
            }
        }
        None
    }
}

fn resource_kind_str(kind: ResourceKind) -> &'static str {
    match kind {
        ResourceKind::Document => "document",
        ResourceKind::Paper => "paper",
        ResourceKind::Image => "image",
        ResourceKind::Concept => "concept",
        ResourceKind::Rationale => "rationale",
    }
}

fn value_as_python_string(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(text) => Some(text.clone()),
        Value::Bool(value) => Some(if *value { "True" } else { "False" }.to_owned()),
        Value::Number(value) => Some(value.to_string()),
        Value::Array(_) | Value::Object(_) => Some(value.to_string()),
    }
}

const fn is_false(value: &bool) -> bool {
    !*value
}

/// Strict Compass records inside a NetworkX node-link envelope.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphDocument {
    pub directed: bool,
    pub multigraph: bool,
    pub graph: GraphMetadata,
    pub nodes: Vec<NodeRecord>,
    pub links: Vec<EdgeRecord>,
}

impl GraphDocument {
    #[must_use]
    pub fn empty_v1(build: BuildMetadata) -> Self {
        Self {
            directed: true,
            multigraph: true,
            graph: GraphMetadata::v1(build),
            nodes: Vec::new(),
            links: Vec::new(),
        }
    }
}
