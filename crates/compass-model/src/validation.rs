use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use rayon::prelude::*;
use serde_json::Value;

use crate::code_graph::{
    CODE_GRAPH_SCHEMA_V1, EdgeKind, GraphDocument as CodeGraphDocument, NodeDetails, NodeKind,
    NodeRole,
};
use crate::identity::{edge_id, file_id};
use crate::provenance::{Provenance, SourceAnchor};

const VALID_FILE_TYPES: [&str; 6] = ["code", "concept", "document", "image", "paper", "rationale"];
const VALID_CONFIDENCES: [&str; 3] = ["AMBIGUOUS", "EXTRACTED", "INFERRED"];

// CPython's set iteration order with the compatibility harness' PYTHONHASHSEED=0.
const REQUIRED_NODE_FIELDS: [&str; 4] = ["file_type", "id", "source_file", "label"];
const REQUIRED_EDGE_FIELDS: [&str; 5] =
    ["source_file", "target", "confidence", "source", "relation"];

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum HashableJson {
    None,
    String(String),
    Integer(i128),
    Float(u64),
}

/// Validate raw extraction JSON using Compass's external schema and diagnostics.
#[must_use]
pub fn validate_extraction(data: &Value) -> Vec<String> {
    let Some(data) = data.as_object() else {
        return vec!["Extraction must be a JSON object".to_owned()];
    };
    let mut errors = Vec::new();
    let mut node_ids = HashSet::new();

    match data.get("nodes") {
        None => errors.push("Missing required key 'nodes'".to_owned()),
        Some(Value::Array(nodes)) => {
            for (index, node) in nodes.iter().enumerate() {
                let Some(node) = node.as_object() else {
                    errors.push(format!("Node {index} must be an object"));
                    continue;
                };
                for field in REQUIRED_NODE_FIELDS {
                    if !node.contains_key(field) {
                        let id = node.get("id").map_or_else(|| "'?'".to_owned(), python_repr);
                        errors.push(format!(
                            "Node {index} (id={id}) missing required field '{field}'"
                        ));
                    }
                }
                if let Some(id) = node.get("id") {
                    if let Some(key) = hashable_json(id) {
                        node_ids.insert(key);
                    } else {
                        errors.push(format!(
                            "Node {index} has non-hashable id {} - id must be a string",
                            python_repr(id)
                        ));
                    }
                }
                if let Some(file_type) = node.get("file_type") {
                    let valid = file_type
                        .as_str()
                        .is_some_and(|value| VALID_FILE_TYPES.contains(&value));
                    if !valid {
                        errors.push(format!(
                            "Node {index} (id={}) has invalid file_type '{}' - must be one of {}",
                            node.get("id").map_or_else(|| "'?'".to_owned(), python_repr),
                            python_string(file_type),
                            python_string_list(&VALID_FILE_TYPES)
                        ));
                    }
                }
            }
        }
        Some(_) => errors.push("'nodes' must be a list".to_owned()),
    }

    let edge_list = if data.contains_key("edges") {
        data.get("edges")
    } else {
        data.get("links")
    };
    match edge_list {
        None | Some(Value::Null) => errors.push("Missing required key 'edges'".to_owned()),
        Some(Value::Array(edges)) => {
            for (index, edge) in edges.iter().enumerate() {
                let Some(edge) = edge.as_object() else {
                    errors.push(format!("Edge {index} must be an object"));
                    continue;
                };
                for field in REQUIRED_EDGE_FIELDS {
                    if !edge.contains_key(field) {
                        errors.push(format!("Edge {index} missing required field '{field}'"));
                    }
                }
                if let Some(confidence) = edge.get("confidence") {
                    let valid = confidence
                        .as_str()
                        .is_some_and(|value| VALID_CONFIDENCES.contains(&value));
                    if !valid {
                        errors.push(format!(
                            "Edge {index} has invalid confidence '{}' - must be one of {}",
                            python_string(confidence),
                            python_string_list(&VALID_CONFIDENCES)
                        ));
                    }
                }
                for endpoint in ["source", "target"] {
                    let Some(value) = edge.get(endpoint) else {
                        continue;
                    };
                    // Python short-circuits before hashing malformed endpoints
                    // when no valid node id has been collected.
                    if node_ids.is_empty() {
                        continue;
                    }
                    let Some(key) = hashable_json(value) else {
                        errors.push(format!(
                            "Edge {index} {endpoint} {} is non-hashable - must be a string",
                            python_repr(value)
                        ));
                        continue;
                    };
                    if !node_ids.contains(&key) {
                        errors.push(format!(
                            "Edge {index} {endpoint} '{}' does not match any node id",
                            python_string(value)
                        ));
                    }
                }
            }
        }
        Some(_) => errors.push("'edges' must be a list".to_owned()),
    }
    errors
}

/// Return an aggregated validation error when an extraction is invalid.
pub fn assert_valid_extraction(data: &Value) -> Result<(), ExtractionValidationError> {
    let errors = validate_extraction(data);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(ExtractionValidationError { errors })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtractionValidationError {
    pub errors: Vec<String>,
}

impl std::fmt::Display for ExtractionValidationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            formatter,
            "Extraction JSON has {} error(s):",
            self.errors.len()
        )?;
        for (index, error) in self.errors.iter().enumerate() {
            write!(formatter, "  • {error}")?;
            if index + 1 != self.errors.len() {
                formatter.write_char('\n')?;
            }
        }
        Ok(())
    }
}

impl std::error::Error for ExtractionValidationError {}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RecordValidationErrors {
    pub id: String,
    pub errors: Vec<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CodeGraphValidationReport {
    pub document_errors: Vec<String>,
    pub node_errors: Vec<RecordValidationErrors>,
    pub edge_errors: Vec<RecordValidationErrors>,
}

impl CodeGraphValidationReport {
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.document_errors.is_empty()
            && self.node_errors.is_empty()
            && self.edge_errors.is_empty()
    }

    fn into_errors(self) -> Vec<String> {
        let mut errors = self.document_errors;
        errors.extend(
            self.node_errors
                .into_iter()
                .flat_map(|record| record.errors),
        );
        errors.extend(
            self.edge_errors
                .into_iter()
                .flat_map(|record| record.errors),
        );
        errors
    }
}

/// Classify strict `compass.graph/1` validation failures by owning record.
#[must_use]
pub fn validate_code_graph_records(document: &CodeGraphDocument) -> CodeGraphValidationReport {
    let mut report = CodeGraphValidationReport::default();
    if !document.directed {
        report
            .document_errors
            .push("directed must be true".to_owned());
    }
    if !document.multigraph {
        report
            .document_errors
            .push("multigraph must be true".to_owned());
    }
    if document.graph.schema != CODE_GRAPH_SCHEMA_V1 {
        report.document_errors.push(format!(
            "graph.schema must be {CODE_GRAPH_SCHEMA_V1}, got {}",
            document.graph.schema
        ));
    }

    let mut files = HashMap::new();
    for file in &document.graph.files {
        if file.id.trim().is_empty() {
            report
                .document_errors
                .push("file ID must not be empty".to_owned());
        }
        if file.path.is_empty()
            || std::path::Path::new(&file.path).is_absolute()
            || file.path.contains('\\')
        {
            report.document_errors.push(format!(
                "file {} has a non-portable repository path",
                file.id
            ));
        }
        if files.insert(file.path.as_str(), file.byte_size).is_some() {
            report
                .document_errors
                .push(format!("duplicate file path {}", file.path));
        }
        if file.id != file_id(&file.path) {
            report.document_errors.push(format!(
                "file {} does not match its deterministic path identity",
                file.path
            ));
        }
    }

    let mut nodes = HashMap::new();
    let mut duplicate_node_positions = HashSet::new();
    for (index, node) in document.nodes.iter().enumerate() {
        if nodes.insert(node.id.as_str(), node).is_some() {
            duplicate_node_positions.insert(index);
        }
    }
    let validate_node = |(index, node): (usize, &crate::code_graph::NodeRecord)| {
        let mut errors = Vec::new();
        if node.id.trim().is_empty() {
            errors.push("node ID must not be empty".to_owned());
        }
        if duplicate_node_positions.contains(&index) {
            errors.push(format!("duplicate node ID {}", node.id));
        }
        if node.name.trim().is_empty() || node.qualified_name.trim().is_empty() {
            errors.push(format!(
                "node {} requires non-empty name and qualifiedName",
                node.id
            ));
        }
        if node.evidence.is_empty() {
            errors.push(format!("node {} has no provenance", node.id));
        }
        validate_evidence(&node.id, &node.evidence, &files, &mut errors);
        if let Some(anchor) = &node.source {
            validate_anchor(&node.id, anchor, &files, &mut errors);
        }
        if !details_match_kind(node.kind, node.details.as_ref()) {
            errors.push(format!(
                "node {} has details incompatible with kind {}",
                node.id,
                node.kind.as_str()
            ));
        }
        if !errors.is_empty() {
            Some(RecordValidationErrors {
                id: node.id.clone(),
                errors,
            })
        } else {
            None
        }
    };
    let node_errors = if document.nodes.len() < 512 {
        document
            .nodes
            .iter()
            .enumerate()
            .map(validate_node)
            .collect::<Vec<_>>()
    } else {
        document
            .nodes
            .par_iter()
            .enumerate()
            .map(validate_node)
            .collect::<Vec<_>>()
    };
    report.node_errors.extend(node_errors.into_iter().flatten());

    let mut edge_ids = HashSet::with_capacity(document.links.len());
    let duplicate_edge_positions = document
        .links
        .iter()
        .enumerate()
        .filter_map(|(index, edge)| (!edge_ids.insert(edge.id.as_str())).then_some(index))
        .collect::<HashSet<_>>();
    let edge_errors = document
        .links
        .par_iter()
        .enumerate()
        .map(|(index, edge)| {
            validate_code_graph_edge(
                edge,
                &nodes,
                &files,
                duplicate_edge_positions.contains(&index),
            )
        })
        .collect::<Vec<_>>();
    report.edge_errors.extend(edge_errors.into_iter().flatten());

    report
}

fn validate_code_graph_edge(
    edge: &crate::code_graph::EdgeRecord,
    nodes: &HashMap<&str, &crate::code_graph::NodeRecord>,
    files: &HashMap<&str, u64>,
    duplicate_id: bool,
) -> Option<RecordValidationErrors> {
    let mut errors = Vec::new();
    if edge
        .occurrence_rule
        .as_ref()
        .is_some_and(|rule| rule.is_endpoint_rewrite())
    {
        errors.push(format!(
            "edge {} occurrence rule uses a reserved endpoint rewrite name",
            edge.id
        ));
    }
    if edge
        .occurrence_rule
        .as_ref()
        .is_some_and(|rule| rule.as_str().trim().is_empty())
    {
        errors.push(format!("edge {} has an empty occurrence rule", edge.id));
    }
    if edge.id.trim().is_empty() || edge.id != edge.key {
        errors.push(format!(
            "edge {} must have a non-empty id matching its NetworkX key",
            edge.id
        ));
    }
    let expected_id = edge_id(
        &edge.source,
        edge.kind,
        &edge.target,
        edge.relationship_site.as_ref(),
        edge.occurrence_rule.as_ref().map(|rule| rule.as_str()),
    );
    if edge.id != expected_id {
        errors.push(format!(
            "edge {} does not match its deterministic relationship identity",
            edge.id
        ));
    }
    if duplicate_id {
        errors.push(format!("duplicate edge ID {}", edge.id));
    }
    let Some(source) = nodes.get(edge.source.as_str()) else {
        errors.push(format!(
            "edge {} source {} does not match a node",
            edge.id, edge.source
        ));
        return Some(RecordValidationErrors {
            id: edge.id.clone(),
            errors,
        });
    };
    let Some(target) = nodes.get(edge.target.as_str()) else {
        errors.push(format!(
            "edge {} target {} does not match a node",
            edge.id, edge.target
        ));
        return Some(RecordValidationErrors {
            id: edge.id.clone(),
            errors,
        });
    };
    if edge.source == edge.target && edge.kind != EdgeKind::Calls {
        errors.push(format!("edge {} is an unsupported self-loop", edge.id));
    }
    if edge.evidence.is_empty() {
        errors.push(format!("edge {} has no provenance", edge.id));
    }
    validate_evidence(&edge.id, &edge.evidence, files, &mut errors);
    if let Some(anchor) = &edge.relationship_site {
        validate_anchor(&edge.id, anchor, files, &mut errors);
    }
    if !endpoint_kinds_are_valid(source, edge.kind, target) {
        let site = edge.relationship_site.as_ref().map_or_else(
            || "<none>".to_owned(),
            |anchor| {
                format!(
                    "{}:{}:{}",
                    anchor.file, anchor.start_line, anchor.start_column
                )
            },
        );
        errors.push(format!(
            "edge {} has invalid {} endpoints {} -> {}; source={} target={} site={}",
            edge.id,
            edge.kind.as_str(),
            source.kind.as_str(),
            target.kind.as_str(),
            source.qualified_name,
            target.qualified_name,
            site
        ));
    }
    (!errors.is_empty()).then(|| RecordValidationErrors {
        id: edge.id.clone(),
        errors,
    })
}

/// Validate all cross-record invariants of a strict `compass.graph/1` document.
pub fn validate_code_graph(document: &CodeGraphDocument) -> Result<(), CodeGraphValidationError> {
    let report = validate_code_graph_records(document);
    if report.is_valid() {
        return Ok(());
    }
    let errors = report.into_errors();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(CodeGraphValidationError { errors })
    }
}

fn validate_evidence(
    owner: &str,
    evidence: &[Provenance],
    files: &HashMap<&str, u64>,
    errors: &mut Vec<String>,
) {
    for item in evidence {
        if let Err(error) = item.validate() {
            errors.push(format!("{owner}: {error}"));
        }
        for anchor in &item.anchors {
            validate_anchor(owner, anchor, files, errors);
        }
        if let Some(anchor) = &item.wiring_site {
            validate_anchor(owner, anchor, files, errors);
        }
        for candidate in &item.candidates {
            if let Some(anchor) = &candidate.anchor {
                validate_anchor(owner, anchor, files, errors);
            }
        }
    }
}

fn validate_anchor(
    owner: &str,
    anchor: &SourceAnchor,
    files: &HashMap<&str, u64>,
    errors: &mut Vec<String>,
) {
    if !anchor.is_valid() {
        errors.push(format!("{owner}: invalid source anchor"));
        return;
    }
    let Some(byte_size) = files.get(anchor.file.as_str()) else {
        errors.push(format!(
            "{owner}: source anchor references missing file {}",
            anchor.file
        ));
        return;
    };
    if anchor.end_byte > *byte_size {
        errors.push(format!(
            "{owner}: source anchor exceeds the recorded size of {}",
            anchor.file
        ));
    }
}

fn details_match_kind(kind: NodeKind, details: Option<&NodeDetails>) -> bool {
    match details {
        None => true,
        Some(NodeDetails::File(_)) => kind == NodeKind::File,
        Some(NodeDetails::Symbol(_)) => matches!(
            kind,
            NodeKind::Module
                | NodeKind::Package
                | NodeKind::Namespace
                | NodeKind::Class
                | NodeKind::Struct
                | NodeKind::Interface
                | NodeKind::Trait
                | NodeKind::Protocol
                | NodeKind::Enum
                | NodeKind::EnumMember
                | NodeKind::TypeAlias
                | NodeKind::Function
                | NodeKind::Method
                | NodeKind::Constructor
                | NodeKind::Property
                | NodeKind::Field
                | NodeKind::Variable
                | NodeKind::Constant
                | NodeKind::Parameter
                | NodeKind::Macro
                | NodeKind::Annotation
                | NodeKind::Migration
        ),
        Some(NodeDetails::ImportExport(_)) => {
            matches!(kind, NodeKind::Import | NodeKind::Export)
        }
        Some(NodeDetails::Route(_)) => kind == NodeKind::Route,
        Some(NodeDetails::Component(_)) => kind == NodeKind::Component,
        Some(NodeDetails::Resource(_)) => kind == NodeKind::Resource,
        Some(NodeDetails::Messaging(_)) => matches!(
            kind,
            NodeKind::Event | NodeKind::Message | NodeKind::Topic | NodeKind::Queue
        ),
        Some(NodeDetails::Job(_)) => kind == NodeKind::Job,
        Some(NodeDetails::Schema(_)) => kind == NodeKind::Schema,
        Some(NodeDetails::Query(_)) => kind == NodeKind::Query,
        Some(NodeDetails::Config(_)) => kind == NodeKind::ConfigKey,
        Some(NodeDetails::Database(_)) => matches!(
            kind,
            NodeKind::Database
                | NodeKind::DatabaseSchema
                | NodeKind::DatabaseTable
                | NodeKind::DatabaseView
                | NodeKind::DatabaseColumn
                | NodeKind::DatabaseIndex
                | NodeKind::DatabaseConstraint
                | NodeKind::DatabaseProcedure
                | NodeKind::DatabaseTrigger
        ),
    }
}

fn endpoint_kinds_are_valid(
    source: &crate::code_graph::NodeRecord,
    kind: EdgeKind,
    target: &crate::code_graph::NodeRecord,
) -> bool {
    match kind {
        EdgeKind::Contains => contains_endpoint_pair(source.kind, target.kind),
        EdgeKind::Embeds => source.kind.is_type() && target.kind.is_type(),
        EdgeKind::Calls => {
            is_call_source(source.kind)
                && (target.kind.is_callable()
                    || matches!(
                        target.kind,
                        NodeKind::Variable
                            | NodeKind::Property
                            | NodeKind::Import
                            | NodeKind::TypeAlias
                    ))
        }
        EdgeKind::Imports => {
            ((matches!(
                source.kind,
                NodeKind::File
                    | NodeKind::Module
                    | NodeKind::Package
                    | NodeKind::Namespace
                    | NodeKind::Import
            ) || source.kind.is_callable()
                || source.kind.is_type())
                && is_import_target(target.kind))
                || (source.kind == NodeKind::ConfigKey && target.kind == NodeKind::Resource)
        }
        EdgeKind::Exports => {
            (source.kind.is_container() || source.kind == NodeKind::Export)
                && is_export_target(target.kind)
        }
        EdgeKind::Extends => source.kind.is_type() && target.kind.is_type(),
        EdgeKind::Implements => {
            (source.kind.is_type()
                || (source.kind == NodeKind::Parameter
                    && source.language.as_deref() == Some("rust")))
                && matches!(
                    target.kind,
                    NodeKind::Interface
                        | NodeKind::Trait
                        | NodeKind::Protocol
                        // TypeScript permits a class to implement a
                        // structural object type declared through a type
                        // alias.
                        | NodeKind::TypeAlias
                )
        }
        EdgeKind::TypeOf => {
            is_typed_value(source.kind)
                && (target.kind.is_type() || target.kind == NodeKind::Parameter)
        }
        EdgeKind::Returns => source.kind.is_callable() && is_return_target(target.kind),
        EdgeKind::Instantiates => {
            is_call_source(source.kind)
                && (target.kind.is_constructible()
                    || (target.kind == NodeKind::EnumMember
                        && target.language.as_deref() == Some("rust")))
        }
        EdgeKind::Overrides => source.kind.is_callable() && target.kind.is_callable(),
        EdgeKind::Decorates => {
            matches!(source.kind, NodeKind::Annotation | NodeKind::Macro)
                && is_decoratable(target.kind)
        }
        EdgeKind::RoutesTo => {
            source.kind == NodeKind::Route
                && matches!(
                    target.kind,
                    NodeKind::File
                        | NodeKind::Function
                        | NodeKind::Method
                        | NodeKind::Class
                        | NodeKind::Component
                )
        }
        EdgeKind::MapsTo => {
            matches!(
                source.kind,
                NodeKind::Class
                    | NodeKind::Struct
                    | NodeKind::Schema
                    | NodeKind::DatabaseTable
                    | NodeKind::DatabaseView
            ) && matches!(
                target.kind,
                NodeKind::DatabaseTable | NodeKind::DatabaseView
            )
        }
        EdgeKind::Reads => {
            (is_executable(source.kind) && is_data(target.kind))
                || (source.kind == NodeKind::DatabaseView && target.kind == NodeKind::DatabaseTable)
        }
        EdgeKind::Writes => is_executable(source.kind) && is_data(target.kind),
        EdgeKind::Aliases => {
            matches!(
                source.kind,
                NodeKind::Import | NodeKind::Export | NodeKind::TypeAlias
            ) && is_alias_target(target.kind)
        }
        EdgeKind::Registers => {
            (is_executable(source.kind) || source.kind.is_container())
                && is_registration_target(target.kind)
        }
        EdgeKind::Handles => {
            is_executable(source.kind)
                && matches!(
                    target.kind,
                    NodeKind::Event | NodeKind::Message | NodeKind::Topic | NodeKind::Queue
                )
        }
        EdgeKind::Publishes | EdgeKind::Produces => {
            is_executable(source.kind)
                && matches!(
                    target.kind,
                    NodeKind::Event | NodeKind::Message | NodeKind::Topic | NodeKind::Queue
                )
        }
        EdgeKind::Subscribes | EdgeKind::Consumes => {
            matches!(
                source.kind,
                NodeKind::Function
                    | NodeKind::Method
                    | NodeKind::Component
                    | NodeKind::Job
                    | NodeKind::Queue
            ) && matches!(
                target.kind,
                NodeKind::Event | NodeKind::Message | NodeKind::Topic | NodeKind::Queue
            )
        }
        EdgeKind::Schedules | EdgeKind::Triggers => {
            (is_executable(source.kind)
                && matches!(
                    target.kind,
                    NodeKind::Function
                        | NodeKind::Method
                        | NodeKind::Job
                        | NodeKind::Event
                        | NodeKind::DatabaseTrigger
                ))
                || (kind == EdgeKind::Triggers
                    && source.kind == NodeKind::DatabaseTrigger
                    && target.kind == NodeKind::DatabaseTable)
        }
        EdgeKind::Tests => {
            matches!(
                source.kind,
                NodeKind::File | NodeKind::Function | NodeKind::Method | NodeKind::Class
            ) && source.roles.contains(&NodeRole::Test)
                && is_test_target(target.kind)
        }
        EdgeKind::Documents => {
            source.kind == NodeKind::Resource && is_documentable_target(target.kind)
        }
        EdgeKind::References => {
            is_reference_source(source.kind) && is_reference_target(target.kind)
        }
        EdgeKind::DependsOn => {
            is_dependency_endpoint(source.kind) && is_dependency_endpoint(target.kind)
        }
    }
}

const fn contains_endpoint_pair(source: NodeKind, target: NodeKind) -> bool {
    if matches!(
        (source, target),
        (NodeKind::Enum, NodeKind::EnumMember) | (NodeKind::Schema, NodeKind::ConfigKey)
    ) {
        return true;
    }
    if matches!(source, NodeKind::EnumMember | NodeKind::Field)
        && matches!(target, NodeKind::Method | NodeKind::Field)
    {
        return true;
    }
    matches!(
        (source, target),
        (
            NodeKind::File,
            NodeKind::Module
                | NodeKind::Package
                | NodeKind::Namespace
                | NodeKind::Class
                | NodeKind::Struct
                | NodeKind::Interface
                | NodeKind::Trait
                | NodeKind::Protocol
                | NodeKind::Enum
                | NodeKind::TypeAlias
                | NodeKind::Function
                | NodeKind::Method
                | NodeKind::Constructor
                | NodeKind::Property
                | NodeKind::Field
                | NodeKind::Variable
                | NodeKind::Constant
                | NodeKind::Parameter
                | NodeKind::Import
                | NodeKind::Export
                | NodeKind::Macro
                | NodeKind::Annotation
                | NodeKind::Route
                | NodeKind::Component
                | NodeKind::Resource
                | NodeKind::Event
                | NodeKind::Message
                | NodeKind::Topic
                | NodeKind::Queue
                | NodeKind::Job
                | NodeKind::Schema
                | NodeKind::Query
                | NodeKind::Migration
                | NodeKind::ConfigKey
                | NodeKind::Database
        ) | (
            NodeKind::Module | NodeKind::Package | NodeKind::Namespace,
            NodeKind::File
                | NodeKind::Module
                | NodeKind::Package
                | NodeKind::Namespace
                | NodeKind::Class
                | NodeKind::Struct
                | NodeKind::Interface
                | NodeKind::Trait
                | NodeKind::Protocol
                | NodeKind::Enum
                | NodeKind::TypeAlias
                | NodeKind::Function
                | NodeKind::Method
                | NodeKind::Constructor
                | NodeKind::Property
                | NodeKind::Field
                | NodeKind::Variable
                | NodeKind::Constant
                | NodeKind::Parameter
                | NodeKind::Import
                | NodeKind::Export
                | NodeKind::Macro
                | NodeKind::Annotation
                | NodeKind::Route
                | NodeKind::Component
                | NodeKind::Resource
                | NodeKind::Event
                | NodeKind::Message
                | NodeKind::Topic
                | NodeKind::Queue
                | NodeKind::Job
                | NodeKind::Schema
                | NodeKind::Query
                | NodeKind::Migration
                | NodeKind::ConfigKey
        ) | (
            NodeKind::Class
                | NodeKind::Struct
                | NodeKind::Interface
                | NodeKind::Trait
                | NodeKind::Protocol
                | NodeKind::Enum
                | NodeKind::Component
                | NodeKind::Schema,
            NodeKind::Class
                | NodeKind::Struct
                | NodeKind::Interface
                | NodeKind::Trait
                | NodeKind::Protocol
                | NodeKind::Enum
                | NodeKind::TypeAlias
                | NodeKind::Function
                | NodeKind::Method
                | NodeKind::Constructor
                | NodeKind::Property
                | NodeKind::Field
                | NodeKind::Variable
                | NodeKind::Constant
                | NodeKind::Parameter
                | NodeKind::Macro
                | NodeKind::Annotation
                | NodeKind::Component
        ) | (
            NodeKind::Function | NodeKind::Method | NodeKind::Constructor | NodeKind::TypeAlias,
            NodeKind::Class
                | NodeKind::Struct
                | NodeKind::Interface
                | NodeKind::Trait
                | NodeKind::Protocol
                | NodeKind::Enum
                | NodeKind::TypeAlias
                | NodeKind::Function
                | NodeKind::Method
                | NodeKind::Constructor
                | NodeKind::Property
                | NodeKind::Field
                | NodeKind::Variable
                | NodeKind::Constant
                | NodeKind::Parameter
                | NodeKind::Macro
        ) | (
            NodeKind::Resource,
            NodeKind::File | NodeKind::Resource | NodeKind::ConfigKey
        ) | (NodeKind::Schema, NodeKind::ConfigKey)
            | (NodeKind::ConfigKey, NodeKind::ConfigKey)
    ) || database_contains(source, target)
}

const fn database_contains(source: NodeKind, target: NodeKind) -> bool {
    matches!(
        (source, target),
        (
            NodeKind::Database,
            NodeKind::DatabaseSchema
                | NodeKind::DatabaseTable
                | NodeKind::DatabaseView
                | NodeKind::DatabaseIndex
                | NodeKind::DatabaseTrigger
        ) | (
            NodeKind::DatabaseSchema,
            NodeKind::DatabaseTable
                | NodeKind::DatabaseView
                | NodeKind::DatabaseProcedure
                | NodeKind::DatabaseTrigger
        ) | (
            NodeKind::DatabaseTable | NodeKind::DatabaseView,
            NodeKind::DatabaseColumn
                | NodeKind::DatabaseIndex
                | NodeKind::DatabaseConstraint
                | NodeKind::DatabaseTrigger
        ) | (NodeKind::File, NodeKind::Database)
    )
}

const fn is_typed_value(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::Property
            | NodeKind::Field
            | NodeKind::Variable
            | NodeKind::Constant
            | NodeKind::Parameter
            | NodeKind::Import
            | NodeKind::Export
            | NodeKind::TypeAlias
    )
}

const fn is_decoratable(kind: NodeKind) -> bool {
    kind.is_callable()
        || kind.is_type()
        || matches!(
            kind,
            NodeKind::Property
                | NodeKind::Field
                | NodeKind::Variable
                | NodeKind::Constant
                | NodeKind::Parameter
                | NodeKind::Component
                | NodeKind::Route
                | NodeKind::Resource
        )
}

const fn is_test_target(kind: NodeKind) -> bool {
    kind.is_callable()
        || kind.is_type()
        || kind.is_container()
        || matches!(
            kind,
            NodeKind::Route
                | NodeKind::Component
                | NodeKind::Event
                | NodeKind::Message
                | NodeKind::Topic
                | NodeKind::Queue
                | NodeKind::Job
                | NodeKind::Query
                | NodeKind::Migration
                | NodeKind::EnumMember
                | NodeKind::DatabaseProcedure
                | NodeKind::DatabaseTrigger
        )
}

const fn is_documentable_target(kind: NodeKind) -> bool {
    kind.is_callable()
        || kind.is_type()
        || kind.is_container()
        || matches!(
            kind,
            NodeKind::Route
                | NodeKind::Component
                | NodeKind::Event
                | NodeKind::Message
                | NodeKind::Topic
                | NodeKind::Queue
                | NodeKind::Job
                | NodeKind::Schema
                | NodeKind::Query
                | NodeKind::Migration
                | NodeKind::DatabaseProcedure
                | NodeKind::DatabaseTrigger
        )
}

const fn is_call_source(kind: NodeKind) -> bool {
    kind.is_callable()
        || kind.is_type()
        || matches!(
            kind,
            NodeKind::File
                | NodeKind::Module
                | NodeKind::Variable
                | NodeKind::Field
                | NodeKind::Constant
                | NodeKind::EnumMember
        )
}

const fn is_import_target(kind: NodeKind) -> bool {
    kind.is_container()
        || kind.is_callable()
        || kind.is_type()
        || matches!(
            kind,
            NodeKind::Import
                | NodeKind::Export
                | NodeKind::TypeAlias
                | NodeKind::Variable
                | NodeKind::Field
                | NodeKind::Constant
                | NodeKind::EnumMember
                | NodeKind::Annotation
                | NodeKind::Macro
                | NodeKind::Resource
                | NodeKind::ConfigKey
        )
}

const fn is_export_target(kind: NodeKind) -> bool {
    kind.is_container()
        || kind.is_callable()
        || kind.is_type()
        || matches!(
            kind,
            NodeKind::Import
                | NodeKind::Export
                | NodeKind::TypeAlias
                | NodeKind::Variable
                | NodeKind::Constant
                | NodeKind::Macro
        )
}

const fn is_return_target(kind: NodeKind) -> bool {
    kind.is_type()
        || matches!(
            kind,
            NodeKind::TypeAlias
                | NodeKind::Parameter
                | NodeKind::Variable
                | NodeKind::Import
                | NodeKind::Schema
                | NodeKind::DatabaseTable
                | NodeKind::DatabaseView
        )
}

const fn is_alias_target(kind: NodeKind) -> bool {
    kind.is_callable()
        || kind.is_type()
        || matches!(
            kind,
            NodeKind::Import
                | NodeKind::Export
                | NodeKind::TypeAlias
                | NodeKind::Variable
                | NodeKind::Constant
        )
}

const fn is_registration_target(kind: NodeKind) -> bool {
    kind.is_callable()
        || kind.is_container()
        || matches!(
            kind,
            NodeKind::Component
                | NodeKind::Route
                | NodeKind::Event
                | NodeKind::Message
                | NodeKind::Topic
                | NodeKind::Queue
                | NodeKind::Job
        )
}

const fn is_reference_source(kind: NodeKind) -> bool {
    kind.is_container()
        || kind.is_callable()
        || kind.is_type()
        || matches!(
            kind,
            NodeKind::File
                | NodeKind::Property
                | NodeKind::Field
                | NodeKind::Variable
                | NodeKind::Constant
                | NodeKind::Parameter
                | NodeKind::EnumMember
                | NodeKind::Import
                | NodeKind::Export
                | NodeKind::Annotation
                | NodeKind::Macro
                | NodeKind::TypeAlias
                | NodeKind::Resource
                | NodeKind::Schema
                | NodeKind::Query
                | NodeKind::ConfigKey
                | NodeKind::DatabaseTable
                | NodeKind::DatabaseView
                | NodeKind::DatabaseColumn
                | NodeKind::DatabaseProcedure
                | NodeKind::DatabaseTrigger
        )
}

const fn is_reference_target(kind: NodeKind) -> bool {
    kind.is_container()
        || kind.is_callable()
        || kind.is_type()
        || matches!(
            kind,
            NodeKind::File
                | NodeKind::Property
                | NodeKind::Field
                | NodeKind::Variable
                | NodeKind::Constant
                | NodeKind::EnumMember
                | NodeKind::Parameter
                | NodeKind::Import
                | NodeKind::Export
                | NodeKind::Annotation
                | NodeKind::Macro
                | NodeKind::TypeAlias
                | NodeKind::Resource
                | NodeKind::Schema
                | NodeKind::Query
                | NodeKind::ConfigKey
                | NodeKind::Database
                | NodeKind::DatabaseSchema
                | NodeKind::DatabaseTable
                | NodeKind::DatabaseView
                | NodeKind::DatabaseColumn
                | NodeKind::DatabaseIndex
                | NodeKind::DatabaseConstraint
                | NodeKind::DatabaseProcedure
                | NodeKind::DatabaseTrigger
        )
}

const fn is_dependency_endpoint(kind: NodeKind) -> bool {
    kind.is_container()
        || kind.is_callable()
        || kind.is_type()
        || matches!(
            kind,
            NodeKind::File
                | NodeKind::Import
                | NodeKind::Export
                | NodeKind::TypeAlias
                | NodeKind::Resource
                | NodeKind::Schema
                | NodeKind::Query
                | NodeKind::ConfigKey
                | NodeKind::Database
                | NodeKind::DatabaseSchema
                | NodeKind::DatabaseTable
                | NodeKind::DatabaseView
                | NodeKind::DatabaseProcedure
                | NodeKind::DatabaseTrigger
        )
}

const fn is_executable(kind: NodeKind) -> bool {
    kind.is_callable()
        || matches!(
            kind,
            NodeKind::Component | NodeKind::Job | NodeKind::Query | NodeKind::DatabaseTrigger
        )
}

const fn is_data(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::Property
            | NodeKind::Field
            | NodeKind::Variable
            | NodeKind::Constant
            | NodeKind::Parameter
            | NodeKind::Resource
            | NodeKind::Schema
            | NodeKind::Query
            | NodeKind::ConfigKey
            | NodeKind::Database
            | NodeKind::DatabaseSchema
            | NodeKind::DatabaseTable
            | NodeKind::DatabaseView
            | NodeKind::DatabaseColumn
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodeGraphValidationError {
    pub errors: Vec<String>,
}

impl std::fmt::Display for CodeGraphValidationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "Compass graph v1 has {} validation error(s)",
            self.errors.len()
        )?;
        for error in &self.errors {
            write!(formatter, "\n  • {error}")?;
        }
        Ok(())
    }
}

impl std::error::Error for CodeGraphValidationError {}

fn hashable_json(value: &Value) -> Option<HashableJson> {
    match value {
        Value::Null => Some(HashableJson::None),
        Value::Bool(value) => Some(HashableJson::Integer(i128::from(*value))),
        Value::String(value) => Some(HashableJson::String(value.clone())),
        Value::Number(value) => {
            if let Some(integer) = value.as_i64() {
                Some(HashableJson::Integer(i128::from(integer)))
            } else if let Some(integer) = value.as_u64() {
                Some(HashableJson::Integer(i128::from(integer)))
            } else {
                let number = value.as_f64()?;
                if number == 0.0 {
                    Some(HashableJson::Integer(0))
                } else if number.fract() == 0.0
                    && number >= i128::MIN as f64
                    && number <= i128::MAX as f64
                {
                    Some(HashableJson::Integer(number as i128))
                } else {
                    Some(HashableJson::Float(number.to_bits()))
                }
            }
        }
        Value::Array(_) | Value::Object(_) => None,
    }
}

fn python_string_list(values: &[&str]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(|value| python_repr(&Value::String((*value).to_owned())))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn python_string(value: &Value) -> String {
    match value {
        Value::Null => "None".to_owned(),
        Value::Bool(value) => if *value { "True" } else { "False" }.to_owned(),
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        Value::Array(_) | Value::Object(_) => python_repr(value),
    }
}

fn python_repr(value: &Value) -> String {
    match value {
        Value::Null => "None".to_owned(),
        Value::Bool(value) => if *value { "True" } else { "False" }.to_owned(),
        Value::String(value) => {
            let escaped = value
                .chars()
                .flat_map(|character| match character {
                    '\\' => "\\\\".chars().collect::<Vec<_>>(),
                    '\'' => "\\'".chars().collect::<Vec<_>>(),
                    '\n' => "\\n".chars().collect::<Vec<_>>(),
                    '\r' => "\\r".chars().collect::<Vec<_>>(),
                    '\t' => "\\t".chars().collect::<Vec<_>>(),
                    character if character.is_control() => {
                        format!("\\x{:02x}", character as u32).chars().collect()
                    }
                    character => vec![character],
                })
                .collect::<String>();
            format!("'{escaped}'")
        }
        Value::Number(value) => value.to_string(),
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(python_repr)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Object(values) => format!(
            "{{{}}}",
            values
                .iter()
                .map(|(key, value)| format!(
                    "{}: {}",
                    python_repr(&Value::String(key.clone())),
                    python_repr(value)
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn accepts_links_and_python_numeric_id_equality() {
        let extraction = json!({
            "nodes":[{"id":true,"label":"x","file_type":"code","source_file":"x"}],
            "links":[{"source":1,"target":1.0,"relation":"x","confidence":"EXTRACTED","source_file":"x"}]
        });
        assert!(validate_extraction(&extraction).is_empty());
    }

    #[test]
    fn aggregate_error_matches_python_shape() -> Result<(), Box<dyn std::error::Error>> {
        let Err(error) = assert_valid_extraction(&json!({"nodes":"bad","edges":[]})) else {
            return Err("invalid extraction unexpectedly passed".into());
        };
        assert_eq!(
            error.to_string(),
            "Extraction JSON has 1 error(s):\n  • 'nodes' must be a list"
        );
        Ok(())
    }
}
