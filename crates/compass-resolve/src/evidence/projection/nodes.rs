//! Declaration, external, and deferred node projection and merging.

use super::*;

pub(super) fn declaration_node(
    declaration: &DeclarationFact,
    definition_range: Option<&EvidenceRange>,
    graph_node_id: &str,
) -> NodeRecord {
    let label = match declaration.kind.as_str() {
        "function" => format!("{}()", declaration.name),
        "method" => format!(".{}()", declaration.name),
        _ => declaration.name.clone(),
    };
    let callable = matches!(declaration.kind.as_str(), "function" | "method");
    // The universal evidence model keeps TypeScript's type-parameter kind
    // explicit for resolution, while the public graph v1 contract represents
    // all generic parameters as `parameter` nodes.
    let graph_kind = match declaration.kind.as_str() {
        "type_parameter" | "lifetime_parameter" | "const_parameter" => "parameter",
        kind => kind,
    };
    let mut attributes = Map::from_iter([
        ("label".to_owned(), Value::String(label)),
        (
            "qualified_name".to_owned(),
            Value::String(declaration.qualified_name.clone()),
        ),
        (
            "symbol_kind".to_owned(),
            Value::String(graph_kind.to_owned()),
        ),
        ("file_type".to_owned(), Value::String("code".to_owned())),
        (
            "source_file".to_owned(),
            Value::String(declaration.range.source_file.clone()),
        ),
        (
            "source_location".to_owned(),
            Value::String(format!("L{}", declaration.range.start_line)),
        ),
        (
            "start_byte".to_owned(),
            Value::from(declaration.range.start_byte),
        ),
        (
            "end_byte".to_owned(),
            Value::from(declaration.range.end_byte),
        ),
        (
            "line_start".to_owned(),
            Value::from(declaration.range.start_line),
        ),
        (
            "line_end".to_owned(),
            Value::from(declaration.range.end_line),
        ),
        (
            "column_start".to_owned(),
            Value::from(declaration.range.start_column),
        ),
        (
            "column_end".to_owned(),
            Value::from(declaration.range.end_column),
        ),
        (
            "language".to_owned(),
            Value::String(declaration.language.clone()),
        ),
        (
            "extractor".to_owned(),
            Value::String(format!(
                "compass.languages.{}.universal",
                declaration.language
            )),
        ),
        (
            "confidence".to_owned(),
            Value::String("EXTRACTED".to_owned()),
        ),
        ("_origin".to_owned(), Value::String("ast".to_owned())),
        (
            "evidence_declaration_id".to_owned(),
            Value::String(declaration.id.clone()),
        ),
    ]);
    if let Some(legacy_qualified_name) = legacy_callable_qualified_name(declaration) {
        attributes.insert(
            "legacy_qualified_name".to_owned(),
            Value::String(legacy_qualified_name),
        );
    }
    if callable {
        attributes.insert("_callable".to_owned(), Value::Bool(true));
    }
    for (key, value) in [
        ("signature", declaration.signature.as_ref()),
        ("signature_hash", declaration.signature_hash.as_ref()),
        (
            "implementation_hash",
            declaration.implementation_hash.as_ref(),
        ),
        ("source_hash", declaration.source_hash.as_ref()),
    ] {
        if let Some(value) = value {
            attributes.insert(key.to_owned(), Value::String(value.clone()));
        }
    }
    if let Some(module_or_package) = declaration.module_or_package.as_ref() {
        attributes.insert(
            "module".to_owned(),
            Value::String(module_or_package.clone()),
        );
    }
    if let Some(definition_range) = definition_range.filter(|range| *range != &declaration.range) {
        attributes.insert(
            "source_anchor".to_owned(),
            source_anchor_value(definition_range),
        );
        attributes.insert(
            NODE_PROVENANCE_ANCHOR_ATTRIBUTE.to_owned(),
            source_anchor_value(&declaration.range),
        );
    }
    NodeRecord {
        id: graph_node_id.to_owned(),
        attributes,
    }
}

/// Preserve the public callable spelling emitted by the pre-universal
/// TypeScript/JavaScript extractor. Universal evidence keeps its richer module
/// qualified name for resolution, while framework route contracts continue to
/// identify source callables as `name()@offset` (or `Owner::name()@offset`).
pub(super) fn legacy_callable_qualified_name(declaration: &DeclarationFact) -> Option<String> {
    let start = declaration
        .definition_start_byte
        .unwrap_or(declaration.range.start_byte);
    match declaration.kind.as_str() {
        "function" => {
            if declaration.name == "default" {
                Some("default".to_owned())
            } else {
                Some(format!("{}()@{start}", declaration.name))
            }
        }
        "method" => {
            let mut parts = declaration.qualified_name.split('.');
            let _module = parts.next()?;
            let owner_parts = parts.collect::<Vec<_>>();
            if owner_parts.len() < 2 {
                return Some(format!("{}()@{start}", declaration.name));
            }
            let (method, owner) = owner_parts.split_last()?;
            Some(format!("{}::{}()@{start}", owner.join("::"), method))
        }
        _ => None,
    }
}

pub(super) fn source_anchor_value(range: &EvidenceRange) -> Value {
    serde_json::json!({
        "file": range.source_file,
        "startByte": range.start_byte,
        "endByte": range.end_byte,
        "startLine": range.start_line,
        "startColumn": range.start_column,
        "endLine": range.end_line,
        "endColumn": range.end_column,
    })
}

pub(super) fn relation_name(relation: CandidateRelation) -> &'static str {
    match relation {
        CandidateRelation::Calls | CandidateRelation::Constructs => "calls",
        CandidateRelation::IndirectCalls => "indirect_call",
        CandidateRelation::Decorates => "references",
        CandidateRelation::Annotates | CandidateRelation::References => "references",
        CandidateRelation::TypeOf => "type_of",
        CandidateRelation::Returns => "returns",
        CandidateRelation::Extends => "inherits",
        CandidateRelation::Implements => "implements",
        CandidateRelation::Overrides => "overrides",
        CandidateRelation::AccessesMember => "accesses",
        CandidateRelation::Contains | CandidateRelation::Owns => "contains",
        CandidateRelation::Embeds => "embeds",
        CandidateRelation::Imports => "imports_from",
        CandidateRelation::Reexports => "re_exports",
        CandidateRelation::InvokesMacro => "references",
        CandidateRelation::Tests => "tests",
    }
}

pub(super) fn external_node(
    id: &str,
    qualified_name: &str,
    language: &str,
    candidate: &RelationshipCandidate,
) -> NodeRecord {
    let kind = external_kind(candidate);
    let role = relation_name(candidate.relation);
    let attributes = Map::from_iter([
        (
            "label".to_owned(),
            Value::String(
                qualified_name
                    .rsplit(['.', '/'])
                    .next()
                    .unwrap_or(qualified_name)
                    .to_owned(),
            ),
        ),
        (
            "qualified_name".to_owned(),
            Value::String(qualified_name.to_owned()),
        ),
        ("symbol_kind".to_owned(), Value::String(kind.to_owned())),
        ("file_type".to_owned(), Value::String("code".to_owned())),
        ("source_file".to_owned(), Value::String(String::new())),
        ("source_location".to_owned(), Value::String(String::new())),
        ("language".to_owned(), Value::String(language.to_owned())),
        ("external_role".to_owned(), Value::String(role.to_owned())),
        (
            "external_roles".to_owned(),
            Value::Array(vec![Value::String(role.to_owned())]),
        ),
        (
            "extractor".to_owned(),
            Value::String(format!("compass.resolve.{language}.universal")),
        ),
        (
            "confidence".to_owned(),
            Value::String(
                if candidate.binding_id.is_some() {
                    "EXTRACTED"
                } else {
                    "INFERRED"
                }
                .to_owned(),
            ),
        ),
        ("external".to_owned(), Value::Bool(true)),
        ("placeholder".to_owned(), Value::Bool(true)),
        ("_canonical_external_symbol".to_owned(), Value::Bool(true)),
    ]);
    NodeRecord {
        id: id.to_owned(),
        attributes,
    }
}

pub(super) fn deferred_receiver_node(
    id: &str,
    qualified_name: &str,
    language: &str,
    candidate: &RelationshipCandidate,
) -> NodeRecord {
    let kind = external_kind(candidate);
    NodeRecord {
        id: id.to_owned(),
        attributes: Map::from_iter([
            (
                "label".to_owned(),
                Value::String(
                    qualified_name
                        .rsplit([':', '.'])
                        .find(|component| !component.is_empty())
                        .unwrap_or(qualified_name)
                        .to_owned(),
                ),
            ),
            (
                "qualified_name".to_owned(),
                Value::String(qualified_name.to_owned()),
            ),
            ("symbol_kind".to_owned(), Value::String(kind.to_owned())),
            ("file_type".to_owned(), Value::String("code".to_owned())),
            ("source_file".to_owned(), Value::String(String::new())),
            ("source_location".to_owned(), Value::String(String::new())),
            ("language".to_owned(), Value::String(language.to_owned())),
            (
                "extractor".to_owned(),
                Value::String(format!("compass.resolve.{language}.universal")),
            ),
            (
                "confidence".to_owned(),
                Value::String("INFERRED".to_owned()),
            ),
            ("external".to_owned(), Value::Bool(false)),
            ("placeholder".to_owned(), Value::Bool(true)),
            ("deferred_receiver".to_owned(), Value::Bool(true)),
            (
                "deferred_role".to_owned(),
                Value::String(relation_name(candidate.relation).to_owned()),
            ),
        ]),
    }
}

pub(in crate::evidence) fn is_deferred_receiver(qualifier: &str) -> bool {
    !qualifier.contains("::") && !qualifier.contains('/')
}

pub(super) fn merge_external_node(node: &mut NodeRecord, candidate: &RelationshipCandidate) {
    let incoming_kind = external_kind(candidate);
    let current_kind = node.string("symbol_kind");
    if external_kind_rank(incoming_kind) > external_kind_rank(&current_kind) {
        node.attributes.insert(
            "symbol_kind".to_owned(),
            Value::String(incoming_kind.to_owned()),
        );
        node.attributes.insert(
            "external_role".to_owned(),
            Value::String(relation_name(candidate.relation).to_owned()),
        );
    }
    let incoming_role = relation_name(candidate.relation);
    let mut roles = node
        .attributes
        .get("external_roles")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    roles.insert(incoming_role.to_owned());
    node.attributes.insert(
        "external_roles".to_owned(),
        Value::Array(roles.into_iter().map(Value::String).collect()),
    );
}

pub(super) fn external_kind(candidate: &RelationshipCandidate) -> &'static str {
    match candidate.relation {
        CandidateRelation::Imports => "import",
        CandidateRelation::Reexports => "export",
        CandidateRelation::Extends => "class",
        CandidateRelation::Annotates
        | CandidateRelation::Embeds
        | CandidateRelation::TypeOf
        | CandidateRelation::Returns => "type_alias",
        CandidateRelation::Implements => "interface",
        CandidateRelation::Overrides => "function",
        CandidateRelation::AccessesMember => "variable",
        CandidateRelation::Calls | CandidateRelation::IndirectCalls => "function",
        CandidateRelation::Constructs => "class",
        CandidateRelation::Decorates => "function",
        CandidateRelation::References => reference_external_kind(candidate),
        CandidateRelation::Contains | CandidateRelation::Owns => "variable",
        CandidateRelation::InvokesMacro => "macro",
        CandidateRelation::Tests => "function",
    }
}

pub(super) fn reference_external_kind(candidate: &RelationshipCandidate) -> &'static str {
    let allowed = &candidate.constraints.allowed_target_kinds;
    if allowed.is_empty() {
        return "variable";
    }
    let type_only = allowed.iter().all(|kind| {
        matches!(
            kind.as_str(),
            "class" | "struct" | "enum" | "interface" | "trait" | "type_alias" | "parameter"
        )
    });
    if !type_only {
        return "variable";
    }
    if allowed
        .iter()
        .all(|kind| matches!(kind.as_str(), "interface" | "trait" | "parameter"))
        && allowed
            .iter()
            .any(|kind| matches!(kind.as_str(), "interface" | "trait"))
    {
        "interface"
    } else {
        "type_alias"
    }
}

pub(super) fn external_kind_rank(kind: &str) -> u8 {
    match kind {
        "interface" => 7,
        "class" => 6,
        "type_alias" => 5,
        "function" => 4,
        "variable" => 3,
        "import" => 2,
        "export" => 1,
        _ => 0,
    }
}
