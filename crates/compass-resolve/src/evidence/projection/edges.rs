//! Relationship edge construction, metadata, anchors, and provenance.

use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn materialized_edge(
    source: String,
    target: String,
    relation: &str,
    candidate: &RelationshipCandidate,
    occurrence: Option<OccurrenceRef<'_>>,
    binding: Option<&BindingFact>,
    target_kind: Option<&str>,
    target_source_file: Option<&str>,
    range: &compass_languages::EvidenceRange,
    resolution_rule: ResolutionRule,
    language: &str,
    project_metadata: Option<&BTreeMap<String, String>>,
) -> EdgeRecord {
    let context = match (relation, resolution_rule) {
        ("calls", ResolutionRule::QualifiedExternal) => "external_call",
        ("calls", ResolutionRule::DeferredReceiver) => "deferred_receiver_call",
        ("calls", _) => "call",
        ("indirect_call", _) => occurrence
            .and_then(OccurrenceRef::context)
            .unwrap_or("reference"),
        ("references", _) if candidate.relation == CandidateRelation::Decorates => "decorator",
        ("decorates", _) => "decorator",
        ("references", _)
            if occurrence.is_some_and(|occurrence| {
                occurrence.role() == compass_languages::SemanticRole::CallableReference
            }) =>
        {
            occurrence
                .and_then(OccurrenceRef::context)
                .unwrap_or("reference")
        }
        ("imports_from", _)
            if candidate.relation == CandidateRelation::Imports && target_kind == Some("file") =>
        {
            "submodule_import"
        }
        ("imports_from", _) => occurrence
            .and_then(OccurrenceRef::context)
            .unwrap_or("import"),
        ("re_exports", _) => "export",
        ("inherits", _) => "base_type",
        ("references", _) => "type_reference",
        ("embeds", _) => "embedding",
        ("method", _) => "receiver",
        _ => "",
    };
    let confidence = if matches!(
        resolution_rule,
        ResolutionRule::QualifiedExternal
            | ResolutionRule::DeferredReceiver
            | ResolutionRule::ClosedWorldReceiverDispatch
            | ResolutionRule::IncompleteHierarchyReceiverDispatch
    ) {
        "INFERRED"
    } else {
        "EXTRACTED"
    };
    let producer_rule = format!(
        "universal-{}-{}",
        candidate_relation_name(candidate.relation),
        resolution_rule_name(resolution_rule)
    );
    let occurrence_rule = binding.map_or_else(
        || producer_rule.clone(),
        |binding| {
            // Import aliases can share one statement anchor and one resolved
            // endpoint. Keep those occurrences distinct without using the
            // candidate ID, whose absolute byte position changes when the
            // statement moves. Binding offsets within the statement remain
            // portable across checkouts and source relocation.
            format!(
                "{producer_rule}:binding:{}:{}:{}",
                binding.spelling,
                binding.range.start_byte.saturating_sub(range.start_byte),
                binding.range.end_byte.saturating_sub(range.start_byte)
            )
        },
    );
    let mut attributes = Map::from_iter([
        ("relation".to_owned(), Value::String(relation.to_owned())),
        ("_origin".to_owned(), Value::String("ast".to_owned())),
        (
            "confidence".to_owned(),
            Value::String(confidence.to_owned()),
        ),
        (
            "source_file".to_owned(),
            Value::String(range.source_file.clone()),
        ),
        (
            "source_location".to_owned(),
            Value::String(format!("L{}", range.start_line)),
        ),
        ("start_byte".to_owned(), Value::from(range.start_byte)),
        ("end_byte".to_owned(), Value::from(range.end_byte)),
        ("line_start".to_owned(), Value::from(range.start_line)),
        ("line_end".to_owned(), Value::from(range.end_line)),
        ("column_start".to_owned(), Value::from(range.start_column)),
        ("column_end".to_owned(), Value::from(range.end_column)),
        ("weight".to_owned(), Value::from(1.0)),
        ("language".to_owned(), Value::String(language.to_owned())),
        (
            "extractor".to_owned(),
            Value::String(format!("compass.resolve.{language}.universal")),
        ),
        (
            "resolution_rule".to_owned(),
            Value::String(resolution_rule_name(resolution_rule).to_owned()),
        ),
        ("rule".to_owned(), Value::String(producer_rule)),
        (
            OCCURRENCE_RULE_ATTRIBUTE.to_owned(),
            Value::String(occurrence_rule),
        ),
    ]);
    // Candidate and occurrence IDs are resolver-internal lookup identities.
    // At this point their relation, occurrence rule, exact anchor, endpoints,
    // and provenance have already been projected into the public edge. Do not
    // duplicate those long IDs in every transient JSON attribute map. The
    // separate universal project-edge builder retains candidate IDs until its
    // own deduplication pass because that path still consumes them.
    if matches!(
        candidate.relation,
        CandidateRelation::Imports | CandidateRelation::Reexports
    ) {
        if let Some(binding) = binding {
            attributes.insert(
                "local_name".to_owned(),
                Value::String(binding.spelling.clone()),
            );
            attributes.insert(
                "imported_name".to_owned(),
                Value::String(candidate.target_spelling.clone()),
            );
            attributes.insert(
                "qualified_target".to_owned(),
                Value::String(binding.qualified_target.clone()),
            );
            attributes.insert(
                "binding_kind".to_owned(),
                Value::String(binding_kind_name(binding.kind).to_owned()),
            );
        }
        let module = candidate
            .constraints
            .module_or_package
            .clone()
            .or_else(|| {
                binding
                    .and_then(|binding| binding.qualified_target.rsplit_once("::"))
                    .map(|(module, _)| module.to_owned())
            })
            .or_else(|| {
                project_metadata
                    .and_then(|metadata| metadata.get("project_module"))
                    .cloned()
            });
        if let Some(module) = module {
            attributes.insert(
                "module".to_owned(),
                Value::String(import_module_for_edge(
                    language,
                    &range.source_file,
                    &module,
                )),
            );
        }
    }
    if !context.is_empty() {
        attributes.insert("context".to_owned(), Value::String(context.to_owned()));
    }
    if let Some(target_source_file) = target_source_file {
        attributes.insert(
            "target_file".to_owned(),
            Value::String(target_source_file.to_owned()),
        );
    }
    if let Some(project_metadata) = project_metadata {
        for (name, value) in project_metadata {
            if name == "project_module" {
                continue;
            }
            let metadata_value = if name == "resolution_project_references" {
                serde_json::from_str(value).unwrap_or_else(|_| Value::String(value.clone()))
            } else {
                Value::String(value.clone())
            };
            attributes.insert(format!("project_{name}"), metadata_value.clone());
            if matches!(
                name.as_str(),
                "package_condition"
                    | "resolution_config"
                    | "module_resolution"
                    | "module_kind"
                    | "resolution_project_references"
            ) {
                attributes.insert(name.clone(), metadata_value);
            }
        }
    }
    if let Some(binding) = binding {
        attributes.extend([
            (
                "binding_name".to_owned(),
                Value::String(binding.spelling.clone()),
            ),
            (
                "binding_qualified_target".to_owned(),
                Value::String(binding.qualified_target.clone()),
            ),
        ]);
    }
    EdgeRecord {
        source,
        target,
        attributes,
    }
}

pub(super) fn binding_kind_name(kind: compass_languages::BindingKind) -> &'static str {
    match kind {
        compass_languages::BindingKind::Import => "import",
        compass_languages::BindingKind::ImportAlias => "import_alias",
        compass_languages::BindingKind::Reexport => "reexport",
        compass_languages::BindingKind::LocalAlias => "local_alias",
        compass_languages::BindingKind::CallResult => "call_result",
        compass_languages::BindingKind::Package => "package",
        compass_languages::BindingKind::Member => "member",
    }
}

pub(super) fn import_module_for_edge(language: &str, source_file: &str, module: &str) -> String {
    if language != "python" {
        return module.to_owned();
    }
    let source_package = Path::new(source_file)
        .parent()
        .map(|parent| {
            parent
                .components()
                .filter_map(|component| component.as_os_str().to_str())
                .filter(|component| !component.is_empty() && *component != ".")
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let module_components = module
        .split('.')
        .filter(|component| !component.is_empty())
        .collect::<Vec<_>>();
    let shared = source_package
        .iter()
        .zip(&module_components)
        .take_while(|(left, right)| left == right)
        .count();
    if shared == 0 {
        return module.to_owned();
    }
    let upward = source_package.len().saturating_sub(shared);
    let suffix = module_components[shared..].join(".");
    format!("{}{}", ".".repeat(upward.saturating_add(1)), suffix)
}

const fn candidate_relation_name(relation: CandidateRelation) -> &'static str {
    match relation {
        CandidateRelation::Calls => "call",
        CandidateRelation::IndirectCalls => "indirect-call",
        CandidateRelation::Constructs => "construction",
        CandidateRelation::Decorates => "decorator",
        CandidateRelation::Annotates => "annotation",
        CandidateRelation::Extends => "extends",
        CandidateRelation::Implements => "implements",
        CandidateRelation::UsesTrait => "trait-use",
        CandidateRelation::Overrides => "override",
        CandidateRelation::References => "reference",
        CandidateRelation::TypeOf => "type-of",
        CandidateRelation::Returns => "return-type",
        CandidateRelation::AccessesMember => "member-access",
        CandidateRelation::Contains => "contains",
        CandidateRelation::Owns => "owns",
        CandidateRelation::Embeds => "embedding",
        CandidateRelation::Imports => "import",
        CandidateRelation::Reexports => "reexport",
        CandidateRelation::InvokesMacro => "macro-invocation",
        CandidateRelation::Tests => "test-call",
    }
}

pub(super) const fn resolution_rule_name(rule: ResolutionRule) -> &'static str {
    match rule {
        ResolutionRule::ExactSourceDeclaration => "exact-source-declaration",
        ResolutionRule::ExactLexicalDeclaration => "exact-lexical-declaration",
        ResolutionRule::ExplicitBinding => "explicit-binding",
        ResolutionRule::ProjectModuleBinding => "project-module-binding",
        ResolutionRule::MemberBinding => "member-binding",
        ResolutionRule::DeferredReceiver => "deferred-receiver",
        ResolutionRule::WildcardBinding => "wildcard-binding",
        ResolutionRule::PhpGlobalFunctionFallback => "php-global-function-fallback",
        ResolutionRule::UniqueModuleOrPackage => "unique-module-or-package",
        ResolutionRule::ExactHierarchyBase => "exact-hierarchy-base",
        ResolutionRule::DirectReceiverSuccessorDispatch => "direct-receiver-successor-dispatch",
        ResolutionRule::LinearizedReceiverDispatch => "linearized-receiver-dispatch",
        ResolutionRule::ClosedWorldReceiverDispatch => "closed-world-receiver-dispatch",
        ResolutionRule::IncompleteHierarchyReceiverDispatch => {
            "incomplete-hierarchy-receiver-dispatch"
        }
        ResolutionRule::RustAssociatedType => "rust-associated-type",
        ResolutionRule::ExactSourceInventory => "exact-source-inventory",
        ResolutionRule::QualifiedExternal => "qualified-external",
    }
}

#[must_use]
pub(crate) fn is_replaced_relation(relation: &str) -> bool {
    matches!(
        relation,
        "contains"
            | "method"
            | "calls"
            | "indirect_call"
            | "imports"
            | "imports_from"
            | "re_exports"
            | "inherits"
            | "implements"
            | "references"
            | "embeds"
            | "decorated_by"
            | "owns"
            | "accesses"
    )
}
