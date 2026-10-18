//! Source-proven leading documentation; no spelling or proximity resolution.
use super::*;
use compass_model::provenance::{
    LEADING_DOCUMENTATION_OWNER_ANCHOR_ATTRIBUTE, LEADING_DOCUMENTATION_RULE,
};

impl UniversalResolutionIndex {
    pub(super) fn materialize_leading_documentation(
        &self,
        nodes: &mut Vec<NodeRecord>,
        edges: &mut Vec<EdgeRecord>,
        graph_ids: &AHashMap<String, String>,
    ) {
        let mut declarations = self
            .facts
            .declarations
            .values()
            .filter(|declaration| declaration.documentation.is_some())
            .collect::<Vec<_>>();
        if declarations.is_empty() {
            return;
        }
        declarations.sort_unstable_by(|left, right| left.id.cmp(&right.id));
        let mut present = nodes
            .iter()
            .map(|node| node.id.clone())
            .collect::<AHashSet<_>>();
        for declaration in declarations {
            let (Some(documentation), Some(definition), Some(owner_id)) = (
                &declaration.documentation,
                self.facts.definition_ranges.get(&declaration.id),
                graph_ids.get(&declaration.id),
            ) else {
                continue;
            };
            if !present.contains(owner_id) {
                continue;
            }
            let owner_range = published_declaration_source_range(declaration, Some(definition));
            let range = &documentation.range;
            if range.source_file != owner_range.source_file
                || range.end_byte > owner_range.start_byte
                || (range.end_line, range.end_column)
                    > (owner_range.start_line, owner_range.start_column)
            {
                continue;
            }
            let identity = format!("{}:{}", range.start_byte, range.end_byte);
            let id = make_id(&[
                LEADING_DOCUMENTATION_RULE,
                owner_id,
                &range.source_file,
                &identity,
            ]);
            if !present.insert(id.clone()) {
                continue;
            }
            let mut attributes = Map::from_iter([
                (
                    "label".to_owned(),
                    Value::String(format!("Documentation for {}", declaration.name)),
                ),
                (
                    "qualified_name".to_owned(),
                    Value::String(format!(
                        "{}::leading_documentation@{identity}",
                        declaration.qualified_name
                    )),
                ),
                (
                    "symbol_kind".to_owned(),
                    Value::String("resource".to_owned()),
                ),
                (
                    "file_type".to_owned(),
                    Value::String("rationale".to_owned()),
                ),
                (
                    "resource_kind".to_owned(),
                    Value::String("rationale".to_owned()),
                ),
                (
                    "resource_content".to_owned(),
                    Value::String(documentation.content.clone()),
                ),
                (
                    "source_file".to_owned(),
                    Value::String(range.source_file.clone()),
                ),
                ("source_anchor".to_owned(), source_anchor_value(range)),
                (
                    "source_location".to_owned(),
                    Value::String(format!("L{}", range.start_line)),
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
            ]);
            nodes.push(NodeRecord {
                id: id.clone(),
                attributes: attributes.clone(),
            });
            attributes.remove("label");
            attributes.remove("qualified_name");
            attributes.remove("symbol_kind");
            attributes.remove("file_type");
            attributes.remove("resource_kind");
            attributes.remove("resource_content");
            attributes.insert("relation".to_owned(), Value::String("documents".to_owned()));
            attributes.insert(
                "rule".to_owned(),
                Value::String(LEADING_DOCUMENTATION_RULE.to_owned()),
            );
            attributes.insert(
                OCCURRENCE_RULE_ATTRIBUTE.to_owned(),
                Value::String(LEADING_DOCUMENTATION_RULE.to_owned()),
            );
            attributes.insert(
                LEADING_DOCUMENTATION_OWNER_ANCHOR_ATTRIBUTE.to_owned(),
                source_anchor_value(owner_range),
            );
            edges.push(EdgeRecord {
                source: id,
                target: owner_id.clone(),
                attributes,
            });
        }
    }
}

/// A bounded summary travels with the graph instead of disappearing when the
/// transient per-file evidence batches are consumed by the resolver.
pub(crate) fn leading_documentation_diagnostics(
    batches: &[SemanticEvidenceBatch],
) -> Vec<compass_model::code_graph::GraphDiagnostic> {
    use compass_languages::DECLARATION_DOCUMENTATION_OMISSION_CODE;
    use compass_model::code_graph::{DiagnosticSeverity, GraphDiagnostic};
    let extraction_omissions = batches
        .iter()
        .flat_map(|batch| &batch.diagnostics)
        .filter(|diagnostic| diagnostic.code == DECLARATION_DOCUMENTATION_OMISSION_CODE)
        .count();
    let declarations = batches
        .iter()
        .flat_map(|batch| &batch.declarations)
        .filter(|declaration| declaration.documentation.is_some())
        .collect::<Vec<_>>();
    let ranges = batch_definition_ranges(batches, &declarations);
    let projection_omissions = declarations
        .iter()
        .filter(|declaration| {
            let Some(documentation) = &declaration.documentation else {
                return false;
            };
            ranges.get(&declaration.id).is_none_or(|definition| {
                documentation.range.source_file != definition.source_file
                    || documentation.range.end_byte > definition.start_byte
                    || (documentation.range.end_line, documentation.range.end_column)
                        > (definition.start_line, definition.start_column)
            })
        })
        .count();
    if extraction_omissions == 0 && projection_omissions == 0 {
        return Vec::new();
    }
    vec![GraphDiagnostic {
        severity: DiagnosticSeverity::Warning,
        code: "leading_documentation_omission_summary".to_owned(),
        message: format!(
            "Leading documentation is incomplete: {extraction_omissions} extraction warning records (at most one per file) report excerpt/attachment limits; {projection_omissions} excerpts lack a unique, non-overlapping declaration definition range. Structural declarations are retained. Shorten oversized groups or use a supported contiguous AST attachment; ambiguous definition ownership requires extractor qualification."
        ),
        anchor: None,
        related_ids: Vec::new(),
    }]
}
