use compass_model::code_graph::{DiagnosticSeverity, GraphDiagnostic, GraphDocument};
use compass_model::provenance::SourceAnchor;

pub const MAX_QUARANTINE_EXAMPLES: usize = 100;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PublicationOmissions {
    pub nodes: usize,
    pub edges: usize,
    pub identity_collisions: usize,
    pub examples_omitted: usize,
}

impl PublicationOmissions {
    #[must_use]
    pub const fn is_partial(self) -> bool {
        self.nodes > 0 || self.edges > 0 || self.identity_collisions > 0
    }
}

#[derive(Clone, Debug)]
pub struct PublicationOutcome {
    pub document: GraphDocument,
    pub omissions: PublicationOmissions,
}

#[derive(Default)]
pub(crate) struct QuarantineCollector {
    omissions: PublicationOmissions,
    diagnostics: Vec<GraphDiagnostic>,
    node_examples: usize,
    edge_examples: usize,
    collision_examples: usize,
}

impl QuarantineCollector {
    pub(crate) fn omit_node(&mut self, identity: &str, reason: &str, anchor: Option<SourceAnchor>) {
        self.omissions.nodes = self.omissions.nodes.saturating_add(1);
        if self.node_examples < MAX_QUARANTINE_EXAMPLES {
            self.node_examples += 1;
            self.diagnostics.push(GraphDiagnostic {
                severity: DiagnosticSeverity::Warning,
                code: "publication_omitted_node".to_owned(),
                message: format!("omitted node {identity}: {reason}"),
                anchor,
                related_ids: Vec::new(),
            });
        } else {
            self.omissions.examples_omitted = self.omissions.examples_omitted.saturating_add(1);
        }
    }

    pub(crate) fn omit_edge(&mut self, identity: &str, reason: &str, anchor: Option<SourceAnchor>) {
        self.omissions.edges = self.omissions.edges.saturating_add(1);
        if self.edge_examples < MAX_QUARANTINE_EXAMPLES {
            self.edge_examples += 1;
            self.diagnostics.push(GraphDiagnostic {
                severity: DiagnosticSeverity::Warning,
                code: "publication_omitted_edge".to_owned(),
                message: format!("omitted edge {identity}: {reason}"),
                anchor,
                related_ids: Vec::new(),
            });
        } else {
            self.omissions.examples_omitted = self.omissions.examples_omitted.saturating_add(1);
        }
    }

    pub(crate) fn identity_collision(
        &mut self,
        identity: &str,
        reason: &str,
        anchor: Option<SourceAnchor>,
    ) {
        self.omissions.nodes = self.omissions.nodes.saturating_add(1);
        self.omissions.identity_collisions = self.omissions.identity_collisions.saturating_add(1);
        if self.collision_examples < MAX_QUARANTINE_EXAMPLES {
            self.collision_examples += 1;
            self.diagnostics.push(GraphDiagnostic {
                severity: DiagnosticSeverity::Warning,
                code: "publication_identity_collision".to_owned(),
                message: format!("omitted conflicting node {identity}: {reason}"),
                anchor,
                related_ids: Vec::new(),
            });
        } else {
            self.omissions.examples_omitted = self.omissions.examples_omitted.saturating_add(1);
        }
    }

    pub(crate) fn finish(mut self, diagnostics: &mut Vec<GraphDiagnostic>) -> PublicationOmissions {
        if self.omissions.is_partial() {
            self.diagnostics.push(GraphDiagnostic {
                severity: DiagnosticSeverity::Warning,
                code: "publication_omission_summary".to_owned(),
                message: format!(
                    "partial graph published after quarantining {} nodes and {} edges with {} identity collisions; {} examples omitted by the diagnostic cap",
                    self.omissions.nodes,
                    self.omissions.edges,
                    self.omissions.identity_collisions,
                    self.omissions.examples_omitted
                ),
                anchor: None,
                related_ids: Vec::new(),
            });
        }
        diagnostics.append(&mut self.diagnostics);
        self.omissions
    }
}
