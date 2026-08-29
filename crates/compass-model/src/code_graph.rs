use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::provenance::{
    Provenance, ResolutionCandidate, ResolutionState, SourceAnchor, effective_confidence,
};
use crate::{GraphError, validate_code_graph};

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
    pub const ALL: [Self; 45] = [
        Self::File,
        Self::Module,
        Self::Package,
        Self::Namespace,
        Self::Class,
        Self::Struct,
        Self::Interface,
        Self::Trait,
        Self::Protocol,
        Self::Enum,
        Self::EnumMember,
        Self::TypeAlias,
        Self::Function,
        Self::Method,
        Self::Constructor,
        Self::Property,
        Self::Field,
        Self::Variable,
        Self::Constant,
        Self::Parameter,
        Self::Import,
        Self::Export,
        Self::Macro,
        Self::Annotation,
        Self::Route,
        Self::Component,
        Self::Event,
        Self::Message,
        Self::Topic,
        Self::Queue,
        Self::Job,
        Self::Resource,
        Self::Schema,
        Self::Query,
        Self::Migration,
        Self::ConfigKey,
        Self::Database,
        Self::DatabaseSchema,
        Self::DatabaseTable,
        Self::DatabaseView,
        Self::DatabaseColumn,
        Self::DatabaseIndex,
        Self::DatabaseConstraint,
        Self::DatabaseProcedure,
        Self::DatabaseTrigger,
    ];

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
                | Self::Resource
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

impl NodeRole {
    pub const ALL: [Self; 13] = [
        Self::Controller,
        Self::RouteHandler,
        Self::Middleware,
        Self::Service,
        Self::Resolver,
        Self::Consumer,
        Self::Producer,
        Self::Subscriber,
        Self::Repository,
        Self::Model,
        Self::Test,
        Self::Fixture,
        Self::Generated,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Controller => "controller",
            Self::RouteHandler => "route_handler",
            Self::Middleware => "middleware",
            Self::Service => "service",
            Self::Resolver => "resolver",
            Self::Consumer => "consumer",
            Self::Producer => "producer",
            Self::Subscriber => "subscriber",
            Self::Repository => "repository",
            Self::Model => "model",
            Self::Test => "test",
            Self::Fixture => "fixture",
            Self::Generated => "generated",
        }
    }
}

/// The closed relationship vocabulary for `compass.graph/1`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    Contains,
    Embeds,
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
    pub const ALL: [Self; 29] = [
        Self::Contains,
        Self::Embeds,
        Self::Calls,
        Self::Imports,
        Self::Exports,
        Self::Extends,
        Self::Implements,
        Self::References,
        Self::TypeOf,
        Self::Returns,
        Self::Instantiates,
        Self::Overrides,
        Self::Decorates,
        Self::RoutesTo,
        Self::Reads,
        Self::Writes,
        Self::Aliases,
        Self::Registers,
        Self::Handles,
        Self::Publishes,
        Self::Subscribes,
        Self::Produces,
        Self::Consumes,
        Self::Schedules,
        Self::Triggers,
        Self::Tests,
        Self::DependsOn,
        Self::Documents,
        Self::MapsTo,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Contains => "contains",
            Self::Embeds => "embeds",
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_commit: Option<String>,
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouteStageDetails {
    pub stage: RouteStage,
    pub position: u32,
    pub reference: String,
    pub resolution: ResolutionState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<ResolutionCandidate>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stages: Vec<RouteStageDetails>,
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
    pub occurrence_rule: Option<crate::provenance::OccurrenceRule>,
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
                .is_empty()
                .then(|| Value::String("EXTRACTED".to_owned()))
                .or_else(|| {
                    effective_confidence(&self.evidence)
                        .map(|confidence| Value::String(confidence.legacy_str().to_owned()))
                }),
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
                .is_empty()
                .then(|| Value::String("EXTRACTED".to_owned()))
                .or_else(|| {
                    effective_confidence(&self.evidence)
                        .map(|confidence| Value::String(confidence.legacy_str().to_owned()))
                }),
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

    /// Load and validate a trusted `compass.graph/1` artifact.
    pub fn load(path: &Path) -> Result<Self, GraphError> {
        if path.extension().and_then(|part| part.to_str()) != Some("json") {
            return Err(GraphError::InvalidExtension(path.to_path_buf()));
        }
        Self::load_strict(path)
    }

    /// Load, validate, and identify the exact bytes read from one opened graph
    /// artifact. The document and digest can therefore never describe two
    /// different path realizations when an atomic publisher replaces the path.
    pub fn load_with_artifact_digest(path: &Path) -> Result<(Self, String), GraphError> {
        if path.extension().and_then(|part| part.to_str()) != Some("json") {
            return Err(GraphError::InvalidExtension(path.to_path_buf()));
        }
        Self::load_for_recluster_with_artifact_digest(path)
    }

    /// Load, validate, and identify an exact graph artifact for re-clustering
    /// without requiring a filename extension.
    pub fn load_for_recluster_with_artifact_digest(
        path: &Path,
    ) -> Result<(Self, String), GraphError> {
        let file = File::open(path).map_err(|source| GraphError::Read {
            path: crate::graph::absolute_path(path),
            source,
        })?;
        load_opened_with_artifact_digest(path, file, crate::graph::graph_size_cap())
    }

    /// Load the complete graph for impact traversal.
    ///
    /// V1 records are already typed and compact projections are derived from
    /// the content-addressed query index, so this retains the trusted document.
    pub fn load_for_affected(path: &Path) -> Result<Self, GraphError> {
        Self::load(path)
    }

    /// Load a graph without enforcing the filename extension.
    pub fn load_for_recluster(path: &Path) -> Result<Self, GraphError> {
        Self::load_strict(path)
    }

    /// Project the validated typed document into the compatibility node-link
    /// view used by legacy renderers and CompassQL.
    pub fn to_legacy_document(&self) -> Result<crate::GraphDocument, GraphError> {
        let nodes = self
            .nodes
            .iter()
            .map(legacy_node_record)
            .collect::<Result<Vec<_>, _>>()?;
        let links = self
            .links
            .iter()
            .map(legacy_edge_record)
            .collect::<Result<Vec<_>, _>>()?;
        legacy_document(self.directed, self.multigraph, &self.graph, nodes, links)
    }

    /// Consume the typed document while projecting it into the compatibility
    /// node-link view without first allocating a second JSON envelope.
    pub fn into_legacy_document(self) -> Result<crate::GraphDocument, GraphError> {
        let Self {
            directed,
            multigraph,
            graph,
            nodes,
            links,
        } = self;
        let nodes = nodes
            .into_iter()
            .map(|node| legacy_node_record(&node))
            .collect::<Result<Vec<_>, _>>()?;
        let links = links
            .into_iter()
            .map(|edge| legacy_edge_record(&edge))
            .collect::<Result<Vec<_>, _>>()?;
        legacy_document(directed, multigraph, &graph, nodes, links)
    }

    /// Consume a compatibility projection produced from a typed document and
    /// reconstruct the strict authority without retaining both full record
    /// sets at once.
    ///
    /// This is the inverse of [`Self::into_legacy_document`]. It is intended
    /// for bounded analysis stages that still consume the compatibility view:
    /// callers may move the typed authority into that view, run the analysis,
    /// and then move the records back before publication. Unknown top-level
    /// compatibility fields are rejected because `compass.graph/1` has a
    /// closed envelope.
    pub fn from_legacy_document(document: crate::GraphDocument) -> Result<Self, GraphError> {
        let crate::GraphDocument {
            directed,
            multigraph,
            graph,
            nodes,
            links,
            extras,
        } = document;
        if !extras.is_empty() {
            return Err(corrupt_projection(
                "legacy projection contains unknown top-level fields",
            ));
        }
        let graph = serde_json::from_value(Value::Object(graph)).map_err(GraphError::Corrupt)?;
        let nodes = nodes
            .into_iter()
            .map(typed_node_record)
            .collect::<Result<Vec<_>, _>>()?;
        let links = links
            .into_iter()
            .map(typed_edge_record)
            .collect::<Result<Vec<_>, _>>()?;
        let document = Self {
            directed,
            multigraph,
            graph,
            nodes,
            links,
        };
        validate_code_graph(&document)?;
        Ok(document)
    }

    #[must_use]
    pub fn size_cap_exceeded(path: &Path) -> Option<(u64, u64)> {
        let size = path.metadata().ok()?.len();
        let cap = crate::graph::graph_size_cap();
        (size > cap).then_some((size, cap))
    }

    fn load_strict(path: &Path) -> Result<Self, GraphError> {
        if !path.exists() {
            return Err(GraphError::NotFound(crate::graph::absolute_path(path)));
        }
        if let Some((size, cap)) = Self::size_cap_exceeded(path) {
            return Err(GraphError::TooLarge {
                path: crate::graph::absolute_path(path),
                size,
                cap,
            });
        }
        let digest = file_digest(path)?;
        if let Some(document) = load_content_cache(path, &digest) {
            validate_code_graph(&document)?;
            return Ok(document);
        }

        #[derive(Deserialize)]
        struct SchemaEnvelope {
            #[serde(default)]
            graph: Option<SchemaHeader>,
        }

        #[derive(Deserialize)]
        struct SchemaHeader {
            #[serde(default)]
            schema: Option<String>,
        }

        // Inspect the version without first allocating a complete generic JSON
        // tree. Serde skips the large node and edge arrays while retaining the
        // explicit unsupported-schema diagnostic.
        let schema_file = File::open(path).map_err(|source| GraphError::Read {
            path: crate::graph::absolute_path(path),
            source,
        })?;
        let found = serde_json::from_reader::<_, SchemaEnvelope>(BufReader::new(schema_file))
            .map_err(GraphError::Corrupt)?
            .graph
            .and_then(|graph| graph.schema);
        if found.as_deref() != Some(CODE_GRAPH_SCHEMA_V1) {
            return Err(GraphError::UnsupportedGraphSchema { found });
        }
        let document_file = File::open(path).map_err(|source| GraphError::Read {
            path: crate::graph::absolute_path(path),
            source,
        })?;
        let document =
            serde_json::from_reader(BufReader::new(document_file)).map_err(GraphError::Corrupt)?;
        validate_code_graph(&document)?;
        let _ = write_content_cache(path, &digest, &document);
        Ok(document)
    }
}

fn load_opened_with_artifact_digest(
    path: &Path,
    file: File,
    cap: u64,
) -> Result<(GraphDocument, String), GraphError> {
    load_opened_with_artifact_digest_after_metadata(path, file, cap, || Ok(()))
}

fn load_opened_with_artifact_digest_after_metadata<F>(
    path: &Path,
    mut file: File,
    cap: u64,
    after_metadata: F,
) -> Result<(GraphDocument, String), GraphError>
where
    F: FnOnce() -> std::io::Result<()>,
{
    let size = file
        .metadata()
        .map_err(|source| GraphError::Read {
            path: crate::graph::absolute_path(path),
            source,
        })?
        .len();
    if size > cap {
        return Err(GraphError::TooLarge {
            path: crate::graph::absolute_path(path),
            size,
            cap,
        });
    }
    after_metadata().map_err(|source| GraphError::Read {
        path: crate::graph::absolute_path(path),
        source,
    })?;
    let actual_size = file
        .metadata()
        .map_err(|source| GraphError::Read {
            path: crate::graph::absolute_path(path),
            source,
        })?
        .len();
    if actual_size > cap {
        return Err(GraphError::TooLarge {
            path: crate::graph::absolute_path(path),
            size: actual_size,
            cap,
        });
    }

    #[derive(Deserialize)]
    struct SchemaEnvelope {
        #[serde(default)]
        graph: Option<SchemaHeader>,
    }

    #[derive(Deserialize)]
    struct SchemaHeader {
        #[serde(default)]
        schema: Option<String>,
    }

    let found = serde_json::from_reader::<_, SchemaEnvelope>(BufReader::new(
        (&mut file).take(cap.saturating_add(1)),
    ))
    .map_err(GraphError::Corrupt)?
    .graph
    .and_then(|graph| graph.schema);
    if found.as_deref() != Some(CODE_GRAPH_SCHEMA_V1) {
        return Err(GraphError::UnsupportedGraphSchema { found });
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|source| GraphError::Read {
            path: crate::graph::absolute_path(path),
            source,
        })?;
    let mut reader = BufReader::new(BoundedHashReader::new(file, cap));
    let decoded = serde_json::from_reader(&mut reader);
    let hashed = reader.into_inner();
    if hashed.exceeded {
        return Err(GraphError::TooLarge {
            path: crate::graph::absolute_path(path),
            size: cap.saturating_add(1),
            cap,
        });
    }
    let document = decoded.map_err(GraphError::Corrupt)?;
    validate_code_graph(&document)?;
    let digest = format!("{:x}", hashed.digest.finalize());
    Ok((document, digest))
}

struct BoundedHashReader<R> {
    inner: R,
    cap: u64,
    bytes: u64,
    exceeded: bool,
    digest: Sha256,
}

impl<R> BoundedHashReader<R> {
    fn new(inner: R, cap: u64) -> Self {
        Self {
            inner,
            cap,
            bytes: 0,
            exceeded: false,
            digest: Sha256::new(),
        }
    }
}

impl<R: Read> Read for BoundedHashReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.exceeded {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "graph grew beyond its configured byte limit",
            ));
        }
        let remaining = self.cap.saturating_sub(self.bytes);
        let maximum = usize::try_from(remaining)
            .unwrap_or(usize::MAX)
            .saturating_add(1)
            .min(buffer.len());
        let read = self.inner.read(&mut buffer[..maximum])?;
        self.bytes = self.bytes.saturating_add(read as u64);
        if self.bytes > self.cap {
            self.exceeded = true;
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "graph grew beyond its configured byte limit",
            ));
        }
        self.digest.update(&buffer[..read]);
        Ok(read)
    }
}

fn legacy_document(
    directed: bool,
    multigraph: bool,
    graph: &GraphMetadata,
    nodes: Vec<crate::NodeRecord>,
    links: Vec<crate::EdgeRecord>,
) -> Result<crate::GraphDocument, GraphError> {
    let graph = serde_json::to_value(graph)
        .map_err(GraphError::Corrupt)?
        .as_object()
        .cloned()
        .ok_or_else(|| {
            GraphError::Corrupt(serde_json::Error::io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "typed graph metadata is not an object",
            )))
        })?;
    Ok(crate::GraphDocument {
        directed,
        multigraph,
        graph,
        nodes,
        links,
        extras: std::collections::BTreeMap::new(),
    })
}

fn legacy_node_record(node: &NodeRecord) -> Result<crate::NodeRecord, GraphError> {
    let value = serde_json::to_value(node).map_err(GraphError::Corrupt)?;
    let mut object = value.as_object().cloned().ok_or_else(|| {
        GraphError::Corrupt(serde_json::Error::io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "typed node is not an object",
        )))
    })?;
    let id = object
        .remove("id")
        .and_then(|value| value.as_str().map(str::to_owned))
        .ok_or(GraphError::MissingNodeId)?;
    Ok(crate::NodeRecord {
        id,
        attributes: object,
    })
}

fn legacy_edge_record(edge: &EdgeRecord) -> Result<crate::EdgeRecord, GraphError> {
    let value = serde_json::to_value(edge).map_err(GraphError::Corrupt)?;
    let mut object = value.as_object().cloned().ok_or_else(|| {
        GraphError::Corrupt(serde_json::Error::io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "typed edge is not an object",
        )))
    })?;
    let source = object
        .remove("source")
        .and_then(|value| value.as_str().map(str::to_owned))
        .ok_or(GraphError::InvalidEdgeEndpoint)?;
    let target = object
        .remove("target")
        .and_then(|value| value.as_str().map(str::to_owned))
        .ok_or(GraphError::InvalidEdgeEndpoint)?;
    Ok(crate::EdgeRecord {
        source,
        target,
        attributes: object,
    })
}

fn typed_node_record(node: crate::NodeRecord) -> Result<NodeRecord, GraphError> {
    let mut object = node.attributes;
    object.insert("id".to_owned(), Value::String(node.id));
    serde_json::from_value(Value::Object(object)).map_err(GraphError::Corrupt)
}

fn typed_edge_record(edge: crate::EdgeRecord) -> Result<EdgeRecord, GraphError> {
    let mut object = edge.attributes;
    object.insert("source".to_owned(), Value::String(edge.source));
    object.insert("target".to_owned(), Value::String(edge.target));
    serde_json::from_value(Value::Object(object)).map_err(GraphError::Corrupt)
}

fn corrupt_projection(message: &str) -> GraphError {
    GraphError::Corrupt(serde_json::Error::io(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        message,
    )))
}

fn file_digest(path: &Path) -> Result<String, GraphError> {
    let file = File::open(path).map_err(|source| GraphError::Read {
        path: crate::graph::absolute_path(path),
        source,
    })?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|source| GraphError::Read {
                path: crate::graph::absolute_path(path),
                source,
            })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

const CONTENT_CACHE_MAGIC: &[u8; 8] = b"CGRPHV01";
static CONTENT_CACHE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn content_cache_path(graph_path: &Path, digest: &str) -> PathBuf {
    let file_name = graph_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("graph.json");
    graph_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("cache")
        .join(format!("{file_name}.{digest}.content-v1.cache"))
}

fn load_content_cache(path: &Path, digest: &str) -> Option<GraphDocument> {
    let mut reader = BufReader::new(File::open(content_cache_path(path, digest)).ok()?);
    let mut magic = [0_u8; CONTENT_CACHE_MAGIC.len()];
    std::io::Read::read_exact(&mut reader, &mut magic).ok()?;
    if &magic != CONTENT_CACHE_MAGIC {
        return None;
    }
    rmp_serde::from_read(reader).ok()
}

fn write_content_cache(
    graph_path: &Path,
    digest: &str,
    document: &GraphDocument,
) -> std::io::Result<()> {
    let cache_path = content_cache_path(graph_path, digest);
    if cache_path.exists() {
        return Ok(());
    }
    let Some(parent) = cache_path.parent() else {
        return Ok(());
    };
    fs::create_dir_all(parent)?;
    let sequence = CONTENT_CACHE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = cache_path.with_extension(format!("tmp-{}-{sequence}", std::process::id()));
    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    let result = (|| {
        let mut writer = BufWriter::new(file);
        writer.write_all(CONTENT_CACHE_MAGIC)?;
        rmp_serde::encode::write_named(&mut writer, document).map_err(std::io::Error::other)?;
        writer.flush()?;
        drop(writer);
        match fs::rename(&temporary, &cache_path) {
            Ok(()) => Ok(()),
            Err(_error) if cache_path.exists() => Ok(()),
            Err(error) => Err(error),
        }
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use std::fs::{self, File, OpenOptions};
    use std::io::Write;
    use std::path::Path;

    use crate::identity::file_id;
    use crate::provenance::{EvidenceConfidence, EvidenceOrigin, Provenance, SourceAnchor};
    use sha2::{Digest, Sha256};

    use super::{
        BuildMetadata, CommunityMetadata, ExtractionStatus, FileRecord, GraphDocument, NodeKind,
        NodeRecord, content_cache_path, load_opened_with_artifact_digest,
        load_opened_with_artifact_digest_after_metadata,
    };

    fn document() -> GraphDocument {
        GraphDocument::empty_v1(BuildMetadata {
            builder_version: "test".to_owned(),
            schema_fingerprint: "schema".to_owned(),
            source_tree_digest: "tree".to_owned(),
            configuration_digest: "config".to_owned(),
            generation_id: "generation".to_owned(),
            source_commit: None,
        })
    }

    #[test]
    fn content_cache_path_is_visible_and_scoped() {
        assert_eq!(
            content_cache_path(Path::new("compass-out/graph.json"), "abc123"),
            Path::new("compass-out/cache/graph.json.abc123.content-v1.cache")
        );
    }

    #[test]
    fn consuming_legacy_projection_round_trips_typed_records() -> Result<(), crate::GraphError> {
        let mut expected = document();
        expected.graph.files.push(FileRecord {
            id: file_id("src/lib.rs"),
            path: "src/lib.rs".to_owned(),
            language: Some("rust".to_owned()),
            content_digest: "sha256:test".to_owned(),
            byte_size: 7,
            generated: false,
            extraction_status: ExtractionStatus::Extracted,
            extractor_versions: vec!["test".to_owned()],
            coverage: Vec::new(),
            diagnostics: Vec::new(),
        });
        let anchor = SourceAnchor {
            file: "src/lib.rs".to_owned(),
            start_byte: 0,
            end_byte: 7,
            start_line: 1,
            start_column: 0,
            end_line: 1,
            end_column: 7,
        };
        expected.nodes.push(NodeRecord {
            id: "fn:example".to_owned(),
            kind: NodeKind::Function,
            roles: Vec::new(),
            name: "example".to_owned(),
            qualified_name: "crate::example".to_owned(),
            language: Some("rust".to_owned()),
            framework: None,
            source: Some(anchor.clone()),
            details: None,
            evidence: vec![Provenance {
                origin: EvidenceOrigin::Ast,
                extractor: "test".to_owned(),
                confidence: EvidenceConfidence::Exact,
                rule: None,
                anchors: vec![anchor],
                wiring_site: None,
                score: None,
                candidates: Vec::new(),
            }],
            coverage: Vec::new(),
            diagnostics: Vec::new(),
            community: Some(CommunityMetadata {
                id: 7,
                label: Some("Example".to_owned()),
                score: None,
                color: None,
            }),
        });

        let legacy = expected.clone().into_legacy_document()?;
        let actual = GraphDocument::from_legacy_document(legacy)?;

        assert_eq!(actual, expected);
        Ok(())
    }

    #[test]
    fn opened_artifact_keeps_document_and_digest_bound_across_path_replacement()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("graph.json");
        let original_document = document();
        let original = serde_json::to_vec(&original_document)?;
        fs::write(&path, &original)?;
        let opened = File::open(&path)?;
        fs::rename(&path, directory.path().join("original.json"))?;
        fs::write(&path, b"not the opened graph")?;

        let (document, digest) = load_opened_with_artifact_digest(&path, opened, 1024 * 1024)?;
        assert_eq!(document, original_document);
        assert_eq!(digest, format!("{:x}", Sha256::digest(&original)));
        Ok(())
    }

    #[test]
    fn opened_artifact_rejects_growth_past_the_limit_after_metadata_check()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("graph.json");
        fs::write(&path, b"1234")?;
        let growth_path = path.clone();
        let mut growth_file = OpenOptions::new().append(true).open(&growth_path)?;
        let result = load_opened_with_artifact_digest_after_metadata(
            &path,
            File::open(&path)?,
            8,
            move || growth_file.write_all(b"56789"),
        );
        let error = match result {
            Err(error) => error,
            Ok(_) => return Err("oversized opened artifact should fail".into()),
        };
        assert!(matches!(
            error,
            crate::GraphError::TooLarge {
                size: 9,
                cap: 8,
                ..
            }
        ));
        Ok(())
    }
}
