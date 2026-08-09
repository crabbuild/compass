use std::collections::BTreeMap;

use crate::{NodeIndex, NodeRecord, canonical_code_token, identifier_tokens};

/// Per-field occurrence counts for one term in one graph node document.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LexicalTermFrequency {
    pub label: u16,
    pub identifier: u16,
    pub kind: u16,
    pub source: u16,
}

impl LexicalTermFrequency {
    #[must_use]
    pub fn total(self) -> u32 {
        u32::from(self.label)
            .saturating_add(u32::from(self.identifier))
            .saturating_add(u32::from(self.kind))
            .saturating_add(u32::from(self.source))
    }
}

/// One deterministic term posting into the graph's node vector.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LexicalPosting {
    pub node: NodeIndex,
    pub frequency: LexicalTermFrequency,
}

/// Lazy, graph-derived lexical statistics used by bounded query retrieval.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LexicalIndex {
    postings: BTreeMap<String, Vec<LexicalPosting>>,
    document_lengths: Vec<u32>,
    average_document_length: f64,
}

impl LexicalIndex {
    pub(crate) fn build(nodes: &[NodeRecord]) -> Self {
        let mut postings = BTreeMap::<String, Vec<LexicalPosting>>::new();
        let mut document_lengths = Vec::with_capacity(nodes.len());
        let mut total_document_length = 0_u64;

        for (node_index, node) in nodes.iter().enumerate() {
            let mut terms = BTreeMap::<String, LexicalTermFrequency>::new();
            add_field_terms(&mut terms, node.label(), LexicalField::Label);
            add_field_terms(&mut terms, &node.id, LexicalField::Identifier);
            if let Some(qualified_name) = node
                .logical_property("qualified_name")
                .and_then(|value| value.as_str().map(str::to_owned))
            {
                add_field_terms(&mut terms, &qualified_name, LexicalField::Identifier);
            }
            add_field_terms(&mut terms, node.kind_name(), LexicalField::Kind);
            if let Some(source_file) = node.source_file() {
                add_field_terms(&mut terms, source_file, LexicalField::Source);
            }

            let document_length = terms
                .values()
                .map(|frequency| frequency.total())
                .fold(0_u32, u32::saturating_add);
            document_lengths.push(document_length);
            total_document_length =
                total_document_length.saturating_add(u64::from(document_length));

            for (term, frequency) in terms {
                postings.entry(term).or_default().push(LexicalPosting {
                    node: node_index,
                    frequency,
                });
            }
        }

        let average_document_length = if nodes.is_empty() {
            0.0
        } else {
            total_document_length as f64 / nodes.len() as f64
        };
        Self {
            postings,
            document_lengths,
            average_document_length,
        }
    }

    #[must_use]
    pub fn postings(&self, term: &str) -> &[LexicalPosting] {
        self.postings.get(term).map_or(&[], Vec::as_slice)
    }

    #[must_use]
    pub fn document_length(&self, node: NodeIndex) -> Option<u32> {
        self.document_lengths.get(node).copied()
    }

    #[must_use]
    pub fn document_count(&self) -> usize {
        self.document_lengths.len()
    }

    #[must_use]
    pub fn term_count(&self) -> usize {
        self.postings.len()
    }

    #[must_use]
    pub fn average_document_length(&self) -> f64 {
        self.average_document_length
    }
}

#[derive(Clone, Copy)]
enum LexicalField {
    Label,
    Identifier,
    Kind,
    Source,
}

fn add_field_terms(
    terms: &mut BTreeMap<String, LexicalTermFrequency>,
    value: &str,
    field: LexicalField,
) {
    for token in identifier_tokens(value) {
        add_term(terms, canonical_code_token(token.clone()), field);
        if token.contains('_') {
            for component in token.split('_').filter(|component| !component.is_empty()) {
                add_term(terms, canonical_code_token(component.to_owned()), field);
            }
        }
    }
}

fn add_term(terms: &mut BTreeMap<String, LexicalTermFrequency>, term: String, field: LexicalField) {
    if term.is_empty()
        || (term.chars().all(|character| character.is_ascii_lowercase())
            && term.chars().count() <= 2)
    {
        return;
    }
    let frequency = terms.entry(term).or_default();
    let slot = match field {
        LexicalField::Label => &mut frequency.label,
        LexicalField::Identifier => &mut frequency.identifier,
        LexicalField::Kind => &mut frequency.kind,
        LexicalField::Source => &mut frequency.source,
    };
    *slot = slot.saturating_add(1);
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::{Graph, LexicalTermFrequency};

    #[test]
    fn graph_lexical_index_is_lazy_deterministic_and_field_aware() {
        let document = serde_json::from_value(json!({
            "directed": true,
            "multigraph": false,
            "graph": {},
            "nodes": [{
                "id": "n:route-register",
                "label": "route_register",
                "qualifiedName": "api::RouteRegister",
                "kind": "function",
                "source_file": "src/routes.py"
            }, {
                "id": "n:dependency-solve",
                "label": "solveDependencies",
                "kind": "method",
                "source_file": "src/dependencies.py"
            }],
            "links": []
        }))
        .unwrap_or_else(|_| std::process::abort());
        let graph = Graph::from_document(document).unwrap_or_else(|_| std::process::abort());
        let first = graph.lexical_index();
        let second = graph.lexical_index();

        assert!(std::ptr::eq(first, second));
        assert_eq!(first.document_count(), 2);
        assert!(first.term_count() > 4);
        assert!(first.average_document_length().is_finite());
        assert!(first.average_document_length() > 0.0);
        assert!(first.document_length(0).is_some());
        assert_eq!(first.document_length(99), None);
        assert_eq!(
            first.postings("route")[0].frequency,
            LexicalTermFrequency {
                label: 1,
                identifier: 2,
                kind: 0,
                source: 1,
            }
        );
        assert_eq!(first.postings("dependency")[0].node, 1);

        let cloned = graph.clone();
        assert!(std::ptr::eq(first, cloned.lexical_index()));
    }
}
