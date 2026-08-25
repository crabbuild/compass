use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tree_sitter::Node;

use crate::{EXTRACTION_SEMANTICS_VERSION, UniversalEvidencePipeline, file_stem, make_id};

use super::model::{
    BindingFact, BindingKind, CandidateRelation, DeclarationFact, EvidenceDiagnostic,
    EvidenceRange, HierarchyConstraint, OccurrenceFact, ReceiverDispatchStrategy,
    RelationshipCandidate, ResolutionConstraint, ScopeFact, SemanticEvidenceBatch, SemanticRole,
    SymbolNamespace, UniversalEvidenceIdentity,
};
use super::validate::{EvidenceError, EvidenceErrorCode, EvidenceLimits, validate_evidence};

// Go selector attribution can cross a closure, a multi-return call, and a
// range expression before reaching the receiver type. Keep that traversal
// bounded, but allow the real-world chain without falling back to an
// unresolved method.
const GO_TYPE_INFERENCE_DEPTH_LIMIT: usize = 16;

/// Bounded direct-construction API shared by hard-cut language producers.
pub struct EvidenceBuilder {
    batch: SemanticEvidenceBatch,
    source_file: String,
    limits: EvidenceLimits,
}

#[derive(Default)]
struct DeclarationMetadata {
    signature: Option<String>,
    parameter_count: Option<u32>,
    parameter_types: Vec<String>,
    direct_bases_complete: bool,
    variadic: bool,
    signature_hash: Option<String>,
    implementation_hash: Option<String>,
    source_hash: Option<String>,
}

impl EvidenceBuilder {
    #[must_use]
    pub fn new(
        pipeline: &'static UniversalEvidencePipeline,
        emitter: impl Into<String>,
        source_file: impl Into<String>,
        limits: EvidenceLimits,
    ) -> Self {
        Self::new_with_dialect(pipeline, emitter, source_file, limits, None)
    }

    #[must_use]
    pub fn new_with_dialect(
        pipeline: &'static UniversalEvidencePipeline,
        emitter: impl Into<String>,
        source_file: impl Into<String>,
        limits: EvidenceLimits,
        dialect: Option<&str>,
    ) -> Self {
        Self {
            batch: SemanticEvidenceBatch {
                pipeline: UniversalEvidenceIdentity {
                    id: pipeline.producer.id.to_owned(),
                    language: pipeline.producer.language.to_owned(),
                    dialect: dialect.map(str::to_owned),
                    version: pipeline.producer.version,
                    evidence_schema: pipeline.producer.evidence_schema.to_owned(),
                    qualification: pipeline.qualification,
                    emitter: emitter.into(),
                    capabilities: pipeline.producer.capabilities.to_vec(),
                },
                declarations: Vec::new(),
                scopes: Vec::new(),
                bindings: Vec::new(),
                occurrences: Vec::new(),
                candidates: Vec::new(),
                diagnostics: Vec::new(),
            },
            source_file: source_file.into(),
            limits,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn declare(
        &mut self,
        kind: &str,
        graph_node_id: &str,
        name: &str,
        qualified_name: &str,
        module_or_package: Option<&str>,
        scope_id: Option<&str>,
        range: EvidenceRange,
    ) -> Result<String, EvidenceError> {
        self.declare_with_metadata(
            kind,
            graph_node_id,
            name,
            qualified_name,
            module_or_package,
            scope_id,
            range,
            DeclarationMetadata::default(),
        )
    }

    /// Add a declaration while retaining its source-level symbol space.
    #[allow(clippy::too_many_arguments)]
    pub fn declare_with_namespace(
        &mut self,
        kind: &str,
        graph_node_id: &str,
        name: &str,
        qualified_name: &str,
        module_or_package: Option<&str>,
        scope_id: Option<&str>,
        namespace: Option<SymbolNamespace>,
        range: EvidenceRange,
    ) -> Result<String, EvidenceError> {
        self.declare_with_metadata_and_namespace(
            kind,
            graph_node_id,
            name,
            qualified_name,
            module_or_package,
            scope_id,
            range,
            namespace,
            DeclarationMetadata::default(),
        )
    }

    /// Add a declaration with a bounded source-level signature fragment.
    ///
    /// Qualifying producers use this for type-shape facts that the shared
    /// resolver can consume without inventing a target from a terminal name.
    /// Keeping the entry point crate-private avoids expanding the public
    /// builder surface while allowing producers implemented in sibling
    /// modules to publish the existing, versioned `signature` field.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn declare_with_signature(
        &mut self,
        kind: &str,
        graph_node_id: &str,
        name: &str,
        qualified_name: &str,
        module_or_package: Option<&str>,
        scope_id: Option<&str>,
        namespace: Option<SymbolNamespace>,
        signature: Option<&str>,
        range: EvidenceRange,
    ) -> Result<String, EvidenceError> {
        let metadata = DeclarationMetadata {
            signature: signature.map(str::to_owned),
            ..DeclarationMetadata::default()
        };
        self.declare_with_metadata_and_namespace(
            kind,
            graph_node_id,
            name,
            qualified_name,
            module_or_package,
            scope_id,
            range,
            namespace,
            metadata,
        )
    }

    /// Add a callable declaration with the source-proven signature shape used
    /// by deterministic overload selection.
    ///
    /// Language producers retain responsibility for canonicalizing their own
    /// parameter spellings. The builder only publishes the bounded, typed
    /// fields already present in the universal evidence schema.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn declare_callable(
        &mut self,
        kind: &str,
        graph_node_id: &str,
        name: &str,
        qualified_name: &str,
        module_or_package: Option<&str>,
        scope_id: Option<&str>,
        namespace: Option<SymbolNamespace>,
        signature: Option<&str>,
        parameter_types: Vec<String>,
        variadic: bool,
        range: EvidenceRange,
    ) -> Result<String, EvidenceError> {
        let metadata = DeclarationMetadata {
            signature: signature.map(str::to_owned),
            parameter_count: Some(u32::try_from(parameter_types.len()).map_err(|_| {
                EvidenceError::new(
                    EvidenceErrorCode::ResourceLimit,
                    "callable parameter count exceeds the evidence schema limit",
                )
            })?),
            parameter_types,
            variadic,
            ..DeclarationMetadata::default()
        };
        self.declare_with_metadata_and_namespace(
            kind,
            graph_node_id,
            name,
            qualified_name,
            module_or_package,
            scope_id,
            range,
            namespace,
            metadata,
        )
    }

    /// Add a nominal type declaration and state whether its complete direct
    /// base list was parsed. This is shared resolver input, not a claim that
    /// every base target is locally resolvable.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn declare_type(
        &mut self,
        kind: &str,
        graph_node_id: &str,
        name: &str,
        qualified_name: &str,
        module_or_package: Option<&str>,
        scope_id: Option<&str>,
        namespace: Option<SymbolNamespace>,
        signature: Option<&str>,
        direct_bases_complete: bool,
        range: EvidenceRange,
    ) -> Result<String, EvidenceError> {
        let metadata = DeclarationMetadata {
            signature: signature.map(str::to_owned),
            direct_bases_complete,
            ..DeclarationMetadata::default()
        };
        self.declare_with_metadata_and_namespace(
            kind,
            graph_node_id,
            name,
            qualified_name,
            module_or_package,
            scope_id,
            range,
            namespace,
            metadata,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn declare_with_metadata(
        &mut self,
        kind: &str,
        graph_node_id: &str,
        name: &str,
        qualified_name: &str,
        module_or_package: Option<&str>,
        scope_id: Option<&str>,
        range: EvidenceRange,
        metadata: DeclarationMetadata,
    ) -> Result<String, EvidenceError> {
        self.declare_with_metadata_and_namespace(
            kind,
            graph_node_id,
            name,
            qualified_name,
            module_or_package,
            scope_id,
            range,
            None,
            metadata,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn declare_with_metadata_and_namespace(
        &mut self,
        kind: &str,
        graph_node_id: &str,
        name: &str,
        qualified_name: &str,
        module_or_package: Option<&str>,
        scope_id: Option<&str>,
        range: EvidenceRange,
        namespace: Option<SymbolNamespace>,
        metadata: DeclarationMetadata,
    ) -> Result<String, EvidenceError> {
        ensure_capacity(
            "declarations",
            self.batch.declarations.len(),
            self.limits.declarations,
        )?;
        let start_byte = range.start_byte.to_string();
        let end_byte = range.end_byte.to_string();
        let mut identity = vec![
            kind,
            graph_node_id,
            name,
            qualified_name,
            module_or_package.unwrap_or_default(),
            scope_id.unwrap_or_default(),
        ];
        if namespace.is_some() {
            identity.push(symbol_namespace_name(namespace));
        }
        identity.extend([start_byte.as_str(), end_byte.as_str()]);
        let id = self.stable_id("declaration", &identity);
        self.batch.declarations.push(DeclarationFact {
            id: id.clone(),
            language: self.batch.pipeline.language.clone(),
            graph_node_id: graph_node_id.to_owned(),
            kind: kind.to_owned(),
            name: name.to_owned(),
            qualified_name: qualified_name.to_owned(),
            namespace,
            module_or_package: module_or_package.map(str::to_owned),
            scope_id: scope_id.map(str::to_owned),
            signature: metadata.signature,
            parameter_count: metadata.parameter_count,
            parameter_types: metadata.parameter_types,
            direct_bases_complete: metadata.direct_bases_complete,
            variadic: metadata.variadic,
            signature_hash: metadata.signature_hash,
            implementation_hash: metadata.implementation_hash,
            source_hash: metadata.source_hash,
            definition_start_byte: None,
            range,
        });
        Ok(id)
    }

    pub fn open_scope(
        &mut self,
        kind: &str,
        owner_declaration_id: Option<&str>,
        parent_scope_id: Option<&str>,
        range: EvidenceRange,
    ) -> Result<String, EvidenceError> {
        ensure_capacity("scopes", self.batch.scopes.len(), self.limits.scopes)?;
        let id = self.stable_id(
            "scope",
            &[
                kind,
                owner_declaration_id.unwrap_or_default(),
                parent_scope_id.unwrap_or_default(),
                &range.start_byte.to_string(),
                &range.end_byte.to_string(),
            ],
        );
        self.batch.scopes.push(ScopeFact {
            id: id.clone(),
            language: self.batch.pipeline.language.clone(),
            kind: kind.to_owned(),
            owner_declaration_id: owner_declaration_id.map(str::to_owned),
            parent_scope_id: parent_scope_id.map(str::to_owned),
            range,
        });
        Ok(id)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn bind(
        &mut self,
        kind: BindingKind,
        spelling: &str,
        qualified_target: &str,
        target_declaration_id: Option<&str>,
        scope_id: Option<&str>,
        range: EvidenceRange,
    ) -> Result<String, EvidenceError> {
        self.bind_with_output_index(
            kind,
            spelling,
            qualified_target,
            target_declaration_id,
            scope_id,
            None,
            None,
            None,
            None,
            range,
        )
    }

    /// Add an import/re-export binding with explicit symbol-space identity.
    #[allow(clippy::too_many_arguments)]
    pub fn bind_with_identity(
        &mut self,
        kind: BindingKind,
        spelling: &str,
        qualified_target: &str,
        target_declaration_id: Option<&str>,
        scope_id: Option<&str>,
        namespace: Option<SymbolNamespace>,
        type_only: bool,
        range: EvidenceRange,
    ) -> Result<String, EvidenceError> {
        self.bind_with_output_index_and_identity(
            kind,
            spelling,
            qualified_target,
            target_declaration_id,
            scope_id,
            None,
            None,
            None,
            None,
            namespace,
            type_only,
            range,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn bind_chained_call_result(
        &mut self,
        spelling: &str,
        qualified_target: &str,
        result_type_qualified_name: Option<&str>,
        receiver_binding_id: Option<&str>,
        fallback_binding_id: Option<&str>,
        scope_id: Option<&str>,
        range: EvidenceRange,
    ) -> Result<String, EvidenceError> {
        self.bind_with_output_index(
            BindingKind::CallResult,
            spelling,
            qualified_target,
            None,
            scope_id,
            None,
            result_type_qualified_name,
            receiver_binding_id,
            fallback_binding_id,
            range,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn bind_with_output_index(
        &mut self,
        kind: BindingKind,
        spelling: &str,
        qualified_target: &str,
        target_declaration_id: Option<&str>,
        scope_id: Option<&str>,
        output_index: Option<u32>,
        result_type_qualified_name: Option<&str>,
        receiver_binding_id: Option<&str>,
        fallback_binding_id: Option<&str>,
        range: EvidenceRange,
    ) -> Result<String, EvidenceError> {
        self.bind_with_output_index_and_identity(
            kind,
            spelling,
            qualified_target,
            target_declaration_id,
            scope_id,
            output_index,
            result_type_qualified_name,
            receiver_binding_id,
            fallback_binding_id,
            None,
            false,
            range,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn bind_with_output_index_and_identity(
        &mut self,
        kind: BindingKind,
        spelling: &str,
        qualified_target: &str,
        target_declaration_id: Option<&str>,
        scope_id: Option<&str>,
        output_index: Option<u32>,
        result_type_qualified_name: Option<&str>,
        receiver_binding_id: Option<&str>,
        fallback_binding_id: Option<&str>,
        namespace: Option<SymbolNamespace>,
        type_only: bool,
        range: EvidenceRange,
    ) -> Result<String, EvidenceError> {
        ensure_capacity("bindings", self.batch.bindings.len(), self.limits.bindings)?;
        let output_index_text = output_index.map(|index| index.to_string());
        let start_byte = range.start_byte.to_string();
        let end_byte = range.end_byte.to_string();
        let mut identity = vec![
            binding_kind_name(kind),
            spelling,
            qualified_target,
            target_declaration_id.unwrap_or_default(),
            scope_id.unwrap_or_default(),
            output_index_text.as_deref().unwrap_or_default(),
            result_type_qualified_name.unwrap_or_default(),
            receiver_binding_id.unwrap_or_default(),
            fallback_binding_id.unwrap_or_default(),
        ];
        if namespace.is_some() {
            identity.push(symbol_namespace_name(namespace));
        }
        if type_only {
            identity.push("type_only");
        }
        identity.extend([start_byte.as_str(), end_byte.as_str()]);
        let id = self.stable_id("binding", &identity);
        self.batch.bindings.push(BindingFact {
            id: id.clone(),
            language: self.batch.pipeline.language.clone(),
            kind,
            spelling: spelling.to_owned(),
            qualified_target: qualified_target.to_owned(),
            namespace,
            type_only,
            target_declaration_id: target_declaration_id.map(str::to_owned),
            scope_id: scope_id.map(str::to_owned),
            output_index,
            result_type_qualified_name: result_type_qualified_name.map(str::to_owned),
            receiver_binding_id: receiver_binding_id.map(str::to_owned),
            fallback_binding_id: fallback_binding_id.map(str::to_owned),
            range,
        });
        Ok(id)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn occur(
        &mut self,
        role: SemanticRole,
        owner_declaration_id: &str,
        spelling: &str,
        qualifier: Option<&str>,
        scope_id: Option<&str>,
        range: EvidenceRange,
    ) -> Result<String, EvidenceError> {
        self.occur_with_context(
            role,
            owner_declaration_id,
            spelling,
            qualifier,
            scope_id,
            None,
            range,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn occur_with_context(
        &mut self,
        role: SemanticRole,
        owner_declaration_id: &str,
        spelling: &str,
        qualifier: Option<&str>,
        scope_id: Option<&str>,
        context: Option<&str>,
        range: EvidenceRange,
    ) -> Result<String, EvidenceError> {
        ensure_capacity(
            "occurrences",
            self.batch.occurrences.len(),
            self.limits.occurrences,
        )?;
        let id = self.stable_id(
            "occurrence",
            &[
                semantic_role_name(role),
                owner_declaration_id,
                spelling,
                qualifier.unwrap_or_default(),
                scope_id.unwrap_or_default(),
                context.unwrap_or_default(),
                &range.start_byte.to_string(),
                &range.end_byte.to_string(),
            ],
        );
        self.batch.occurrences.push(OccurrenceFact {
            id: id.clone(),
            language: self.batch.pipeline.language.clone(),
            role,
            owner_declaration_id: owner_declaration_id.to_owned(),
            spelling: spelling.to_owned(),
            qualifier: qualifier.map(str::to_owned),
            context: context.map(str::to_owned),
            scope_id: scope_id.map(str::to_owned),
            range,
        });
        Ok(id)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn relate(
        &mut self,
        relation: CandidateRelation,
        source_declaration_id: &str,
        occurrence_id: Option<&str>,
        binding_id: Option<&str>,
        target_spelling: &str,
        mut constraints: ResolutionConstraint,
    ) -> Result<String, EvidenceError> {
        ensure_capacity(
            "candidates",
            self.batch.candidates.len(),
            self.limits.candidates,
        )?;
        constraints.allowed_target_kinds.sort_unstable();
        constraints.allowed_target_kinds.dedup();
        let argument_count_identity = constraints
            .argument_count
            .map(|count| count.to_string())
            .unwrap_or_default();
        let argument_type_identities = constraints
            .argument_types
            .iter()
            .map(|argument| {
                argument.as_deref().map_or_else(
                    || "unknown".to_owned(),
                    |argument| format!("type:{argument}"),
                )
            })
            .collect::<Vec<_>>();
        let hierarchy_identity = match constraints.hierarchy.as_ref() {
            None => String::new(),
            Some(HierarchyConstraint::DirectBase { base_set_complete }) => {
                format!("direct_base:{base_set_complete}")
            }
            Some(HierarchyConstraint::ReceiverDispatch {
                receiver_qualified_name,
                strategy,
            }) => format!(
                "receiver_dispatch:{}:{receiver_qualified_name}",
                match strategy {
                    ReceiverDispatchStrategy::C3FromReceiver => "c3_from_receiver",
                    ReceiverDispatchStrategy::C3AfterReceiver => "c3_after_receiver",
                }
            ),
            Some(HierarchyConstraint::RustAssociatedType {
                receiver_declaration_id,
                receiver_qualified_name,
                trait_qualified_name,
            }) => format!(
                "rust_associated_type:{receiver_declaration_id}:{receiver_qualified_name}:{trait_qualified_name}"
            ),
        };
        let mut identity = vec![
            candidate_relation_name(relation),
            source_declaration_id,
            occurrence_id.unwrap_or_default(),
            binding_id.unwrap_or_default(),
            target_spelling,
            constraints
                .exact_target_declaration_id
                .as_deref()
                .unwrap_or_default(),
            constraints.exact_language.as_deref().unwrap_or_default(),
            constraints.module_or_package.as_deref().unwrap_or_default(),
            constraints.scope_id.as_deref().unwrap_or_default(),
            constraints.qualified_name.as_deref().unwrap_or_default(),
            argument_count_identity.as_str(),
            hierarchy_identity.as_str(),
        ];
        if !argument_type_identities.is_empty() {
            identity.push("argument_types");
            identity.extend(argument_type_identities.iter().map(String::as_str));
        }
        identity.extend(constraints.allowed_target_kinds.iter().map(String::as_str));
        identity.push(if constraints.allow_external {
            "allow_external"
        } else {
            "internal_only"
        });
        let id = self.stable_id("candidate", &identity);
        self.batch.candidates.push(RelationshipCandidate {
            id: id.clone(),
            language: self.batch.pipeline.language.clone(),
            relation,
            source_declaration_id: source_declaration_id.to_owned(),
            occurrence_id: occurrence_id.map(str::to_owned),
            binding_id: binding_id.map(str::to_owned),
            target_spelling: target_spelling.to_owned(),
            constraints,
        });
        Ok(id)
    }

    pub fn diagnose(
        &mut self,
        code: &str,
        fact_id: Option<&str>,
        range: Option<EvidenceRange>,
        message: &str,
    ) -> Result<(), EvidenceError> {
        ensure_capacity(
            "diagnostics",
            self.batch.diagnostics.len(),
            self.limits.diagnostics,
        )?;
        if message.len() > self.limits.diagnostic_message_bytes {
            return Err(EvidenceError::new(
                EvidenceErrorCode::ResourceLimit,
                format!(
                    "diagnostic message exceeds byte limit {}",
                    self.limits.diagnostic_message_bytes
                ),
            ));
        }
        self.batch.diagnostics.push(EvidenceDiagnostic {
            code: code.to_owned(),
            language: self.batch.pipeline.language.clone(),
            fact_id: fact_id.map(str::to_owned),
            range,
            message: message.to_owned(),
        });
        Ok(())
    }

    pub fn finish(mut self) -> Result<SemanticEvidenceBatch, EvidenceError> {
        self.batch.pipeline.capabilities.sort_unstable();
        self.batch.pipeline.capabilities.dedup();
        self.batch
            .declarations
            .sort_unstable_by(|left, right| left.id.cmp(&right.id));
        self.batch.declarations.dedup();
        self.batch
            .scopes
            .sort_unstable_by(|left, right| left.id.cmp(&right.id));
        self.batch.scopes.dedup();
        self.batch
            .bindings
            .sort_unstable_by(|left, right| left.id.cmp(&right.id));
        self.batch.bindings.dedup();
        self.batch
            .occurrences
            .sort_unstable_by(|left, right| left.id.cmp(&right.id));
        self.batch.occurrences.dedup();
        self.batch
            .candidates
            .sort_unstable_by(|left, right| left.id.cmp(&right.id));
        self.batch.candidates.dedup();
        self.batch.diagnostics.sort_unstable_by(|left, right| {
            left.fact_id
                .cmp(&right.fact_id)
                .then_with(|| left.code.cmp(&right.code))
                .then_with(|| left.message.cmp(&right.message))
        });
        self.batch.diagnostics.dedup();
        validate_evidence(&self.batch, self.limits)?;
        Ok(self.batch)
    }

    fn stable_id(&self, category: &str, parts: &[&str]) -> String {
        let mut digest = Sha256::new();
        for part in [
            EXTRACTION_SEMANTICS_VERSION,
            self.batch.pipeline.language.as_str(),
            self.batch.pipeline.emitter.as_str(),
            self.source_file.as_str(),
            category,
        ]
        .into_iter()
        .chain(parts.iter().copied())
        {
            digest.update(u64::try_from(part.len()).unwrap_or(u64::MAX).to_be_bytes());
            digest.update(part.as_bytes());
        }
        let digest = digest.finalize();
        let mut encoded = String::with_capacity(category.len() + 65);
        encoded.push_str(category);
        encoded.push(':');
        use std::fmt::Write as _;
        for byte in digest {
            let _ = write!(encoded, "{byte:02x}");
        }
        encoded
    }
}

#[must_use]
pub fn range_for_node(source_file: &str, node: Node<'_>) -> EvidenceRange {
    let start = node.start_position();
    let end = node.end_position();
    EvidenceRange {
        source_file: source_file.to_owned(),
        start_byte: u64::try_from(node.start_byte()).unwrap_or(u64::MAX),
        end_byte: u64::try_from(node.end_byte()).unwrap_or(u64::MAX),
        start_line: u32::try_from(start.row.saturating_add(1)).unwrap_or(u32::MAX),
        start_column: u32::try_from(start.column).unwrap_or(u32::MAX),
        end_line: u32::try_from(end.row.saturating_add(1)).unwrap_or(u32::MAX),
        end_column: u32::try_from(end.column).unwrap_or(u32::MAX),
    }
}

/// Return the inventory range for an entire source file, including parser
/// trivia such as a file that contains only whitespace or comments.
#[must_use]
pub(super) fn range_for_file(source_file: &str, source: &[u8]) -> EvidenceRange {
    let (end_row, end_column) = source.iter().fold((0_u32, 0_u32), |(row, column), byte| {
        if *byte == b'\n' {
            (row.saturating_add(1), 0)
        } else {
            (row, column.saturating_add(1))
        }
    });
    EvidenceRange {
        source_file: source_file.to_owned(),
        start_byte: 0,
        end_byte: u64::try_from(source.len()).unwrap_or(u64::MAX),
        start_line: 1,
        start_column: 0,
        end_line: end_row.saturating_add(1),
        end_column,
    }
}

/// Extract hard-cut universal evidence directly from the parser tree.
///
/// This path never reads `RawNodeRecord`, `RawEdgeRecord`, or `RawCall`.
pub(crate) fn extract_tree_evidence(
    path: &Path,
    source_file: &str,
    source: &[u8],
    root: Node<'_>,
    pipeline: &'static UniversalEvidencePipeline,
    project_evidence: Option<&crate::ProjectEvidence>,
) -> Result<SemanticEvidenceBatch, EvidenceError> {
    if pipeline.producer.language == "csharp" {
        return super::csharp::emit_tree_evidence(path, source_file, source, root);
    }
    if pipeline.producer.language == "php" {
        return super::php::emit_tree_evidence(path, source_file, source, root);
    }
    if pipeline.producer.language == "kotlin" {
        return super::kotlin::emit_tree_evidence(path, source_file, source, root);
    }
    if pipeline.producer.language == "ruby" {
        return super::ruby::emit_tree_evidence(path, source_file, source, root);
    }
    if matches!(pipeline.producer.language, "javascript" | "typescript") {
        return super::typescript::emit_tree_evidence(
            path,
            source_file,
            source,
            root,
            pipeline.producer.language,
        );
    }
    match pipeline.producer.language {
        "dart" => {
            return super::dart::emit_tree_evidence(path, source_file, source, root);
        }
        "groovy" => {
            return super::groovy::emit_tree_evidence(path, source_file, source, root);
        }
        "scala" => {
            return super::scala::emit_tree_evidence(path, source_file, source, root);
        }
        "swift" => {
            return super::swift::emit_tree_evidence(path, source_file, source, root);
        }
        _ => {}
    }
    let python_module_keys = if pipeline.producer.language == "python" {
        project_evidence
            .map(|evidence| evidence.python_module_keys(source_file))
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let unique_python_module =
        (python_module_keys.len() == 1).then(|| python_module_keys[0].clone());
    let mut state = DirectEvidenceState::new(
        path,
        source_file,
        source,
        root,
        pipeline,
        unique_python_module.as_deref(),
    );
    if python_module_keys.len() > 1 {
        state.builder.diagnose(
            "python_module_identity_ambiguous",
            None,
            Some(range_for_file(source_file, source)),
            &format!(
                "source has multiple admissible Python module identities: {}",
                python_module_keys.join(", ")
            ),
        )?;
    }
    state.add_file(root)?;
    if root.end_byte() == root.start_byte() {
        let DirectEvidenceState { builder, .. } = state;
        return builder.finish();
    }
    state.capture_parser_errors(root);
    match pipeline.producer.language {
        "python" => state.extract_python(root)?,
        "go" => state.extract_go(root)?,
        "java" => state.extract_java(root)?,
        "rust" => state.extract_rust(root)?,
        _ => {
            return Err(EvidenceError::new(
                EvidenceErrorCode::InvalidPipeline,
                format!(
                    "language {:?} has no direct universal extractor",
                    pipeline.producer.language
                ),
            ));
        }
    }
    if root.has_error() {
        state.builder.diagnose(
            "partial_parser_recovery",
            None,
            Some(range_for_node(source_file, root)),
            "parser recovered from malformed source; emitted evidence remains source-bounded",
        )?;
    }
    let DirectEvidenceState { builder, .. } = state;
    builder.finish()
}

#[derive(Clone)]
struct DeclarationContext {
    fact_id: String,
    scope_id: String,
    graph_node_id: String,
    name: String,
    qualified_name: String,
    kind: String,
    enclosing_type_qualified_name: Option<String>,
}

struct ImportBindingVersion {
    binding_id: String,
    target: String,
    active_from: usize,
}

#[derive(Default)]
struct PythonTypeBases {
    complete: bool,
    qualified_names: Vec<String>,
}

#[derive(Clone, Copy)]
struct PythonParameter<'tree> {
    syntax: Node<'tree>,
    name: Node<'tree>,
    annotation: Option<Node<'tree>>,
    defaulted: bool,
    list_splat: bool,
    dictionary_splat: bool,
}

struct PythonCanonicalAnnotation {
    canonical: String,
    runtime_targets: Vec<String>,
}

#[derive(Clone)]
struct RustImplContext {
    scope_id: String,
    type_qualified_name: String,
    trait_qualified_name: Option<String>,
    owner_declaration_id: Option<String>,
}

#[derive(Clone)]
struct RustValueTypeVersion {
    raw: Option<String>,
    active_from: usize,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum RustPlatformCfg {
    Fallback,
    Unix,
    Windows,
}

struct DirectEvidenceState<'source> {
    path: &'source Path,
    source_file: &'source str,
    source: &'source [u8],
    language: &'static str,
    module_or_package: String,
    rust_namespace_aliases: HashMap<String, String>,
    stem: String,
    file: Option<DeclarationContext>,
    declarations: HashMap<usize, DeclarationContext>,
    import_bindings: HashMap<String, HashMap<String, Vec<ImportBindingVersion>>>,
    local_bindings: HashMap<String, HashMap<String, String>>,
    local_targets: HashMap<String, HashMap<String, String>>,
    local_shadows: HashMap<String, HashSet<String>>,
    scope_parents: HashMap<String, String>,
    ambiguous_bindings: HashSet<(String, String)>,
    python_module_bound_names: HashSet<String>,
    python_local_bound_names: HashMap<String, HashSet<String>>,
    python_global_names: HashMap<String, HashSet<String>>,
    python_type_bases: HashMap<String, PythonTypeBases>,
    python_parameters: HashMap<usize, DeclarationContext>,
    python_callable_return_types: HashMap<String, String>,
    python_ambiguous_callable_returns: HashSet<String>,
    python_call_result_binding_ids: HashMap<(String, String, usize), String>,
    rust_containers: HashMap<usize, DeclarationContext>,
    rust_impls: HashMap<usize, RustImplContext>,
    rust_types_by_qualified_name: HashMap<String, DeclarationContext>,
    rust_types_by_name: HashMap<String, Vec<DeclarationContext>>,
    rust_type_parameters_by_qualified_name: HashMap<String, DeclarationContext>,
    rust_associated_types_by_scope: HashMap<String, HashMap<String, Vec<DeclarationContext>>>,
    rust_associated_types_by_qualified_name: HashMap<String, Vec<DeclarationContext>>,
    rust_trait_methods: HashMap<(String, String), Vec<String>>,
    rust_generic_bounds: HashMap<String, HashMap<String, Vec<String>>>,
    rust_receiver_methods: HashMap<(String, String), Vec<String>>,
    rust_receiver_traits: HashMap<String, Vec<String>>,
    rust_typed_receivers: HashSet<(String, String)>,
    rust_imported_typed_receivers: HashSet<(String, String)>,
    rust_platform_reexport_bindings: HashMap<String, RustPlatformCfg>,
    rust_platform_fallbacks: HashSet<(String, String)>,
    rust_field_types: HashMap<String, HashMap<String, String>>,
    rust_value_types: HashMap<String, HashMap<String, Vec<RustValueTypeVersion>>>,
    rust_callable_return_types: HashMap<String, Vec<String>>,
    rust_call_result_bindings: HashMap<(String, String, usize), String>,
    rust_import_nodes: HashSet<usize>,
    rust_test_declarations: HashSet<String>,
    go_lexical_bindings: HashMap<usize, Vec<GoLexicalBinding>>,
    go_call_result_bindings: HashMap<(String, String, usize), String>,
    go_return_types: HashMap<String, Vec<Option<String>>>,
    go_member_types: HashMap<(String, String), String>,
    go_collection_element_types: HashMap<String, String>,
    go_collection_binding_element_types: HashMap<String, HashMap<String, String>>,
    go_range_return_types: HashMap<String, Vec<Option<String>>>,
    go_range_member_types: HashMap<(String, String), String>,
    java_containers: HashMap<usize, DeclarationContext>,
    java_value_types: HashMap<String, HashMap<String, String>>,
    graph_ids: HashSet<String>,
    parser_error_ranges: Vec<(usize, usize)>,
    builder: EvidenceBuilder,
}

impl<'source> DirectEvidenceState<'source> {
    fn new(
        path: &'source Path,
        source_file: &'source str,
        source: &'source [u8],
        root: Node<'_>,
        pipeline: &'static UniversalEvidencePipeline,
        python_module: Option<&str>,
    ) -> Self {
        let stem = file_stem(path);
        let module_or_package = if pipeline.producer.language == "python" {
            python_module
                .map(str::to_owned)
                .unwrap_or_else(|| python_module_identity(path, source_file))
        } else if pipeline.producer.language == "go" {
            go_package_identity(path, source_file, source, root)
        } else if pipeline.producer.language == "java" {
            java_package_identity(source).unwrap_or_else(|| "<default>".to_owned())
        } else if pipeline.producer.language == "rust" {
            rust_module_identity(path, source_file)
        } else {
            path.parent()
                .and_then(Path::file_name)
                .and_then(|value| value.to_str())
                .unwrap_or(&stem)
                .to_owned()
        };
        Self {
            path,
            source_file,
            source,
            language: pipeline.producer.language,
            module_or_package,
            rust_namespace_aliases: if pipeline.producer.language == "rust" {
                rust_manifest_dependency_aliases(path)
            } else {
                HashMap::new()
            },
            stem,
            file: None,
            declarations: HashMap::new(),
            import_bindings: HashMap::new(),
            local_bindings: HashMap::new(),
            local_targets: HashMap::new(),
            local_shadows: HashMap::new(),
            scope_parents: HashMap::new(),
            ambiguous_bindings: HashSet::new(),
            python_module_bound_names: HashSet::new(),
            python_local_bound_names: HashMap::new(),
            python_global_names: HashMap::new(),
            python_type_bases: HashMap::new(),
            python_parameters: HashMap::new(),
            python_callable_return_types: HashMap::new(),
            python_ambiguous_callable_returns: HashSet::new(),
            python_call_result_binding_ids: HashMap::new(),
            rust_containers: HashMap::new(),
            rust_impls: HashMap::new(),
            rust_types_by_qualified_name: HashMap::new(),
            rust_types_by_name: HashMap::new(),
            rust_type_parameters_by_qualified_name: HashMap::new(),
            rust_associated_types_by_scope: HashMap::new(),
            rust_associated_types_by_qualified_name: HashMap::new(),
            rust_trait_methods: HashMap::new(),
            rust_generic_bounds: HashMap::new(),
            rust_receiver_methods: HashMap::new(),
            rust_receiver_traits: HashMap::new(),
            rust_typed_receivers: HashSet::new(),
            rust_imported_typed_receivers: HashSet::new(),
            rust_platform_reexport_bindings: HashMap::new(),
            rust_platform_fallbacks: HashSet::new(),
            rust_field_types: HashMap::new(),
            rust_value_types: HashMap::new(),
            rust_callable_return_types: HashMap::new(),
            rust_call_result_bindings: HashMap::new(),
            rust_import_nodes: HashSet::new(),
            rust_test_declarations: HashSet::new(),
            go_lexical_bindings: HashMap::new(),
            go_call_result_bindings: HashMap::new(),
            go_return_types: HashMap::new(),
            go_member_types: HashMap::new(),
            go_collection_element_types: HashMap::new(),
            go_collection_binding_element_types: HashMap::new(),
            go_range_return_types: HashMap::new(),
            go_range_member_types: HashMap::new(),
            java_containers: HashMap::new(),
            java_value_types: HashMap::new(),
            graph_ids: HashSet::new(),
            parser_error_ranges: Vec::new(),
            builder: EvidenceBuilder::new(
                pipeline,
                format!("compass.languages.{}.universal", pipeline.producer.language),
                source_file,
                EvidenceLimits::default(),
            ),
        }
    }

    fn capture_parser_errors(&mut self, root: Node<'_>) {
        collect_parser_error_ranges(root, &mut self.parser_error_ranges);
        self.parser_error_ranges.sort_unstable();
        self.parser_error_ranges.dedup();
    }

    fn overlaps_parser_error(&self, node: Node<'_>) -> bool {
        let start = node.start_byte();
        let end = node.end_byte();
        let line_end = self.source[end..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(self.source.len(), |offset| end.saturating_add(offset));
        self.parser_error_ranges
            .iter()
            .any(|(error_start, error_end)| {
                if error_start == error_end {
                    start <= *error_start && *error_start <= line_end
                } else {
                    *error_start <= line_end && start < *error_end
                }
            })
    }

    fn has_invalid_python_line_prefix(&self, node: Node<'_>) -> bool {
        let start = node.start_byte();
        let line_start = self.source[..start]
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |position| position.saturating_add(1));
        std::str::from_utf8(&self.source[line_start..start])
            .is_ok_and(|prefix| !valid_python_import_whitespace(prefix))
    }

    fn has_invalid_python_import_suffix(&self, node: Node<'_>) -> bool {
        let end = node.end_byte();
        let line_end = self.source[end..]
            .iter()
            .position(|byte| matches!(*byte, b'\n' | b'\r'))
            .map_or(self.source.len(), |offset| end.saturating_add(offset));
        std::str::from_utf8(&self.source[end..line_end]).is_ok_and(|suffix| {
            let suffix = suffix.trim_start();
            !suffix.is_empty() && !suffix.starts_with('#') && !suffix.starts_with(';')
        })
    }

    fn declaration_metadata(&self, node: Node<'_>) -> DeclarationMetadata {
        let body = evidence_declaration_body(node);
        DeclarationMetadata {
            signature: evidence_readable_signature(node, body, self.source),
            parameter_count: None,
            parameter_types: Vec::new(),
            direct_bases_complete: false,
            variadic: false,
            signature_hash: Some(evidence_ast_hash(
                node,
                self.source,
                body.map(|body| body.id()),
            )),
            implementation_hash: body.map(|body| evidence_ast_hash(body, self.source, None)),
            source_hash: self
                .source
                .get(node.start_byte()..node.end_byte())
                .map(evidence_normalized_source_hash),
        }
    }

    fn add_file(&mut self, _root: Node<'_>) -> Result<(), EvidenceError> {
        let label = self
            .path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or(self.source_file);
        let graph_node_id = make_id(&[self.source_file]);
        self.graph_ids.insert(graph_node_id.clone());
        let range = range_for_file(self.source_file, self.source);
        let fact_id = self.builder.declare(
            "file",
            &graph_node_id,
            label,
            &self.module_or_package,
            Some(&self.module_or_package),
            None,
            range.clone(),
        )?;
        let scope_id = self
            .builder
            .open_scope("module", Some(&fact_id), None, range)?;
        self.file = Some(DeclarationContext {
            fact_id,
            scope_id,
            graph_node_id,
            name: label.to_owned(),
            qualified_name: self.module_or_package.clone(),
            kind: "file".to_owned(),
            enclosing_type_qualified_name: None,
        });
        Ok(())
    }

    fn extract_python(&mut self, root: Node<'_>) -> Result<(), EvidenceError> {
        let file = self.file.clone().ok_or_else(|| {
            EvidenceError::new(
                EvidenceErrorCode::InvalidFact,
                "non-empty Python source has no file evidence",
            )
        })?;
        self.collect_python_declarations(root, &file)?;
        self.collect_python_imports(root, &file)?;
        self.collect_python_partial_aliases(root, &file)?;
        self.collect_python_module_variables(root, &file)?;
        let module_bound = crate::engine::python_bound_names(root, self.source, true);
        self.python_module_bound_names.clone_from(&module_bound);
        self.walk_python_value_references(root, &file, true, &module_bound)?;
        self.walk_python_evidence(root, &file, true)
    }

    fn collect_python_partial_aliases(
        &mut self,
        root: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        let mut cursor = root.walk();
        for assignment in root
            .children(&mut cursor)
            .filter(|child| child.is_named() && child.kind() == "assignment")
        {
            let (Some(alias_node), Some(call)) = (
                assignment.child_by_field_name("left"),
                assignment.child_by_field_name("right"),
            ) else {
                continue;
            };
            if alias_node.kind() != "identifier" || call.kind() != "call" {
                continue;
            }
            let Some(function) = call.child_by_field_name("function") else {
                continue;
            };
            let partial_name = self.text(function);
            if function.kind() != "identifier"
                || self
                    .imported_target_for_occurrence(
                        owner,
                        &partial_name,
                        function.start_byte(),
                        false,
                    )
                    .map(String::as_str)
                    != Some("functools.partial")
                || self.python_name_rebound_between(root, 0, assignment.start_byte(), &partial_name)
            {
                continue;
            }
            let Some(arguments) = call.child_by_field_name("arguments") else {
                continue;
            };
            let mut arguments_cursor = arguments.walk();
            let Some(target_node) = arguments
                .named_children(&mut arguments_cursor)
                .next()
                .filter(|node| node.kind() == "identifier")
            else {
                continue;
            };
            let target_name = self.text(target_node);
            let target_qualified_name = format!("{}.{}", self.module_or_package, target_name);
            let matching_targets = self
                .declarations
                .values()
                .filter(|context| {
                    context.kind == "function"
                        && context.qualified_name == target_qualified_name
                        && context.scope_id != owner.scope_id
                })
                .cloned()
                .collect::<Vec<_>>();
            let [target] = matching_targets.as_slice() else {
                continue;
            };
            let Some(target_definition) = direct_python_function(root, &target_name, self.source)
            else {
                continue;
            };
            if target_definition.end_byte() > assignment.start_byte()
                || self.python_name_rebound_between(
                    root,
                    target_definition.end_byte(),
                    assignment.start_byte(),
                    &target_name,
                )
            {
                continue;
            }

            let alias = self.text(alias_node);
            let qualified_name = format!("{}.{}", self.module_or_package, alias);
            let graph_node_id =
                self.unique_graph_id(make_id(&[&self.module_or_package, &alias]), assignment);
            let fact_id = self.builder.declare(
                "function",
                &graph_node_id,
                &alias,
                &qualified_name,
                Some(&self.module_or_package),
                Some(&owner.scope_id),
                range_for_node(self.source_file, alias_node),
            )?;
            let alias_context = DeclarationContext {
                fact_id: fact_id.clone(),
                scope_id: owner.scope_id.clone(),
                graph_node_id,
                name: alias,
                qualified_name,
                kind: "function".to_owned(),
                enclosing_type_qualified_name: None,
            };
            self.add_ownership(owner, &alias_context)?;
            let occurrence_id = self.builder.occur_with_context(
                SemanticRole::CallableReference,
                &fact_id,
                &target_name,
                None,
                Some(&owner.scope_id),
                Some("partial_target"),
                range_for_node(self.source_file, target_node),
            )?;
            self.builder.relate(
                CandidateRelation::References,
                &fact_id,
                Some(&occurrence_id),
                None,
                &target_name,
                ResolutionConstraint {
                    exact_target_declaration_id: Some(target.fact_id.clone()),
                    exact_language: Some(self.language.to_owned()),
                    module_or_package: Some(self.module_or_package.clone()),
                    scope_id: Some(owner.scope_id.clone()),
                    qualified_name: Some(target.qualified_name.clone()),
                    argument_count: None,
                    argument_types: Vec::new(),
                    allowed_target_kinds: vec!["function".to_owned()],
                    hierarchy: None,
                    allow_external: false,
                },
            )?;
            self.declarations.insert(assignment.id(), alias_context);
        }
        Ok(())
    }

    fn collect_python_module_variables(
        &mut self,
        root: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        let mut binding_statements = HashMap::<String, HashSet<usize>>::new();
        let mut deleted_names = HashSet::<String>::new();
        let mut cursor = root.walk();
        for statement in root.children(&mut cursor).filter(|child| child.is_named()) {
            let statement_id = statement.id();
            if !matches!(
                statement.kind(),
                "function_definition" | "class_definition" | "decorated_definition"
            ) {
                for name in crate::engine::python_bound_names(statement, self.source, true) {
                    binding_statements
                        .entry(name)
                        .or_default()
                        .insert(statement_id);
                }
            }
            let mut declaration_names = HashSet::new();
            collect_python_module_declaration_names(statement, self.source, &mut declaration_names);
            collect_python_module_mutations(
                statement,
                self.source,
                &mut binding_statements,
                &mut deleted_names,
                statement_id,
            );
            for name in declaration_names {
                binding_statements
                    .entry(name)
                    .or_default()
                    .insert(statement_id);
            }
        }

        let mut cursor = root.walk();
        for assignment in root
            .children(&mut cursor)
            .filter(|child| child.is_named() && child.kind() == "assignment")
        {
            if self.declarations.contains_key(&assignment.id())
                || self.overlaps_parser_error(assignment)
            {
                continue;
            }
            let Some(name_node) = assignment
                .child_by_field_name("left")
                .filter(|node| node.kind() == "identifier")
            else {
                continue;
            };
            let name = self.text(name_node);
            if !valid_python_identifier(&name)
                || deleted_names.contains(&name)
                || binding_statements.get(&name).is_none_or(|statements| {
                    statements.len() != 1 || !statements.contains(&assignment.id())
                })
                || self
                    .import_bindings
                    .get(&owner.scope_id)
                    .is_some_and(|bindings| bindings.contains_key(&name))
                || assignment
                    .child_by_field_name("right")
                    .is_some_and(|right| {
                        crate::engine::python_bound_names(right, self.source, true).contains(&name)
                    })
            {
                continue;
            }

            let qualified_name = format!("{}.{}", self.module_or_package, name);
            let graph_node_id =
                self.unique_graph_id(make_id(&[&self.module_or_package, &name]), assignment);
            let fact_id = self.builder.declare(
                "variable",
                &graph_node_id,
                &name,
                &qualified_name,
                Some(&self.module_or_package),
                Some(&owner.scope_id),
                range_for_node(self.source_file, name_node),
            )?;
            let context = DeclarationContext {
                fact_id: fact_id.clone(),
                scope_id: owner.scope_id.clone(),
                graph_node_id,
                name,
                qualified_name,
                kind: "variable".to_owned(),
                enclosing_type_qualified_name: None,
            };
            self.add_ownership(owner, &context)?;
            self.add_python_module_variable_type(root, assignment, &context)?;
            self.declarations.insert(assignment.id(), context);
        }
        Ok(())
    }

    fn add_python_module_variable_type(
        &mut self,
        root: Node<'_>,
        assignment: Node<'_>,
        variable: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        let Some(call) = assignment
            .child_by_field_name("right")
            .filter(|node| node.kind() == "call")
        else {
            return Ok(());
        };
        let Some(function) = call
            .child_by_field_name("function")
            .filter(|node| node.kind() == "identifier")
        else {
            return Ok(());
        };
        let spelling = self.text(function);
        let imported_target = self
            .imported_target_for_occurrence(variable, &spelling, function.start_byte(), false)
            .cloned();
        let local_qualified_name = format!("{}.{}", self.module_or_package, spelling);
        let local_targets = self
            .declarations
            .values()
            .filter(|context| {
                context.kind == "class" && context.qualified_name == local_qualified_name
            })
            .cloned()
            .collect::<Vec<_>>();
        let local_target = match local_targets.as_slice() {
            [target]
                if direct_python_definition(root, &spelling, "class_definition", self.source)
                    .is_some_and(|definition| definition.end_byte() <= assignment.start_byte()) =>
            {
                Some(target)
            }
            _ => None,
        };
        let qualified_name = imported_target
            .clone()
            .or_else(|| local_target.map(|target| target.qualified_name.clone()));
        let Some(qualified_name) = qualified_name else {
            return Ok(());
        };
        let binding_id = self
            .binding_for_occurrence(variable, &spelling, function.start_byte(), false)
            .cloned();
        let occurrence_id = self.builder.occur_with_context(
            SemanticRole::TypeReference,
            &variable.fact_id,
            &spelling,
            None,
            Some(&variable.scope_id),
            Some("initializer_type"),
            range_for_node(self.source_file, function),
        )?;
        self.builder.relate(
            CandidateRelation::TypeOf,
            &variable.fact_id,
            Some(&occurrence_id),
            binding_id.as_deref(),
            &spelling,
            ResolutionConstraint {
                exact_target_declaration_id: local_target.map(|target| target.fact_id.clone()),
                exact_language: Some(self.language.to_owned()),
                module_or_package: qualified_name
                    .rsplit_once('.')
                    .map(|(module, _)| module.to_owned()),
                scope_id: Some(variable.scope_id.clone()),
                qualified_name: Some(qualified_name),
                argument_count: None,
                argument_types: Vec::new(),
                allowed_target_kinds: vec!["class".to_owned()],
                hierarchy: None,
                allow_external: false,
            },
        )?;
        Ok(())
    }

    fn python_name_rebound_between(
        &self,
        root: Node<'_>,
        start: usize,
        end: usize,
        name: &str,
    ) -> bool {
        let mut cursor = root.walk();
        root.children(&mut cursor)
            .filter(|child| child.is_named())
            .any(|child| {
                child.start_byte() >= start
                    && child.end_byte() <= end
                    && !matches!(child.kind(), "import_statement" | "import_from_statement")
                    && crate::engine::python_bound_names(child, self.source, true).contains(name)
            })
    }

    fn collect_python_imports(
        &mut self,
        node: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        let active = if matches!(node.kind(), "class_definition" | "function_definition") {
            let Some(context) = self.declarations.get(&node.id()).cloned() else {
                return Ok(());
            };
            context
        } else {
            owner.clone()
        };
        if matches!(node.kind(), "import_statement" | "import_from_statement") {
            self.add_python_imports(node, &active)?;
            return Ok(());
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(|child| child.is_named()) {
            self.collect_python_imports(child, &active)?;
        }
        Ok(())
    }

    fn collect_python_declarations(
        &mut self,
        node: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        if matches!(node.kind(), "class_definition" | "function_definition") {
            let Some(name_node) = node.child_by_field_name("name") else {
                return Ok(());
            };
            let name = self.text(name_node);
            if name.is_empty() {
                return Ok(());
            }
            let is_class = node.kind() == "class_definition";
            let qualified_name = if owner.kind == "file" {
                format!("{}.{}", self.module_or_package, name)
            } else {
                format!("{}::{name}", owner.qualified_name)
            };
            let graph_node_id = if owner.kind == "file" {
                make_id(&[
                    &self.module_or_package,
                    qualified_name.rsplit('.').next().unwrap_or(&name),
                ])
            } else {
                make_id(&[&owner.graph_node_id, &name])
            };
            let graph_node_id = self.unique_graph_id(graph_node_id, node);
            let kind = if is_class {
                "class"
            } else if owner.kind == "class" {
                "method"
            } else {
                "function"
            };
            let mut metadata = self.declaration_metadata(node);
            if !is_class {
                let parameters = python_parameter_nodes(node);
                let implicit_receiver = kind == "method"
                    && !python_has_decorator(node, self.source, "staticmethod")
                    && !parameters.is_empty();
                let callable_parameters = parameters.iter().skip(usize::from(implicit_receiver));
                metadata.parameter_count = (!callable_parameters
                    .clone()
                    .any(|parameter| parameter.defaulted || parameter.dictionary_splat))
                .then(|| {
                    u32::try_from(callable_parameters.clone().count()).map_err(|_| {
                        EvidenceError::new(
                            EvidenceErrorCode::ResourceLimit,
                            "Python callable parameter count exceeds the evidence schema limit",
                        )
                    })
                })
                .transpose()?;
                metadata.variadic = callable_parameters
                    .clone()
                    .any(|parameter| parameter.list_splat || parameter.dictionary_splat);
            }
            let fact_id = self.builder.declare_with_metadata(
                kind,
                &graph_node_id,
                &name,
                &qualified_name,
                Some(&self.module_or_package),
                Some(&owner.scope_id),
                range_for_node(self.source_file, name_node),
                metadata,
            )?;
            let scope_id = self.builder.open_scope(
                kind,
                Some(&fact_id),
                Some(&owner.scope_id),
                range_for_node(self.source_file, node),
            )?;
            self.scope_parents
                .insert(scope_id.clone(), owner.scope_id.clone());
            let context = DeclarationContext {
                fact_id,
                scope_id,
                graph_node_id,
                name,
                qualified_name,
                kind: kind.to_owned(),
                enclosing_type_qualified_name: if owner.kind == "class" {
                    Some(owner.qualified_name.clone())
                } else {
                    owner.enclosing_type_qualified_name.clone()
                },
            };
            self.add_ownership(owner, &context)?;
            self.declarations.insert(node.id(), context.clone());
            if !is_class {
                self.add_python_parameter_declarations(node, &context)?;
            }
            let body = node.child_by_field_name("body").unwrap_or(node);
            let mut cursor = body.walk();
            for child in body.children(&mut cursor).filter(|child| child.is_named()) {
                self.collect_python_declarations(child, &context)?;
            }
            return Ok(());
        }
        if node.kind() == "assignment"
            && node.child_by_field_name("type").is_some()
            && !self.overlaps_parser_error(node)
            && let Some(name_node) = node
                .child_by_field_name("left")
                .filter(|left| left.kind() == "identifier")
        {
            let name = self.text(name_node);
            if valid_python_identifier(&name) {
                let kind = if owner.kind == "class" {
                    "field"
                } else {
                    "variable"
                };
                let qualified_name = if owner.kind == "file" {
                    format!("{}.{}", self.module_or_package, name)
                } else {
                    format!("{}::{name}", owner.qualified_name)
                };
                let graph_node_id =
                    self.unique_graph_id(make_id(&[&owner.graph_node_id, kind, &name]), node);
                let mut metadata = self.declaration_metadata(node);
                metadata.signature = node.child_by_field_name("type").map(|kind| self.text(kind));
                let fact_id = self.builder.declare_with_metadata(
                    kind,
                    &graph_node_id,
                    &name,
                    &qualified_name,
                    Some(&self.module_or_package),
                    Some(&owner.scope_id),
                    range_for_node(self.source_file, name_node),
                    metadata,
                )?;
                let context = DeclarationContext {
                    fact_id,
                    scope_id: owner.scope_id.clone(),
                    graph_node_id,
                    name,
                    qualified_name,
                    kind: kind.to_owned(),
                    enclosing_type_qualified_name: owner.enclosing_type_qualified_name.clone(),
                };
                self.add_ownership(owner, &context)?;
                self.declarations.insert(node.id(), context);
            }
            return Ok(());
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(|child| child.is_named()) {
            self.collect_python_declarations(child, owner)?;
        }
        Ok(())
    }

    fn add_python_parameter_declarations(
        &mut self,
        declaration: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        for parameter in python_parameter_nodes(declaration) {
            let name = self.text(parameter.name);
            if name.is_empty() {
                continue;
            }
            let qualified_name = format!("{}::parameter:{name}", owner.qualified_name);
            let graph_node_id = self.unique_graph_id(
                make_id(&[&owner.graph_node_id, "parameter", &name]),
                parameter.syntax,
            );
            let mut metadata = self.declaration_metadata(parameter.syntax);
            metadata.signature = parameter.annotation.map(|annotation| self.text(annotation));
            let fact_id = self.builder.declare_with_metadata(
                "parameter",
                &graph_node_id,
                &name,
                &qualified_name,
                Some(&self.module_or_package),
                Some(&owner.scope_id),
                range_for_node(self.source_file, parameter.name),
                metadata,
            )?;
            let context = DeclarationContext {
                fact_id,
                scope_id: owner.scope_id.clone(),
                graph_node_id,
                name,
                qualified_name,
                kind: "parameter".to_owned(),
                enclosing_type_qualified_name: owner.enclosing_type_qualified_name.clone(),
            };
            self.add_ownership(owner, &context)?;
            self.python_parameters
                .insert(parameter.syntax.id(), context);
        }
        Ok(())
    }

    fn walk_python_evidence(
        &mut self,
        node: Node<'_>,
        owner: &DeclarationContext,
        root: bool,
    ) -> Result<(), EvidenceError> {
        let mut active = owner.clone();
        if matches!(node.kind(), "class_definition" | "function_definition") {
            let Some(context) = self.declarations.get(&node.id()).cloned() else {
                return Ok(());
            };
            active = context;
            self.add_python_decorators(node, &active)?;
            if node.kind() == "class_definition" {
                self.add_python_bases(node, &active)?;
                self.add_python_class_member_aliases(node, &active)?;
            }
            self.add_python_annotations(node, &active)?;
            if node.kind() == "function_definition" {
                let body = node.child_by_field_name("body").unwrap_or(node);
                let mut bound = crate::engine::python_bound_names(node, self.source, false);
                let (global_names, nonlocal_names) =
                    python_scope_directive_names(node, self.source);
                bound.retain(|name| !global_names.contains(name) && !nonlocal_names.contains(name));
                self.python_local_bound_names
                    .insert(active.scope_id.clone(), bound.clone());
                self.python_global_names
                    .insert(active.scope_id.clone(), global_names);
                self.walk_python_value_references(body, &active, true, &bound)?;
            }
        }
        match node.kind() {
            "import_statement" | "import_from_statement" => return Ok(()),
            "call" => self.add_call(node, &active, "call")?,
            "assignment" => {
                if let (Some(variable), Some(annotation)) = (
                    self.declarations.get(&node.id()).cloned(),
                    node.child_by_field_name("type"),
                ) && let Some(canonical) = self.python_canonical_annotation(&active, annotation)
                {
                    self.add_python_exact_annotation_relationship(
                        &variable,
                        annotation,
                        CandidateRelation::TypeOf,
                        &canonical,
                    )?;
                }
            }
            _ => {}
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(|child| child.is_named()) {
            if !root
                && matches!(child.kind(), "class_definition" | "function_definition")
                && !self.declarations.contains_key(&child.id())
            {
                continue;
            }
            self.walk_python_evidence(child, &active, false)?;
        }
        Ok(())
    }

    fn add_python_class_member_aliases(
        &mut self,
        class: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        let Some(body) = class.child_by_field_name("body") else {
            return Ok(());
        };
        let mut cursor = body.walk();
        let statements = body
            .children(&mut cursor)
            .filter(|child| child.is_named())
            .collect::<Vec<_>>();
        let mut root = class;
        while let Some(parent) = root.parent() {
            root = parent;
        }
        for (index, statement) in statements.iter().enumerate() {
            let assignment = if statement.kind() == "assignment" {
                *statement
            } else if statement.kind() == "expression_statement" {
                let Some(assignment) = statement.named_child(0) else {
                    continue;
                };
                if assignment.kind() != "assignment" {
                    continue;
                }
                assignment
            } else {
                continue;
            };
            let (Some(left), Some(right)) = (
                assignment.child_by_field_name("left"),
                assignment.child_by_field_name("right"),
            ) else {
                continue;
            };
            if left.kind() != "identifier" || right.kind() != "identifier" {
                continue;
            }
            let spelling = self.text(left);
            let target_name = self.text(right);
            if !valid_python_identifier(&spelling) || !valid_python_identifier(&target_name) {
                continue;
            }
            if statements[..index].iter().any(|earlier| {
                let bound = crate::engine::python_bound_names(*earlier, self.source, true);
                bound.contains(&spelling) || bound.contains(&target_name)
            }) || statements[index.saturating_add(1)..].iter().any(|later| {
                crate::engine::python_bound_names(*later, self.source, true).contains(&spelling)
            }) {
                continue;
            }
            let target =
                direct_python_definition(root, &target_name, "function_definition", self.source)
                    .or_else(|| {
                        direct_python_definition(
                            root,
                            &target_name,
                            "class_definition",
                            self.source,
                        )
                    });
            let Some(target) = target.filter(|target| target.end_byte() <= class.start_byte())
            else {
                continue;
            };
            if self.python_name_rebound_between(
                root,
                target.end_byte(),
                class.start_byte(),
                &target_name,
            ) {
                continue;
            }
            let Some(target) = self.declarations.get(&target.id()) else {
                continue;
            };
            self.builder.bind(
                BindingKind::Member,
                &spelling,
                &target.qualified_name,
                Some(&target.fact_id),
                Some(&owner.scope_id),
                range_for_node(self.source_file, left),
            )?;
        }
        Ok(())
    }

    fn walk_python_value_references(
        &mut self,
        node: Node<'_>,
        owner: &DeclarationContext,
        root: bool,
        bound: &HashSet<String>,
    ) -> Result<(), EvidenceError> {
        if !root && matches!(node.kind(), "function_definition" | "class_definition") {
            return Ok(());
        }
        if owner.kind != "file"
            && node.kind() == "call"
            && let Some(arguments) = node.child_by_field_name("arguments")
        {
            let mut cursor = arguments.walk();
            for argument in arguments.children(&mut cursor) {
                let candidate = if argument.kind() == "identifier" {
                    Some(argument)
                } else if argument.kind() == "keyword_argument" {
                    argument.child_by_field_name("value")
                } else {
                    None
                };
                if candidate.is_some_and(|candidate| candidate.kind() == "identifier") {
                    self.add_python_value_reference(owner, candidate, "argument", bound)?;
                }
            }
        }
        if matches!(node.kind(), "dictionary" | "list" | "set" | "tuple") {
            let mut identifiers = Vec::new();
            crate::engine::collect_python_collection_values(node, &mut identifiers);
            for identifier in identifiers {
                self.add_python_value_reference(owner, Some(identifier), "collection", bound)?;
            }
        } else if node.kind() == "assignment"
            && let Some(value) = node.child_by_field_name("right")
        {
            let mut identifiers = Vec::new();
            crate::engine::collect_python_reference_values(value, &mut identifiers);
            for identifier in identifiers {
                self.add_python_value_reference(owner, Some(identifier), "assignment", bound)?;
            }
        } else if node.kind() == "return_statement" {
            let mut cursor = node.walk();
            if let Some(value) = node.children(&mut cursor).find(|child| child.is_named()) {
                let mut identifiers = Vec::new();
                crate::engine::collect_python_reference_values(value, &mut identifiers);
                for identifier in identifiers {
                    self.add_python_value_reference(owner, Some(identifier), "return", bound)?;
                }
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk_python_value_references(child, owner, false, bound)?;
        }
        Ok(())
    }

    fn add_python_value_reference(
        &mut self,
        owner: &DeclarationContext,
        node: Option<Node<'_>>,
        context: &str,
        bound: &HashSet<String>,
    ) -> Result<(), EvidenceError> {
        let Some(node) = node else {
            return Ok(());
        };
        let spelling = self.text(node);
        if spelling.is_empty()
            || bound.contains(&spelling)
            || !valid_python_identifier(&spelling)
            || self.overlaps_parser_error(node)
        {
            return Ok(());
        }
        let allow_later_file_binding = matches!(owner.kind.as_str(), "function" | "method");
        if self.import_binding_declared_but_not_visible(
            owner,
            &spelling,
            node.start_byte(),
            allow_later_file_binding,
        ) {
            return Ok(());
        }
        let binding = self
            .binding_for_occurrence(
                owner,
                &spelling,
                node.start_byte(),
                allow_later_file_binding,
            )
            .or_else(|| {
                self.python_wildcard_binding(owner, node.start_byte(), allow_later_file_binding)
            })
            .cloned();
        let qualified_name = self
            .imported_target_for_occurrence(
                owner,
                &spelling,
                node.start_byte(),
                allow_later_file_binding,
            )
            .cloned();
        let occurrence_id = self.builder.occur_with_context(
            SemanticRole::CallableReference,
            &owner.fact_id,
            &spelling,
            None,
            Some(&owner.scope_id),
            Some(context),
            range_for_node(self.source_file, node),
        )?;
        self.builder.relate(
            CandidateRelation::References,
            &owner.fact_id,
            Some(&occurrence_id),
            binding.as_deref(),
            &spelling,
            ResolutionConstraint {
                exact_target_declaration_id: None,
                exact_language: Some(self.language.to_owned()),
                module_or_package: qualified_name
                    .as_deref()
                    .and_then(|qualified| qualified.rsplit_once('.').map(|(module, _)| module))
                    .map(str::to_owned)
                    .or_else(|| Some(self.module_or_package.clone())),
                scope_id: Some(owner.scope_id.clone()),
                qualified_name: qualified_name.clone(),
                argument_count: None,
                argument_types: Vec::new(),
                allowed_target_kinds: vec![
                    "class".to_owned(),
                    "function".to_owned(),
                    "method".to_owned(),
                    "variable".to_owned(),
                ],
                hierarchy: None,
                allow_external: qualified_name.is_some(),
            },
        )?;
        Ok(())
    }

    fn add_python_imports(
        &mut self,
        node: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        if self.overlaps_parser_error(node) || self.has_invalid_python_line_prefix(node) {
            return Ok(());
        }
        let statement = self.text(node);
        if !valid_python_import_whitespace(&statement)
            || !valid_python_line_continuations(&statement)
        {
            return Ok(());
        }
        let module = node.child_by_field_name("module_name").map(|module| {
            resolve_python_module(
                &self.module_or_package,
                &self.text(module),
                matches!(
                    self.path.file_name().and_then(|name| name.to_str()),
                    Some("__init__.py" | "__init__.pyi")
                ),
            )
        });
        let named_import = module.is_some();
        if python_import_contains_wildcard(&statement) {
            if !self.has_invalid_python_import_suffix(node)
                && let (Some(module), Some((start, end))) =
                    (module.as_deref(), python_wildcard_import_span(&statement))
            {
                let range = range_for_byte_span(
                    self.source_file,
                    self.source,
                    node.start_byte().saturating_add(start),
                    node.start_byte().saturating_add(end),
                );
                self.add_python_wildcard_import(owner, module, range)?;
            }
            return Ok(());
        }
        let mut cursor = node.walk();
        let imported_names = node
            .children_by_field_name("name", &mut cursor)
            .collect::<Vec<_>>();
        if imported_names.iter().any(|imported| {
            if imported.kind() == "aliased_import" {
                imported
                    .child_by_field_name("name")
                    .is_some_and(|target| self.text(target) == "*")
            } else {
                self.text(*imported) == "*"
            }
        }) {
            return Ok(());
        }
        for imported in imported_names {
            let (target_name, alias, alias_node) = if imported.kind() == "aliased_import" {
                let Some(target) = imported.child_by_field_name("name") else {
                    continue;
                };
                let alias_node = imported.child_by_field_name("alias");
                let alias = alias_node.as_ref().and_then(|alias| {
                    let alias = self.text(*alias);
                    valid_python_identifier(&alias).then_some(alias)
                });
                (self.text(target), alias, alias_node)
            } else {
                (self.text(imported), None, None)
            };
            if !valid_python_import_target(&target_name)
                || alias_node
                    .as_ref()
                    .is_some_and(|alias_node| alias.is_none() && !self.text(*alias_node).is_empty())
            {
                continue;
            }
            if target_name.is_empty() || target_name == "*" {
                continue;
            }
            let (local, binding_target, import_target) = if let Some(module) = module.as_deref() {
                let local = alias.unwrap_or_else(|| target_name.clone());
                let target = if module.is_empty() {
                    target_name
                } else {
                    format!("{module}.{target_name}")
                };
                (local, target.clone(), target)
            } else if let Some(alias) = alias {
                (alias, target_name.clone(), target_name)
            } else {
                let local = target_name.split('.').next().unwrap_or_default().to_owned();
                (local.clone(), local, target_name)
            };
            if local.is_empty()
                || binding_target.rsplit('.').next().is_none_or(str::is_empty)
                || import_target.rsplit('.').next().is_none_or(str::is_empty)
            {
                self.builder.diagnose(
                    "unsupported_import_target",
                    Some(&owner.fact_id),
                    Some(range_for_node(self.source_file, imported)),
                    "import target could not be represented as a source-grounded binding",
                )?;
                continue;
            }
            self.add_python_import_binding(
                imported,
                owner,
                local,
                binding_target,
                import_target,
                alias_node,
                named_import,
            )?;
        }
        Ok(())
    }

    fn add_python_wildcard_import(
        &mut self,
        owner: &DeclarationContext,
        module: &str,
        range: EvidenceRange,
    ) -> Result<(), EvidenceError> {
        if module.is_empty() {
            return Ok(());
        }
        let is_reexport = owner.kind == "file"
            && matches!(
                self.path.file_name().and_then(|name| name.to_str()),
                Some("__init__.py" | "__init__.pyi")
            );
        let kind = if is_reexport {
            BindingKind::Reexport
        } else {
            BindingKind::Import
        };
        let binding_id = self.builder.bind(
            kind,
            "*",
            module,
            None,
            Some(&owner.scope_id),
            range.clone(),
        )?;
        self.record_import_binding(
            owner,
            "*",
            module,
            &binding_id,
            usize::try_from(range.end_byte).unwrap_or(usize::MAX),
        );
        let occurrence_id = self.builder.occur(
            if is_reexport {
                SemanticRole::Reexport
            } else {
                SemanticRole::Import
            },
            &owner.fact_id,
            "*",
            None,
            Some(&owner.scope_id),
            range,
        )?;
        self.builder.relate(
            if is_reexport {
                CandidateRelation::Reexports
            } else {
                CandidateRelation::Imports
            },
            &owner.fact_id,
            Some(&occurrence_id),
            Some(&binding_id),
            module.rsplit('.').next().unwrap_or(module),
            ResolutionConstraint {
                exact_target_declaration_id: None,
                exact_language: Some(self.language.to_owned()),
                module_or_package: module
                    .rsplit_once('.')
                    .map(|(package, _)| package.to_owned()),
                scope_id: Some(owner.scope_id.clone()),
                qualified_name: Some(module.to_owned()),
                argument_count: None,
                argument_types: Vec::new(),
                allowed_target_kinds: vec![
                    "file".to_owned(),
                    "module".to_owned(),
                    "package".to_owned(),
                ],
                hierarchy: None,
                allow_external: true,
            },
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn add_python_import_binding(
        &mut self,
        imported: Node<'_>,
        owner: &DeclarationContext,
        local: String,
        binding_target: String,
        import_target: String,
        alias_name_node: Option<Node<'_>>,
        named_import: bool,
    ) -> Result<(), EvidenceError> {
        let is_reexport = owner.kind == "file"
            && matches!(
                self.path.file_name().and_then(|name| name.to_str()),
                Some("__init__.py" | "__init__.pyi")
            );
        let kind = if is_reexport {
            BindingKind::Reexport
        } else if local == binding_target.rsplit('.').next().unwrap_or_default() {
            BindingKind::Import
        } else {
            BindingKind::ImportAlias
        };
        let binding_range = range_for_node(self.source_file, imported);
        let occurrence_range = alias_name_node
            .map(|alias_name_node| range_for_node(self.source_file, alias_name_node))
            .unwrap_or_else(|| binding_range.clone());
        let binding_id = self.builder.bind(
            kind,
            &local,
            &binding_target,
            None,
            Some(&owner.scope_id),
            binding_range,
        )?;
        self.record_import_binding(
            owner,
            &local,
            &binding_target,
            &binding_id,
            usize::try_from(occurrence_range.end_byte).unwrap_or(usize::MAX),
        );
        let occurrence_id = self.builder.occur(
            if is_reexport {
                SemanticRole::Reexport
            } else {
                SemanticRole::Import
            },
            &owner.fact_id,
            &local,
            None,
            Some(&owner.scope_id),
            occurrence_range,
        )?;
        let target_spelling = import_target.rsplit('.').next().unwrap_or(&import_target);
        self.builder.relate(
            if is_reexport {
                CandidateRelation::Reexports
            } else {
                CandidateRelation::Imports
            },
            &owner.fact_id,
            Some(&occurrence_id),
            Some(&binding_id),
            target_spelling,
            ResolutionConstraint {
                exact_target_declaration_id: None,
                exact_language: Some(self.language.to_owned()),
                module_or_package: import_target
                    .rsplit_once('.')
                    .map(|(module, _)| module.to_owned()),
                scope_id: Some(owner.scope_id.clone()),
                qualified_name: Some(import_target.clone()),
                argument_count: None,
                argument_types: Vec::new(),
                allowed_target_kinds: if named_import {
                    vec![
                        "file".to_owned(),
                        "module".to_owned(),
                        "class".to_owned(),
                        "function".to_owned(),
                        "variable".to_owned(),
                    ]
                } else {
                    vec!["file".to_owned(), "module".to_owned(), "package".to_owned()]
                },
                hierarchy: None,
                allow_external: true,
            },
        )?;
        Ok(())
    }

    fn add_python_decorators(
        &mut self,
        declaration: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        let Some(parent) = declaration
            .parent()
            .filter(|node| node.kind() == "decorated_definition")
        else {
            return Ok(());
        };
        let mut cursor = parent.walk();
        for decorator in parent
            .children(&mut cursor)
            .filter(|child| child.kind() == "decorator")
        {
            let raw = self.text(decorator);
            let target = raw
                .trim()
                .trim_start_matches('@')
                .split('(')
                .next()
                .unwrap_or_default()
                .trim();
            if target.is_empty() {
                continue;
            }
            let (qualifier, spelling) = split_qualified(target);
            self.add_relationship_occurrence(
                SemanticRole::Decorator,
                CandidateRelation::Decorates,
                owner,
                spelling,
                qualifier,
                decorator,
            )?;
        }
        Ok(())
    }

    fn add_python_bases(
        &mut self,
        declaration: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        let Some(arguments) = declaration.child_by_field_name("superclasses") else {
            return Ok(());
        };
        let mut bases = PythonTypeBases {
            complete: true,
            qualified_names: Vec::new(),
        };
        let mut relationships = Vec::new();
        let mut cursor = arguments.walk();
        for argument in arguments
            .children(&mut cursor)
            .filter(|child| child.is_named() && child.kind() != "keyword_argument")
        {
            let target = match argument.kind() {
                "identifier" | "attribute" => Some(argument),
                "subscript" => argument
                    .child_by_field_name("value")
                    .filter(|value| matches!(value.kind(), "identifier" | "attribute")),
                _ => None,
            };
            let Some(target) = target else {
                bases.complete = false;
                continue;
            };
            let raw = self.text(target);
            let (qualifier, spelling) = split_qualified(&raw);
            let qualified_name =
                self.python_base_qualified_name(owner, qualifier, spelling, target.start_byte());
            if let Some(qualified_name) = qualified_name.as_ref() {
                bases.qualified_names.push(qualified_name.clone());
            } else {
                bases.complete = false;
            }
            relationships.push((
                target,
                spelling.to_owned(),
                qualifier.map(str::to_owned),
                qualified_name,
            ));
        }
        for (target, spelling, qualifier, qualified_name) in relationships {
            self.add_relationship_occurrence_with_hierarchy(
                SemanticRole::BaseType,
                CandidateRelation::Extends,
                owner,
                &spelling,
                qualifier.as_deref(),
                target,
                qualified_name,
                Some(HierarchyConstraint::DirectBase {
                    base_set_complete: bases.complete,
                }),
            )?;
        }
        bases.qualified_names.sort();
        bases.qualified_names.dedup();
        self.python_type_bases
            .insert(owner.qualified_name.clone(), bases);
        Ok(())
    }

    fn add_python_annotations(
        &mut self,
        declaration: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        let parameters = python_parameter_nodes(declaration);
        let implicit_receiver = owner.kind == "method"
            && !python_has_decorator(declaration, self.source, "staticmethod")
            && !parameters.is_empty();
        let mut canonical_parameter_types = Vec::new();
        let mut complete_parameter_types = true;
        for parameter in &parameters {
            let parameter_owner = self.python_parameters.get(&parameter.syntax.id()).cloned();
            let canonical = parameter
                .annotation
                .and_then(|annotation| self.python_canonical_annotation(owner, annotation));
            if let (Some(parameter_owner), Some(annotation), Some(canonical)) =
                (parameter_owner, parameter.annotation, canonical.as_ref())
            {
                self.add_python_exact_annotation_relationship(
                    &parameter_owner,
                    annotation,
                    CandidateRelation::TypeOf,
                    canonical,
                )?;
            }
            if !implicit_receiver || parameter.syntax.id() != parameters[0].syntax.id() {
                if let Some(canonical) = canonical {
                    canonical_parameter_types.push(canonical.canonical);
                } else {
                    complete_parameter_types = false;
                }
            }
        }
        if declaration.kind() == "function_definition" {
            if let Some(callable) = self
                .builder
                .batch
                .declarations
                .iter_mut()
                .find(|callable| callable.id == owner.fact_id)
            {
                let canonical_count = u32::try_from(canonical_parameter_types.len()).ok();
                callable.parameter_types =
                    if complete_parameter_types && callable.parameter_count == canonical_count {
                        canonical_parameter_types
                    } else {
                        Vec::new()
                    };
            }
            if let Some(return_type) = declaration.child_by_field_name("return_type")
                && let Some(canonical) = self.python_canonical_annotation(owner, return_type)
            {
                self.add_python_exact_annotation_relationship(
                    owner,
                    return_type,
                    CandidateRelation::Returns,
                    &canonical,
                )?;
                if let [returned] = canonical.runtime_targets.as_slice()
                    && returned != "builtins.NoneType"
                {
                    if self
                        .python_ambiguous_callable_returns
                        .contains(&owner.qualified_name)
                    {
                        // A same-named source overload already disagreed. Keep
                        // call-result inference fail closed.
                    } else if self
                        .python_callable_return_types
                        .get(&owner.qualified_name)
                        .is_some_and(|existing| existing != returned)
                    {
                        self.python_callable_return_types
                            .remove(&owner.qualified_name);
                        self.python_ambiguous_callable_returns
                            .insert(owner.qualified_name.clone());
                    } else {
                        self.python_callable_return_types
                            .insert(owner.qualified_name.clone(), returned.clone());
                    }
                }
            }
        }

        let mut annotation_roots = Vec::new();
        if let Some(return_type) = declaration.child_by_field_name("return_type") {
            annotation_roots.push(return_type);
        }
        if let Some(parameters) = declaration.child_by_field_name("parameters") {
            let mut cursor = parameters.walk();
            for parameter in parameters.children(&mut cursor) {
                if let Some(annotation) = parameter.child_by_field_name("type") {
                    annotation_roots.push(annotation);
                }
            }
        }
        for annotation in annotation_roots {
            let mut targets = Vec::new();
            collect_named_targets(annotation, &["identifier", "attribute"], &mut targets);
            for target in targets {
                let raw = self.text(target);
                if is_python_builtin_type(&raw) {
                    continue;
                }
                let (qualifier, spelling) = split_qualified(&raw);
                self.add_relationship_occurrence(
                    SemanticRole::Annotation,
                    CandidateRelation::Annotates,
                    owner,
                    spelling,
                    qualifier,
                    target,
                )?;
            }
        }
        Ok(())
    }

    fn add_python_exact_annotation_relationship(
        &mut self,
        owner: &DeclarationContext,
        annotation: Node<'_>,
        relation: CandidateRelation,
        canonical: &PythonCanonicalAnnotation,
    ) -> Result<(), EvidenceError> {
        for qualified_name in &canonical.runtime_targets {
            let spelling = qualified_name
                .rsplit(['.', ':'])
                .find(|part| !part.is_empty())
                .unwrap_or(qualified_name);
            let local_target = self
                .builder
                .batch
                .declarations
                .iter()
                .filter(|declaration| {
                    declaration.language == "python"
                        && declaration.qualified_name == *qualified_name
                        && matches!(
                            declaration.kind.as_str(),
                            "class" | "type_alias" | "parameter"
                        )
                })
                .map(|declaration| declaration.id.clone())
                .collect::<Vec<_>>();
            let exact_target_declaration_id = match local_target.as_slice() {
                [target] => Some(target.clone()),
                _ => None,
            };
            let occurrence_id = self.builder.occur_with_context(
                SemanticRole::TypeReference,
                &owner.fact_id,
                spelling,
                None,
                Some(&owner.scope_id),
                Some("exact_python_annotation"),
                range_for_node(self.source_file, annotation),
            )?;
            self.builder.relate(
                relation,
                &owner.fact_id,
                Some(&occurrence_id),
                None,
                spelling,
                ResolutionConstraint {
                    exact_target_declaration_id: exact_target_declaration_id.clone(),
                    exact_language: Some(self.language.to_owned()),
                    module_or_package: qualified_name
                        .rsplit_once('.')
                        .map(|(module, _)| module.to_owned()),
                    scope_id: Some(owner.scope_id.clone()),
                    qualified_name: Some(qualified_name.clone()),
                    argument_count: None,
                    argument_types: Vec::new(),
                    allowed_target_kinds: target_kinds_for_relation(relation),
                    hierarchy: None,
                    allow_external: exact_target_declaration_id.is_none(),
                },
            )?;
        }
        Ok(())
    }

    fn python_canonical_annotation(
        &self,
        owner: &DeclarationContext,
        annotation: Node<'_>,
    ) -> Option<PythonCanonicalAnnotation> {
        let raw = self.text(annotation);
        (raw.len() <= 1_024)
            .then(|| self.python_canonical_annotation_raw(owner, &raw, annotation.start_byte(), 0))
            .flatten()
    }

    fn python_canonical_annotation_raw(
        &self,
        owner: &DeclarationContext,
        raw: &str,
        use_start: usize,
        depth: usize,
    ) -> Option<PythonCanonicalAnnotation> {
        if depth >= 16 {
            return None;
        }
        let raw = raw.trim();
        if raw.len() >= 2
            && ((raw.starts_with('"') && raw.ends_with('"'))
                || (raw.starts_with('\'') && raw.ends_with('\'')))
        {
            let inner = raw.get(1..raw.len().saturating_sub(1))?;
            if inner.contains(['\\', '\n', '\r']) {
                return None;
            }
            return self.python_canonical_annotation_raw(owner, inner, use_start, depth + 1);
        }
        let union = split_python_annotation_top_level(raw, '|')?;
        if union.len() > 1 {
            let mut canonical = Vec::new();
            let mut runtime_targets = Vec::new();
            for member in union {
                let member =
                    self.python_canonical_annotation_raw(owner, member, use_start, depth + 1)?;
                canonical.push(member.canonical);
                runtime_targets.extend(member.runtime_targets);
            }
            canonical.sort_unstable();
            canonical.dedup();
            runtime_targets.sort_unstable();
            runtime_targets.dedup();
            return Some(PythonCanonicalAnnotation {
                canonical: canonical.join(" | "),
                runtime_targets,
            });
        }
        if let Some(open) = python_annotation_generic_open(raw) {
            let base_raw = raw.get(..open)?.trim();
            let arguments_raw = raw.get(open + 1..raw.len().saturating_sub(1))?;
            let base = self.python_nominal_annotation(owner, base_raw, use_start)?;
            let arguments = split_python_annotation_top_level(arguments_raw, ',')?;
            if base == "typing.Annotated" {
                let first = arguments.first()?;
                return self.python_canonical_annotation_raw(owner, first, use_start, depth + 1);
            }
            if base == "typing.Optional" {
                let [argument] = arguments.as_slice() else {
                    return None;
                };
                let mut inner =
                    self.python_canonical_annotation_raw(owner, argument, use_start, depth + 1)?;
                inner.canonical = format!("{} | builtins.NoneType", inner.canonical);
                return Some(inner);
            }
            if base == "typing.Union" {
                let joined = arguments.join(" | ");
                return self.python_canonical_annotation_raw(owner, &joined, use_start, depth + 1);
            }
            if base == "typing.Literal" || arguments.is_empty() {
                return None;
            }
            let mut canonical_arguments = Vec::new();
            for argument in arguments {
                canonical_arguments.push(
                    self.python_canonical_annotation_raw(owner, argument, use_start, depth + 1)?
                        .canonical,
                );
            }
            return Some(PythonCanonicalAnnotation {
                canonical: format!("{base}[{}]", canonical_arguments.join(", ")),
                runtime_targets: vec![base],
            });
        }
        let canonical = self.python_nominal_annotation(owner, raw, use_start)?;
        let runtime_targets = if canonical == "builtins.NoneType" {
            Vec::new()
        } else {
            vec![canonical.clone()]
        };
        Some(PythonCanonicalAnnotation {
            canonical,
            runtime_targets,
        })
    }

    fn python_nominal_annotation(
        &self,
        owner: &DeclarationContext,
        raw: &str,
        use_start: usize,
    ) -> Option<String> {
        let raw = raw.trim();
        if matches!(raw, "Any" | "typing.Any") {
            return None;
        }
        if let Some(builtin) = python_builtin_annotation(raw) {
            return Some(builtin.to_owned());
        }
        let (head, suffix) = split_qualified_head(raw);
        if let Some(imported) = self.imported_target_for_occurrence(owner, head, use_start, false) {
            let target =
                suffix.map_or_else(|| imported.clone(), |suffix| format!("{imported}.{suffix}"));
            if target == "typing.Any" {
                return None;
            }
            return Some(python_normalize_typing_alias(&target));
        }
        if head == "builtins" {
            return suffix
                .and_then(python_builtin_annotation)
                .map(str::to_owned);
        }
        let local = self
            .builder
            .batch
            .declarations
            .iter()
            .filter(|declaration| {
                declaration.language == "python"
                    && declaration.name == raw
                    && declaration.range.start_byte < u64::try_from(use_start).unwrap_or(u64::MAX)
                    && matches!(
                        declaration.kind.as_str(),
                        "class" | "type_alias" | "parameter"
                    )
            })
            .map(|declaration| declaration.qualified_name.clone())
            .collect::<BTreeSet<_>>();
        match local.len() {
            1 => local.into_iter().next(),
            _ => None,
        }
    }

    fn extract_java(&mut self, root: Node<'_>) -> Result<(), EvidenceError> {
        let file = self.file.clone().ok_or_else(|| {
            EvidenceError::new(
                EvidenceErrorCode::InvalidFact,
                "non-empty Java source has no file evidence",
            )
        })?;
        self.collect_java_imports(root, &file)?;
        self.collect_java_declarations(root, &file)?;
        self.collect_java_value_types(root, &file)?;
        self.walk_java_evidence(root, &file, true)
    }

    fn collect_java_imports(
        &mut self,
        node: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        if node.kind() == "import_declaration" {
            return self.add_java_import(node, owner);
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(|child| child.is_named()) {
            self.collect_java_imports(child, owner)?;
        }
        Ok(())
    }

    fn add_java_import(
        &mut self,
        node: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        let raw = self.text(node);
        let Some(mut target) = raw
            .strip_prefix("import")
            .map(str::trim)
            .map(|value| value.trim_end_matches(';').trim())
        else {
            return Ok(());
        };
        let is_static = target.starts_with("static ");
        if is_static {
            target = target.trim_start_matches("static ").trim();
        }
        if target.is_empty() {
            return Ok(());
        }
        let wildcard = target.ends_with(".*");
        let local = if wildcard {
            "*"
        } else {
            target.rsplit('.').next().unwrap_or(target)
        };
        let kind = if local == target {
            BindingKind::Import
        } else {
            BindingKind::ImportAlias
        };
        let target_node = last_java_import_name(node).unwrap_or(node);
        let binding_id = self.builder.bind(
            kind,
            local,
            target,
            None,
            Some(&owner.scope_id),
            range_for_node(self.source_file, target_node),
        )?;
        self.record_import_binding(owner, local, target, &binding_id, 0);
        let occurrence_id = self.builder.occur_with_context(
            SemanticRole::Import,
            &owner.fact_id,
            local,
            None,
            Some(&owner.scope_id),
            is_static.then_some("static"),
            range_for_node(self.source_file, target_node),
        )?;
        self.builder.relate(
            CandidateRelation::Imports,
            &owner.fact_id,
            Some(&occurrence_id),
            Some(&binding_id),
            local,
            ResolutionConstraint {
                exact_target_declaration_id: None,
                exact_language: Some(self.language.to_owned()),
                module_or_package: java_qualified_parent(target).map(str::to_owned),
                scope_id: Some(owner.scope_id.clone()),
                qualified_name: Some(target.to_owned()),
                argument_count: None,
                argument_types: Vec::new(),
                allowed_target_kinds: vec![
                    "file".to_owned(),
                    "package".to_owned(),
                    "class".to_owned(),
                    "interface".to_owned(),
                    "enum".to_owned(),
                    "record".to_owned(),
                    "annotation_type".to_owned(),
                    "method".to_owned(),
                    "field".to_owned(),
                ],
                hierarchy: None,
                allow_external: true,
            },
        )?;
        Ok(())
    }

    fn collect_java_declarations(
        &mut self,
        node: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        if let Some(kind) = java_container_kind(node.kind()) {
            let Some(name_node) = node.child_by_field_name("name") else {
                return Ok(());
            };
            let name = self.text(name_node);
            if name.is_empty() {
                return Ok(());
            }
            let qualified_name = java_child_qualified_name(owner, &name);
            let graph_node_id = self.unique_graph_id(
                make_id(&[&self.module_or_package, &qualified_name, kind]),
                node,
            );
            let mut metadata = self.declaration_metadata(node);
            metadata.direct_bases_complete = !self.overlaps_parser_error(node);
            let fact_id = self.builder.declare_with_metadata(
                kind,
                &graph_node_id,
                &name,
                &qualified_name,
                Some(&self.module_or_package),
                Some(&owner.scope_id),
                range_for_node(self.source_file, name_node),
                metadata,
            )?;
            let scope_id = self.builder.open_scope(
                kind,
                Some(&fact_id),
                Some(&owner.scope_id),
                range_for_node(self.source_file, node),
            )?;
            self.scope_parents
                .insert(scope_id.clone(), owner.scope_id.clone());
            let context = DeclarationContext {
                fact_id,
                scope_id,
                graph_node_id,
                name: name.clone(),
                qualified_name: qualified_name.clone(),
                kind: kind.to_owned(),
                enclosing_type_qualified_name: Some(qualified_name.clone()),
            };
            self.add_ownership(owner, &context)?;
            self.local_targets
                .entry(owner.scope_id.clone())
                .or_default()
                .insert(name, qualified_name);
            self.java_containers.insert(node.id(), context.clone());
            self.declarations.insert(node.id(), context.clone());
            let body = node.child_by_field_name("body").unwrap_or(node);
            let mut cursor = body.walk();
            for child in body.children(&mut cursor).filter(|child| child.is_named()) {
                self.collect_java_member_declarations(child, &context)?;
            }
            return Ok(());
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(|child| child.is_named()) {
            self.collect_java_declarations(child, owner)?;
        }
        Ok(())
    }

    fn collect_java_member_declarations(
        &mut self,
        node: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        if java_container_kind(node.kind()).is_some() {
            return self.collect_java_declarations(node, owner);
        }
        match node.kind() {
            "method_declaration" | "constructor_declaration" => {
                self.add_java_callable_declaration(node, owner)?;
                return Ok(());
            }
            "field_declaration" | "constant_declaration" => {
                self.add_java_field_declarations(node, owner)?;
                return Ok(());
            }
            "enum_constant" => {
                let enum_member = self.add_java_enum_member(node, owner)?;
                if let Some(enum_member) = enum_member {
                    let body = node.child_by_field_name("body").or_else(|| {
                        let mut cursor = node.walk();
                        node.children(&mut cursor)
                            .find(|child| child.kind() == "class_body")
                    });
                    if let Some(body) = body {
                        let mut cursor = body.walk();
                        for child in body.children(&mut cursor).filter(|child| child.is_named()) {
                            self.collect_java_member_declarations(child, &enum_member)?;
                        }
                    }
                }
                return Ok(());
            }
            _ => {}
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(|child| child.is_named()) {
            self.collect_java_member_declarations(child, owner)?;
        }
        Ok(())
    }

    fn add_java_callable_declaration(
        &mut self,
        node: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        let constructor = node.kind() == "constructor_declaration";
        let Some(name_node) = node.child_by_field_name("name") else {
            return Ok(());
        };
        let source_name = self.text(name_node);
        if source_name.is_empty() {
            return Ok(());
        }
        let name = if constructor { "<init>" } else { &source_name };
        let (parameters, parameter_count, variadic, raw_parameter_types) =
            java_parameter_signature(node, self.source);
        let signature = format!("{name}({parameters})");
        let qualified_name = format!("{}::{name}", owner.qualified_name);
        let graph_node_id = self.unique_graph_id(
            make_id(&[
                &self.module_or_package,
                &owner.qualified_name,
                if constructor { "constructor" } else { "method" },
                name,
                &parameters,
            ]),
            node,
        );
        let mut metadata = self.declaration_metadata(node);
        metadata.signature = Some(signature);
        metadata.parameter_count = Some(parameter_count);
        metadata.parameter_types = raw_parameter_types
            .iter()
            .filter_map(|parameter| self.java_canonical_type(owner, parameter, node.start_byte()))
            .collect();
        if metadata.parameter_types.len() != raw_parameter_types.len() {
            metadata.parameter_types.clear();
        }
        metadata.variadic = variadic;
        let kind = if constructor { "constructor" } else { "method" };
        let fact_id = self.builder.declare_with_metadata(
            kind,
            &graph_node_id,
            name,
            &qualified_name,
            Some(&self.module_or_package),
            Some(&owner.scope_id),
            range_for_node(self.source_file, name_node),
            metadata,
        )?;
        let scope_id = self.builder.open_scope(
            kind,
            Some(&fact_id),
            Some(&owner.scope_id),
            range_for_node(self.source_file, node),
        )?;
        self.scope_parents
            .insert(scope_id.clone(), owner.scope_id.clone());
        let context = DeclarationContext {
            fact_id: fact_id.clone(),
            scope_id,
            graph_node_id,
            name: name.to_owned(),
            qualified_name: qualified_name.clone(),
            kind: kind.to_owned(),
            enclosing_type_qualified_name: Some(owner.qualified_name.clone()),
        };
        let occurrence_id = self.builder.occur_with_context(
            SemanticRole::Ownership,
            &owner.fact_id,
            name,
            None,
            Some(&owner.scope_id),
            None,
            range_for_node(self.source_file, name_node),
        )?;
        self.builder.relate(
            CandidateRelation::Contains,
            &owner.fact_id,
            Some(&occurrence_id),
            None,
            name,
            ResolutionConstraint {
                exact_target_declaration_id: Some(fact_id.clone()),
                exact_language: Some(self.language.to_owned()),
                module_or_package: Some(self.module_or_package.clone()),
                scope_id: Some(owner.scope_id.clone()),
                qualified_name: Some(qualified_name.clone()),
                argument_count: Some(parameter_count),
                argument_types: Vec::new(),
                allowed_target_kinds: vec![kind.to_owned()],
                hierarchy: None,
                allow_external: false,
            },
        )?;
        let binding_id = self.builder.bind(
            BindingKind::Member,
            name,
            &qualified_name,
            Some(&fact_id),
            Some(&owner.scope_id),
            range_for_node(self.source_file, name_node),
        )?;
        let local_bindings = self
            .local_bindings
            .entry(owner.scope_id.clone())
            .or_default();
        if local_bindings.contains_key(name) {
            self.ambiguous_bindings
                .insert((owner.scope_id.clone(), name.to_owned()));
        } else {
            local_bindings.insert(name.to_owned(), binding_id);
        }
        self.declarations.insert(node.id(), context);
        Ok(())
    }

    fn add_java_field_declarations(
        &mut self,
        node: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        let declared_type = node
            .child_by_field_name("type")
            .map(|type_node| java_normalize_type(&self.text(type_node)));
        let mut declarators = Vec::new();
        collect_direct_or_nested_nodes(node, "variable_declarator", &mut declarators);
        for declarator in declarators {
            let Some(name_node) = declarator.child_by_field_name("name") else {
                continue;
            };
            let name = self.text(name_node);
            if name.is_empty() {
                continue;
            }
            let qualified_name = format!("{}::{name}", owner.qualified_name);
            let graph_node_id = self.unique_graph_id(
                make_id(&[&self.module_or_package, &qualified_name, "field"]),
                declarator,
            );
            let fact_id = self.builder.declare(
                "field",
                &graph_node_id,
                &name,
                &qualified_name,
                Some(&self.module_or_package),
                Some(&owner.scope_id),
                range_for_node(self.source_file, name_node),
            )?;
            let context = DeclarationContext {
                fact_id: fact_id.clone(),
                scope_id: owner.scope_id.clone(),
                graph_node_id,
                name: name.clone(),
                qualified_name: qualified_name.clone(),
                kind: "field".to_owned(),
                enclosing_type_qualified_name: Some(owner.qualified_name.clone()),
            };
            self.add_ownership(owner, &context)?;
            self.builder.bind(
                BindingKind::Member,
                &name,
                &qualified_name,
                Some(&fact_id),
                Some(&owner.scope_id),
                range_for_node(self.source_file, name_node),
            )?;
            if let Some(declared_type) = declared_type.as_ref() {
                self.java_value_types
                    .entry(owner.scope_id.clone())
                    .or_default()
                    .insert(name, declared_type.clone());
            }
            self.declarations.insert(declarator.id(), context.clone());
            if let Some(value) = declarator.child_by_field_name("value") {
                let mut creations = Vec::new();
                collect_nodes(value, "object_creation_expression", &mut creations);
                let body = creations.first().and_then(|creation| {
                    creation.child_by_field_name("body").or_else(|| {
                        let mut bodies = Vec::new();
                        collect_nodes(*creation, "class_body", &mut bodies);
                        bodies.into_iter().next()
                    })
                });
                if let Some(body) = body {
                    let mut cursor = body.walk();
                    for child in body.children(&mut cursor).filter(|child| child.is_named()) {
                        self.collect_java_member_declarations(child, &context)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn add_java_enum_member(
        &mut self,
        node: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<Option<DeclarationContext>, EvidenceError> {
        let Some(name_node) = node.child_by_field_name("name") else {
            return Ok(None);
        };
        let name = self.text(name_node);
        if name.is_empty() {
            return Ok(None);
        }
        let qualified_name = format!("{}::{name}", owner.qualified_name);
        let graph_node_id = self.unique_graph_id(
            make_id(&[&self.module_or_package, &qualified_name, "enum_member"]),
            node,
        );
        let fact_id = self.builder.declare(
            "enum_member",
            &graph_node_id,
            &name,
            &qualified_name,
            Some(&self.module_or_package),
            Some(&owner.scope_id),
            range_for_node(self.source_file, name_node),
        )?;
        let scope_id = self.builder.open_scope(
            "enum_member",
            Some(&fact_id),
            Some(&owner.scope_id),
            range_for_node(self.source_file, node),
        )?;
        self.scope_parents
            .insert(scope_id.clone(), owner.scope_id.clone());
        let context = DeclarationContext {
            fact_id: fact_id.clone(),
            scope_id,
            graph_node_id,
            name: name.clone(),
            qualified_name: qualified_name.clone(),
            kind: "enum_member".to_owned(),
            enclosing_type_qualified_name: Some(owner.qualified_name.clone()),
        };
        self.add_ownership(owner, &context)?;
        self.builder.bind(
            BindingKind::Member,
            &name,
            &qualified_name,
            Some(&fact_id),
            Some(&owner.scope_id),
            range_for_node(self.source_file, name_node),
        )?;
        self.declarations.insert(node.id(), context.clone());
        Ok(Some(context))
    }

    fn collect_java_value_types(
        &mut self,
        node: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        let active = self
            .declarations
            .get(&node.id())
            .cloned()
            .unwrap_or_else(|| owner.clone());
        if matches!(
            node.kind(),
            "method_declaration" | "constructor_declaration"
        ) {
            self.add_java_parameter_value_types(node, &active);
        } else if node.kind() == "local_variable_declaration" {
            self.add_java_local_value_types(node, &active);
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(|child| child.is_named()) {
            self.collect_java_value_types(child, &active)?;
        }
        Ok(())
    }

    fn add_java_parameter_value_types(&mut self, node: Node<'_>, owner: &DeclarationContext) {
        let Some(parameters) = node.child_by_field_name("parameters") else {
            return;
        };
        let mut cursor = parameters.walk();
        for parameter in parameters
            .children(&mut cursor)
            .filter(|child| child.is_named())
        {
            if !matches!(parameter.kind(), "formal_parameter" | "spread_parameter") {
                continue;
            }
            let Some(name) = parameter
                .child_by_field_name("name")
                .map(|node| self.text(node))
            else {
                continue;
            };
            let Some(target) = parameter
                .child_by_field_name("type")
                .map(|node| java_normalize_type(&self.text(node)))
            else {
                continue;
            };
            self.java_value_types
                .entry(owner.scope_id.clone())
                .or_default()
                .insert(name, target);
        }
    }

    fn add_java_local_value_types(&mut self, node: Node<'_>, owner: &DeclarationContext) {
        let Some(target) = node
            .child_by_field_name("type")
            .map(|node| java_normalize_type(&self.text(node)))
        else {
            return;
        };
        let mut declarators = Vec::new();
        collect_direct_or_nested_nodes(node, "variable_declarator", &mut declarators);
        for declarator in declarators {
            if let Some(name) = declarator
                .child_by_field_name("name")
                .map(|node| self.text(node))
            {
                self.java_value_types
                    .entry(owner.scope_id.clone())
                    .or_default()
                    .insert(name, target.clone());
            }
        }
    }

    fn walk_java_evidence(
        &mut self,
        node: Node<'_>,
        owner: &DeclarationContext,
        root: bool,
    ) -> Result<(), EvidenceError> {
        let active = self
            .declarations
            .get(&node.id())
            .cloned()
            .unwrap_or_else(|| owner.clone());
        if self.declarations.contains_key(&node.id()) {
            self.add_java_annotations(node, &active)?;
            self.add_java_type_relationships(node, &active)?;
        }
        if matches!(node.kind(), "field_declaration" | "constant_declaration") {
            let mut declarators = Vec::new();
            collect_direct_or_nested_nodes(node, "variable_declarator", &mut declarators);
            for declarator in declarators {
                if let Some(field) = self.declarations.get(&declarator.id()).cloned() {
                    self.add_java_annotations(node, &field)?;
                    self.add_java_type_relationships(node, &field)?;
                }
            }
        }
        if matches!(
            node.kind(),
            "method_declaration" | "constructor_declaration"
        ) {
            self.add_java_parameter_annotations(node, &active)?;
        }
        if node.kind() == "annotation_type_element_declaration" {
            self.add_java_annotations(node, &active)?;
        }
        match node.kind() {
            "import_declaration" => return Ok(()),
            "method_invocation" => self.add_java_method_call(node, &active)?,
            "object_creation_expression" => self.add_java_construction(node, &active)?,
            _ => {}
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(|child| child.is_named()) {
            if !root && child.id() == node.id() {
                continue;
            }
            self.walk_java_evidence(child, &active, false)?;
        }
        Ok(())
    }

    fn add_java_annotations(
        &mut self,
        node: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        let Some(modifiers) = node.child_by_field_name("modifiers").or_else(|| {
            let mut cursor = node.walk();
            node.children(&mut cursor)
                .find(|child| child.kind() == "modifiers")
        }) else {
            return Ok(());
        };
        let mut annotations = Vec::new();
        collect_java_annotations(modifiers, &mut annotations);
        for annotation in annotations {
            let Some(name_node) = annotation
                .child_by_field_name("name")
                .or_else(|| first_java_type_name(annotation))
            else {
                continue;
            };
            self.add_java_named_relationship(
                SemanticRole::Annotation,
                CandidateRelation::Annotates,
                owner,
                name_node,
                Some("annotation"),
                vec!["annotation_type", "interface", "class"],
            )?;
        }
        Ok(())
    }

    fn add_java_parameter_annotations(
        &mut self,
        node: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        let Some(parameters) = node.child_by_field_name("parameters") else {
            return Ok(());
        };
        let mut parameter_nodes = Vec::new();
        collect_nodes(parameters, "formal_parameter", &mut parameter_nodes);
        collect_nodes(parameters, "spread_parameter", &mut parameter_nodes);
        parameter_nodes.sort_by_key(Node::start_byte);
        parameter_nodes.dedup_by_key(|parameter| parameter.id());
        for parameter in parameter_nodes {
            self.add_java_annotations(parameter, owner)?;
        }
        Ok(())
    }

    fn add_java_type_relationships(
        &mut self,
        node: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        if java_container_kind(node.kind()).is_some() {
            if let Some(superclass) = node.child_by_field_name("superclass") {
                let mut names = Vec::new();
                collect_java_type_nodes(superclass, &mut names);
                if let Some(target) = names.first().copied() {
                    self.add_java_named_relationship(
                        SemanticRole::BaseType,
                        CandidateRelation::Extends,
                        owner,
                        target,
                        Some("superclass"),
                        vec!["class", "record"],
                    )?;
                }
            }
            let interfaces = node.child_by_field_name("interfaces").or_else(|| {
                let mut cursor = node.walk();
                node.children(&mut cursor)
                    .find(|child| child.kind() == "extends_interfaces")
            });
            if let Some(interfaces) = interfaces {
                let mut names = Vec::new();
                collect_java_direct_supertype_nodes(interfaces, &mut names);
                for target in names {
                    let interface_extends = node.kind() == "interface_declaration";
                    self.add_java_named_relationship(
                        if interface_extends {
                            SemanticRole::BaseType
                        } else {
                            SemanticRole::TypeReference
                        },
                        if interface_extends {
                            CandidateRelation::Extends
                        } else {
                            CandidateRelation::Implements
                        },
                        owner,
                        target,
                        Some(if interface_extends {
                            "superinterface"
                        } else {
                            "interface"
                        }),
                        vec!["interface", "annotation_type"],
                    )?;
                }
            }
            if let Some(interfaces) = node.child_by_field_name("extends_interfaces").or_else(|| {
                let mut cursor = node.walk();
                node.children(&mut cursor)
                    .find(|child| child.kind() == "extends_interfaces")
            }) {
                let mut names = Vec::new();
                collect_java_direct_supertype_nodes(interfaces, &mut names);
                for target in names {
                    self.add_java_named_relationship(
                        SemanticRole::BaseType,
                        CandidateRelation::Extends,
                        owner,
                        target,
                        Some("superinterface"),
                        vec!["interface", "annotation_type"],
                    )?;
                }
            }
        }
        let type_root = match node.kind() {
            "method_declaration" => node.child_by_field_name("type"),
            "field_declaration"
            | "constant_declaration"
            | "formal_parameter"
            | "spread_parameter" => node.child_by_field_name("type"),
            _ => None,
        };
        if let Some(type_root) = type_root {
            let mut names = Vec::new();
            collect_java_type_nodes(type_root, &mut names);
            for target in names {
                self.add_java_named_relationship(
                    SemanticRole::TypeReference,
                    CandidateRelation::References,
                    owner,
                    target,
                    Some("type"),
                    vec!["class", "interface", "enum", "record", "annotation_type"],
                )?;
            }
        }
        if matches!(
            node.kind(),
            "method_declaration" | "constructor_declaration"
        ) && let Some(parameters) = node.child_by_field_name("parameters")
        {
            let mut parameter_types = Vec::new();
            collect_java_parameter_type_nodes(parameters, &mut parameter_types);
            for target in parameter_types {
                self.add_java_named_relationship(
                    SemanticRole::TypeReference,
                    CandidateRelation::References,
                    owner,
                    target,
                    Some("parameter_type"),
                    vec!["class", "interface", "enum", "record", "annotation_type"],
                )?;
            }
        }
        Ok(())
    }

    fn add_java_named_relationship(
        &mut self,
        role: SemanticRole,
        relation: CandidateRelation,
        owner: &DeclarationContext,
        target: Node<'_>,
        context: Option<&str>,
        kinds: Vec<&str>,
    ) -> Result<(), EvidenceError> {
        let raw = self.text(target);
        let normalized = java_normalize_type(&raw);
        let (qualifier, spelling) = split_qualified(&normalized);
        if spelling.is_empty() || java_primitive_type(spelling) {
            return Ok(());
        }
        let qualified_name = self.java_qualified_type(owner, &normalized, target.start_byte());
        let lookup = qualifier.map(qualified_binding_head).unwrap_or(spelling);
        let binding = self
            .binding_for_occurrence(owner, lookup, target.start_byte(), true)
            .cloned();
        let occurrence_id = self.builder.occur_with_context(
            role,
            &owner.fact_id,
            spelling,
            qualifier,
            Some(&owner.scope_id),
            context,
            range_for_node(self.source_file, target),
        )?;
        self.builder.relate(
            relation,
            &owner.fact_id,
            Some(&occurrence_id),
            binding.as_deref(),
            spelling,
            ResolutionConstraint {
                exact_target_declaration_id: None,
                exact_language: Some(self.language.to_owned()),
                module_or_package: Some(self.module_or_package.clone()),
                scope_id: Some(owner.scope_id.clone()),
                qualified_name: qualified_name.clone(),
                argument_count: None,
                argument_types: Vec::new(),
                allowed_target_kinds: kinds.into_iter().map(str::to_owned).collect(),
                hierarchy: None,
                allow_external: qualified_name.is_some(),
            },
        )?;
        Ok(())
    }

    fn add_java_method_call(
        &mut self,
        node: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        let Some(name_node) = node.child_by_field_name("name") else {
            return Ok(());
        };
        let spelling = self.text(name_node);
        if spelling.is_empty() {
            return Ok(());
        }
        let receiver = node
            .child_by_field_name("object")
            .map(|object| self.text(object));
        let receiver_type = receiver
            .as_deref()
            .and_then(|receiver| self.java_receiver_type(owner, receiver, node.start_byte()));
        let qualified_name = receiver_type
            .as_ref()
            .map(|receiver| format!("{receiver}::{spelling}"));
        let lookup = receiver
            .as_deref()
            .map(qualified_binding_head)
            .unwrap_or(&spelling);
        let binding = self
            .binding_for_occurrence(owner, lookup, node.start_byte(), true)
            .cloned();
        let argument_count = java_argument_count(node);
        let argument_types = self.java_argument_types(node, owner);
        let occurrence_id = self.builder.occur_with_context(
            SemanticRole::Call,
            &owner.fact_id,
            &spelling,
            receiver.as_deref(),
            Some(&owner.scope_id),
            Some(&format!("arity:{argument_count}")),
            range_for_node(self.source_file, name_node),
        )?;
        self.builder.relate(
            CandidateRelation::Calls,
            &owner.fact_id,
            Some(&occurrence_id),
            binding.as_deref(),
            &spelling,
            ResolutionConstraint {
                exact_target_declaration_id: None,
                exact_language: Some(self.language.to_owned()),
                module_or_package: Some(self.module_or_package.clone()),
                scope_id: Some(owner.scope_id.clone()),
                qualified_name,
                argument_count: Some(argument_count),
                argument_types,
                allowed_target_kinds: vec!["method".to_owned()],
                hierarchy: None,
                allow_external: receiver_type.is_some(),
            },
        )?;
        Ok(())
    }

    fn add_java_construction(
        &mut self,
        node: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        let Some(type_node) = node.child_by_field_name("type") else {
            return Ok(());
        };
        let normalized = java_normalize_type(&self.text(type_node));
        let (qualifier, spelling) = split_qualified(&normalized);
        if spelling.is_empty() {
            return Ok(());
        }
        let qualified_name = self.java_qualified_type(owner, &normalized, type_node.start_byte());
        let lookup = qualifier.map(qualified_binding_head).unwrap_or(spelling);
        let binding = self
            .binding_for_occurrence(owner, lookup, type_node.start_byte(), true)
            .cloned();
        let argument_count = java_argument_count(node);
        let argument_types = self.java_argument_types(node, owner);
        let occurrence_id = self.builder.occur_with_context(
            SemanticRole::Construction,
            &owner.fact_id,
            spelling,
            qualifier,
            Some(&owner.scope_id),
            Some(&format!("arity:{argument_count}")),
            range_for_node(self.source_file, type_node),
        )?;
        self.builder.relate(
            CandidateRelation::Constructs,
            &owner.fact_id,
            Some(&occurrence_id),
            binding.as_deref(),
            spelling,
            ResolutionConstraint {
                exact_target_declaration_id: None,
                exact_language: Some(self.language.to_owned()),
                module_or_package: Some(self.module_or_package.clone()),
                scope_id: Some(owner.scope_id.clone()),
                qualified_name,
                argument_count: Some(argument_count),
                argument_types,
                allowed_target_kinds: vec!["class".to_owned(), "record".to_owned()],
                hierarchy: None,
                allow_external: true,
            },
        )?;
        Ok(())
    }

    fn java_qualified_type(
        &self,
        owner: &DeclarationContext,
        raw: &str,
        use_start: usize,
    ) -> Option<String> {
        let normalized = java_normalize_type(raw);
        if normalized.is_empty() || java_primitive_type(&normalized) {
            return None;
        }
        if normalized.contains('.') && normalized.starts_with(char::is_lowercase) {
            return Some(normalized);
        }
        let (qualifier, spelling) = split_qualified(&normalized);
        if let Some(qualifier) = qualifier {
            return self
                .imported_qualified_target_for(owner, qualifier, use_start, true)
                .map(|target| format!("{target}.{spelling}"))
                .or(Some(normalized));
        }
        self.imported_target_for_occurrence(owner, spelling, use_start, true)
            .cloned()
            .or_else(|| self.local_target_for(owner, spelling).cloned())
            .or_else(|| java_lang_type(spelling))
            .or_else(|| Some(format!("{}.{}", self.module_or_package, spelling)))
    }

    fn java_canonical_type(
        &self,
        owner: &DeclarationContext,
        raw: &str,
        use_start: usize,
    ) -> Option<String> {
        let mut normalized = java_normalize_type(raw);
        if normalized.is_empty() || normalized == "var" {
            return None;
        }
        let mut suffix = String::new();
        while normalized.ends_with("[]") {
            normalized.truncate(normalized.len().saturating_sub(2));
            suffix.push_str("[]");
        }
        if java_primitive_type(&normalized) {
            return Some(format!("{normalized}{suffix}"));
        }
        self.java_qualified_type(owner, &normalized, use_start)
            .map(|qualified| format!("{qualified}{suffix}"))
    }

    fn java_argument_types(
        &self,
        call: Node<'_>,
        owner: &DeclarationContext,
    ) -> Vec<Option<String>> {
        let Some(arguments) = call.child_by_field_name("arguments") else {
            return Vec::new();
        };
        let mut cursor = arguments.walk();
        arguments
            .children(&mut cursor)
            .filter(|child| child.is_named())
            .map(|argument| self.java_expression_type(owner, argument, 0))
            .collect()
    }

    fn java_expression_type(
        &self,
        owner: &DeclarationContext,
        expression: Node<'_>,
        depth: usize,
    ) -> Option<String> {
        if depth >= 8 {
            return None;
        }
        match expression.kind() {
            "identifier" => self
                .local_java_value_type(owner, &self.text(expression))
                .and_then(|target| {
                    self.java_canonical_type(owner, target, expression.start_byte())
                }),
            "string_literal" => Some("java.lang.String".to_owned()),
            "character_literal" => Some("char".to_owned()),
            "true" | "false" => Some("boolean".to_owned()),
            "null_literal" => Some("null".to_owned()),
            "decimal_integer_literal"
            | "hex_integer_literal"
            | "octal_integer_literal"
            | "binary_integer_literal" => Some(
                if self.text(expression).ends_with(['l', 'L']) {
                    "long"
                } else {
                    "int"
                }
                .to_owned(),
            ),
            "decimal_floating_point_literal" | "hex_floating_point_literal" => Some(
                if self.text(expression).ends_with(['f', 'F']) {
                    "float"
                } else {
                    "double"
                }
                .to_owned(),
            ),
            "object_creation_expression" | "array_creation_expression" => {
                expression.child_by_field_name("type").and_then(|target| {
                    self.java_canonical_type(owner, &self.text(target), target.start_byte())
                })
            }
            "cast_expression" => expression.child_by_field_name("type").and_then(|target| {
                self.java_canonical_type(owner, &self.text(target), target.start_byte())
            }),
            "class_literal" => Some("java.lang.Class".to_owned()),
            "parenthesized_expression" => expression
                .named_child(0)
                .and_then(|inner| self.java_expression_type(owner, inner, depth + 1)),
            _ => None,
        }
    }

    fn java_receiver_type(
        &self,
        owner: &DeclarationContext,
        receiver: &str,
        use_start: usize,
    ) -> Option<String> {
        if receiver == "this" || receiver == "super" {
            return owner.enclosing_type_qualified_name.clone();
        }
        if let Some(target) = self.local_java_value_type(owner, receiver) {
            return self.java_qualified_type(owner, target, use_start);
        }
        if receiver
            .rsplit('.')
            .next()
            .is_some_and(|name| name.starts_with(char::is_uppercase))
        {
            return self.java_qualified_type(owner, receiver, use_start);
        }
        None
    }

    fn local_java_value_type<'a>(
        &'a self,
        owner: &DeclarationContext,
        name: &str,
    ) -> Option<&'a String> {
        let mut scope = Some(owner.scope_id.as_str());
        for _ in 0..64 {
            let current = scope?;
            if let Some(target) = self
                .java_value_types
                .get(current)
                .and_then(|values| values.get(name))
            {
                return Some(target);
            }
            scope = self.scope_parents.get(current).map(String::as_str);
        }
        None
    }

    fn extract_rust(&mut self, root: Node<'_>) -> Result<(), EvidenceError> {
        let file = self.file.clone().ok_or_else(|| {
            EvidenceError::new(
                EvidenceErrorCode::InvalidFact,
                "non-empty Rust source has no file evidence",
            )
        })?;
        self.collect_rust_containers(root, &file)?;
        self.collect_rust_module_imports(root, &file)?;
        self.collect_rust_declarations(root, &file, None)?;
        self.collect_rust_imports(root, &file)?;
        self.collect_rust_callable_return_types(root);
        self.collect_rust_parameter_bindings(root, &file)?;
        self.walk_rust_evidence(root, &file, true)
    }

    fn collect_rust_callable_return_types(&mut self, node: Node<'_>) {
        if let Some(owner) = self.declarations.get(&node.id()).cloned()
            && matches!(owner.kind.as_str(), "function" | "method")
            && let Some(return_type) = node.child_by_field_name("return_type")
        {
            let raw = self.text(return_type);
            let qualified = if raw.trim() == "Self" {
                self.rust_concrete_callable_receiver(&owner)
            } else {
                rust_return_receiver_type_path(&raw).and_then(|nominal| {
                    rust_qualify_evidence_path(self, &owner, &nominal, return_type.start_byte())
                })
            };
            if let Some(qualified) = qualified {
                self.rust_callable_return_types
                    .entry(owner.qualified_name)
                    .or_default()
                    .push(qualified);
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(|child| child.is_named()) {
            self.collect_rust_callable_return_types(child);
        }
    }

    fn collect_rust_module_imports(
        &mut self,
        node: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        if matches!(
            node.kind(),
            "function_item" | "function_signature_item" | "impl_item"
        ) {
            return Ok(());
        }
        let active = self
            .rust_containers
            .get(&node.id())
            .filter(|context| context.kind == "module")
            .cloned()
            .unwrap_or_else(|| owner.clone());
        if node.kind() == "use_declaration" {
            if self.rust_import_nodes.insert(node.id()) {
                self.add_rust_use(node, &active)?;
            }
            return Ok(());
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(|child| child.is_named()) {
            self.collect_rust_module_imports(child, &active)?;
        }
        Ok(())
    }

    fn collect_rust_containers(
        &mut self,
        node: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        if matches!(node.kind(), "function_item" | "function_signature_item") {
            return Ok(());
        }
        if let Some(kind) = rust_container_kind(node.kind()) {
            let Some(name_node) = node.child_by_field_name("name") else {
                return Ok(());
            };
            let name = self.text(name_node);
            if name.is_empty() {
                return Ok(());
            }
            let qualified_name = rust_join_qualified(&owner.qualified_name, &name);
            let graph_node_id =
                self.unique_graph_id(make_id(&[&self.module_or_package, &qualified_name]), node);
            let mut metadata = self.declaration_metadata(node);
            if kind == "trait" {
                metadata.direct_bases_complete = !self.overlaps_parser_error(node);
            }
            let fact_id = self.builder.declare_with_metadata(
                kind,
                &graph_node_id,
                &name,
                &qualified_name,
                Some(&self.module_or_package),
                Some(&owner.scope_id),
                range_for_node(self.source_file, name_node),
                metadata,
            )?;
            let scope_id = self.builder.open_scope(
                kind,
                Some(&fact_id),
                Some(&owner.scope_id),
                range_for_node(self.source_file, node),
            )?;
            self.scope_parents
                .insert(scope_id.clone(), owner.scope_id.clone());
            let context = DeclarationContext {
                fact_id,
                scope_id,
                graph_node_id,
                name: name.clone(),
                qualified_name: qualified_name.clone(),
                kind: kind.to_owned(),
                enclosing_type_qualified_name: matches!(kind, "trait" | "struct" | "enum")
                    .then_some(qualified_name.clone()),
            };
            self.collect_rust_generic_bounds_in_scope(node, &context.scope_id);
            self.add_ownership(owner, &context)?;
            self.local_targets
                .entry(owner.scope_id.clone())
                .or_default()
                .insert(name.clone(), qualified_name.clone());
            self.rust_containers.insert(node.id(), context.clone());
            self.declarations.insert(node.id(), context.clone());
            self.add_rust_type_parameters(node, &context)?;
            if matches!(kind, "trait" | "struct" | "enum") {
                self.rust_types_by_qualified_name
                    .insert(qualified_name, context.clone());
                self.rust_types_by_name
                    .entry(name)
                    .or_default()
                    .push(context.clone());
            }
            if node.kind() == "mod_item"
                && let Some(body) = node.child_by_field_name("body")
            {
                let mut cursor = body.walk();
                for child in body.children(&mut cursor).filter(|child| child.is_named()) {
                    self.collect_rust_containers(child, &context)?;
                }
            }
            return Ok(());
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(|child| child.is_named()) {
            self.collect_rust_containers(child, owner)?;
        }
        Ok(())
    }

    fn collect_rust_declarations(
        &mut self,
        node: Node<'_>,
        owner: &DeclarationContext,
        active_impl: Option<&RustImplContext>,
    ) -> Result<(), EvidenceError> {
        if let Some(container) = self.rust_containers.get(&node.id()).cloned() {
            match node.kind() {
                "mod_item" | "trait_item" => {
                    if let Some(body) = node.child_by_field_name("body") {
                        let mut cursor = body.walk();
                        for child in body.children(&mut cursor).filter(|child| child.is_named()) {
                            self.collect_rust_declarations(child, &container, None)?;
                        }
                    }
                }
                "struct_item" => self.add_rust_struct_fields(node, &container)?,
                "enum_item" => self.add_rust_enum_members(node, &container)?,
                _ => {}
            }
            return Ok(());
        }
        match node.kind() {
            "impl_item" => {
                self.add_rust_impl(node, owner)?;
                return Ok(());
            }
            "function_item" | "function_signature_item" => {
                self.add_rust_callable(node, owner, active_impl)?;
                if let (Some(callable), Some(body)) = (
                    self.declarations.get(&node.id()).cloned(),
                    node.child_by_field_name("body"),
                ) {
                    let mut cursor = body.walk();
                    for child in body.children(&mut cursor).filter(|child| child.is_named()) {
                        self.collect_rust_declarations(child, &callable, None)?;
                    }
                }
                return Ok(());
            }
            "type_item" | "associated_type" => {
                if active_impl.is_some()
                    || owner.kind == "trait"
                    || node.kind() == "associated_type"
                {
                    self.add_rust_associated_type(node, owner, active_impl.is_some())?;
                } else {
                    self.add_rust_named_declaration(node, owner, "type_alias")?;
                }
                return Ok(());
            }
            "const_item" | "static_item" => {
                self.add_rust_named_declaration(node, owner, "constant")?;
                return Ok(());
            }
            "macro_definition" => {
                self.add_rust_named_declaration(node, owner, "macro")?;
                return Ok(());
            }
            _ => {}
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(|child| child.is_named()) {
            self.collect_rust_declarations(child, owner, active_impl)?;
        }
        Ok(())
    }

    fn add_rust_named_declaration(
        &mut self,
        node: Node<'_>,
        owner: &DeclarationContext,
        kind: &str,
    ) -> Result<Option<DeclarationContext>, EvidenceError> {
        let Some(name_node) = node.child_by_field_name("name") else {
            return Ok(None);
        };
        let name = self.text(name_node);
        if name.is_empty() {
            return Ok(None);
        }
        let qualified_name = rust_join_qualified(&owner.qualified_name, &name);
        let graph_node_id =
            self.unique_graph_id(make_id(&[&self.module_or_package, &qualified_name]), node);
        let metadata = self.declaration_metadata(node);
        let fact_id = self.builder.declare_with_metadata(
            kind,
            &graph_node_id,
            &name,
            &qualified_name,
            Some(&self.module_or_package),
            Some(&owner.scope_id),
            range_for_node(self.source_file, name_node),
            metadata,
        )?;
        let scope_id = if matches!(kind, "function" | "method" | "type_alias") {
            let scope_id = self.builder.open_scope(
                if kind == "type_alias" {
                    "type_alias"
                } else {
                    "callable"
                },
                Some(&fact_id),
                Some(&owner.scope_id),
                range_for_node(self.source_file, node),
            )?;
            self.scope_parents
                .insert(scope_id.clone(), owner.scope_id.clone());
            scope_id
        } else {
            owner.scope_id.clone()
        };
        let context = DeclarationContext {
            fact_id,
            scope_id,
            graph_node_id,
            name: name.clone(),
            qualified_name: qualified_name.clone(),
            kind: kind.to_owned(),
            enclosing_type_qualified_name: owner.enclosing_type_qualified_name.clone(),
        };
        self.add_ownership(owner, &context)?;
        self.local_targets
            .entry(owner.scope_id.clone())
            .or_default()
            .insert(name, qualified_name);
        self.declarations.insert(node.id(), context.clone());
        self.add_rust_type_parameters(node, &context)?;
        Ok(Some(context))
    }

    fn add_rust_associated_type(
        &mut self,
        node: Node<'_>,
        owner: &DeclarationContext,
        implementation_scoped: bool,
    ) -> Result<(), EvidenceError> {
        let Some(context) = self.add_rust_named_declaration(node, owner, "type_alias")? else {
            return Ok(());
        };
        if implementation_scoped
            && let Some(targets) = self.local_targets.get_mut(&owner.scope_id)
            && targets.get(&context.name) == Some(&context.qualified_name)
        {
            targets.remove(&context.name);
        }
        self.rust_associated_types_by_scope
            .entry(owner.scope_id.clone())
            .or_default()
            .entry(context.name.clone())
            .or_default()
            .push(context.clone());
        self.rust_associated_types_by_qualified_name
            .entry(context.qualified_name.clone())
            .or_default()
            .push(context);
        Ok(())
    }

    fn add_rust_callable(
        &mut self,
        node: Node<'_>,
        owner: &DeclarationContext,
        active_impl: Option<&RustImplContext>,
    ) -> Result<(), EvidenceError> {
        let Some(name_node) = node.child_by_field_name("name") else {
            return Ok(());
        };
        let name = self.text(name_node);
        if name.is_empty() {
            return Ok(());
        }
        let method = active_impl.is_some() || owner.kind == "trait";
        let qualified_owner = active_impl.map_or_else(
            || owner.qualified_name.clone(),
            |implementation| {
                implementation.trait_qualified_name.as_ref().map_or_else(
                    || implementation.type_qualified_name.clone(),
                    |trait_name| {
                        format!("<{} as {}>", implementation.type_qualified_name, trait_name)
                    },
                )
            },
        );
        let qualified_name = rust_join_qualified(&qualified_owner, &name);
        let graph_node_id =
            self.unique_graph_id(make_id(&[&self.module_or_package, &qualified_name]), node);
        let metadata = self.declaration_metadata(node);
        let parent_scope =
            active_impl.map_or(owner.scope_id.as_str(), |value| value.scope_id.as_str());
        let fact_id = self.builder.declare_with_metadata(
            if method { "method" } else { "function" },
            &graph_node_id,
            &name,
            &qualified_name,
            Some(&self.module_or_package),
            Some(parent_scope),
            range_for_node(self.source_file, name_node),
            metadata,
        )?;
        let scope_id = self.builder.open_scope(
            "callable",
            Some(&fact_id),
            Some(parent_scope),
            range_for_node(self.source_file, node),
        )?;
        self.scope_parents
            .insert(scope_id.clone(), parent_scope.to_owned());
        let context = DeclarationContext {
            fact_id,
            scope_id,
            graph_node_id,
            name: name.clone(),
            qualified_name: qualified_name.clone(),
            kind: if method { "method" } else { "function" }.to_owned(),
            enclosing_type_qualified_name: active_impl
                .map(|implementation| implementation.type_qualified_name.clone())
                .or_else(|| owner.enclosing_type_qualified_name.clone()),
        };
        self.collect_rust_value_types(node, &context);
        self.collect_rust_generic_bounds(node, &context);
        if owner.kind == "trait" {
            self.rust_trait_methods
                .entry((owner.qualified_name.clone(), name.clone()))
                .or_default()
                .push(qualified_name.clone());
        }
        if let Some(implementation) = active_impl {
            if let Some(type_owner) = implementation
                .owner_declaration_id
                .as_deref()
                .and_then(|id| self.rust_declaration_context(id))
                .cloned()
            {
                self.add_ownership(&type_owner, &context)?;
                self.builder.bind(
                    BindingKind::Member,
                    &name,
                    &qualified_name,
                    Some(&context.fact_id),
                    Some(&type_owner.scope_id),
                    range_for_node(self.source_file, name_node),
                )?;
            } else {
                self.add_ownership(owner, &context)?;
            }
        } else {
            self.add_ownership(owner, &context)?;
        }
        self.local_targets
            .entry(parent_scope.to_owned())
            .or_default()
            .insert(name.clone(), qualified_name.clone());
        if !method && rust_platform_cfg(node, self.source) == Some(RustPlatformCfg::Fallback) {
            self.rust_platform_fallbacks
                .insert((parent_scope.to_owned(), name.clone()));
        }
        if let Some(implementation) = active_impl {
            self.rust_receiver_methods
                .entry((implementation.type_qualified_name.clone(), name))
                .or_default()
                .push(qualified_name);
        } else if owner.kind == "trait" {
            self.rust_receiver_methods
                .entry((owner.qualified_name.clone(), name))
                .or_default()
                .push(qualified_name);
        }
        if rust_has_test_attribute(node, self.source) {
            self.rust_test_declarations.insert(context.fact_id.clone());
        }
        self.declarations.insert(node.id(), context.clone());
        self.add_rust_type_parameters(node, &context)?;
        Ok(())
    }

    fn add_rust_type_parameters(
        &mut self,
        declaration: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        let Some(parameters) = declaration
            .child_by_field_name("type_parameters")
            .or_else(|| {
                let mut cursor = declaration.walk();
                declaration
                    .children(&mut cursor)
                    .find(|child| child.kind() == "type_parameters")
            })
        else {
            return Ok(());
        };
        let mut cursor = parameters.walk();
        for parameter in parameters.children(&mut cursor).filter(|child| {
            matches!(
                child.kind(),
                "type_parameter" | "lifetime_parameter" | "const_parameter"
            )
        }) {
            let Some(name_node) = rust_generic_parameter_name(parameter) else {
                continue;
            };
            let name = self.text(name_node);
            if name.is_empty() {
                continue;
            }
            let qualified_name = format!("{}::<{name}>", owner.qualified_name);
            let graph_node_id = self.unique_graph_id(
                make_id(&[&self.module_or_package, &qualified_name]),
                parameter,
            );
            let fact_id = self.builder.declare_with_metadata(
                "parameter",
                &graph_node_id,
                &name,
                &qualified_name,
                Some(&self.module_or_package),
                Some(&owner.scope_id),
                range_for_node(self.source_file, name_node),
                self.declaration_metadata(parameter),
            )?;
            let context = DeclarationContext {
                fact_id,
                scope_id: owner.scope_id.clone(),
                graph_node_id,
                name: name.clone(),
                qualified_name: qualified_name.clone(),
                kind: "parameter".to_owned(),
                enclosing_type_qualified_name: owner.enclosing_type_qualified_name.clone(),
            };
            self.add_ownership(owner, &context)?;
            self.local_targets
                .entry(owner.scope_id.clone())
                .or_default()
                .insert(name, qualified_name.clone());
            self.rust_type_parameters_by_qualified_name
                .insert(qualified_name, context.clone());
            self.declarations.insert(parameter.id(), context);
        }
        Ok(())
    }

    fn collect_rust_generic_bounds(&mut self, callable: Node<'_>, owner: &DeclarationContext) {
        self.collect_rust_generic_bounds_in_scope(callable, &owner.scope_id);
    }

    fn collect_rust_generic_bounds_in_scope(&mut self, declaration: Node<'_>, scope_id: &str) {
        if let Some(parameters) = declaration.child_by_field_name("type_parameters") {
            let mut parameter_cursor = parameters.walk();
            for parameter in parameters
                .children(&mut parameter_cursor)
                .filter(|child| child.kind() == "type_parameter")
            {
                let Some(name_node) = parameter.child_by_field_name("name") else {
                    continue;
                };
                let name = self.text(name_node);
                if name.is_empty() {
                    continue;
                }
                let mut bounds = Vec::new();
                if let Some(bound_nodes) = parameter.child_by_field_name("bounds") {
                    self.collect_rust_trait_bound_paths(bound_nodes, &mut bounds);
                } else {
                    // Keep compatibility with older Rust grammars that exposed
                    // bounds as `trait_bound` descendants instead of a
                    // `trait_bounds` field.
                    let mut pending = vec![parameter];
                    while let Some(descendant) = pending.pop() {
                        if descendant.kind() == "trait_bound"
                            && let Some(bound) = rust_trait_bound_path(&self.text(descendant))
                        {
                            bounds.push(bound);
                        }
                        let mut cursor = descendant.walk();
                        pending.extend(
                            descendant
                                .children(&mut cursor)
                                .filter(|child| child.is_named()),
                        );
                    }
                }
                self.record_rust_generic_bounds(scope_id, &name, bounds);
            }
        }

        let mut declaration_cursor = declaration.walk();
        for where_clause in declaration
            .children(&mut declaration_cursor)
            .filter(|child| child.kind() == "where_clause")
        {
            let mut predicate_cursor = where_clause.walk();
            for predicate in where_clause
                .children(&mut predicate_cursor)
                .filter(|child| child.kind() == "where_predicate")
            {
                let Some(left) = predicate.child_by_field_name("left") else {
                    continue;
                };
                let name = self.text(left);
                if name.is_empty() {
                    continue;
                }
                let Some(bound_nodes) = predicate.child_by_field_name("bounds") else {
                    continue;
                };
                let mut bounds = Vec::new();
                self.collect_rust_trait_bound_paths(bound_nodes, &mut bounds);
                self.record_rust_generic_bounds(scope_id, &name, bounds);
            }
        }
    }

    fn collect_rust_trait_bound_paths(&self, node: Node<'_>, output: &mut Vec<String>) {
        if matches!(node.kind(), "trait_bound" | "higher_ranked_trait_bound")
            || rust_is_type_node(node.kind())
        {
            if let Some(bound) = rust_trait_bound_path(&self.text(node)) {
                output.push(bound);
            }
            return;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(|child| child.is_named()) {
            self.collect_rust_trait_bound_paths(child, output);
        }
    }

    fn record_rust_generic_bounds(&mut self, scope_id: &str, name: &str, bounds: Vec<String>) {
        if name.is_empty() || bounds.is_empty() {
            return;
        }
        let entry = self
            .rust_generic_bounds
            .entry(scope_id.to_owned())
            .or_default()
            .entry(name.to_owned())
            .or_default();
        entry.extend(bounds);
        entry.sort_unstable();
        entry.dedup();
    }

    fn collect_rust_parameter_bindings(
        &mut self,
        node: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        let active = self
            .declarations
            .get(&node.id())
            .cloned()
            .unwrap_or_else(|| owner.clone());
        if matches!(node.kind(), "function_item" | "function_signature_item")
            && self.declarations.contains_key(&node.id())
        {
            self.add_rust_parameter_bindings(node, &active)?;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(|child| child.is_named()) {
            self.collect_rust_parameter_bindings(child, &active)?;
        }
        Ok(())
    }

    fn add_rust_parameter_bindings(
        &mut self,
        callable: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        let Some(parameters) = callable.child_by_field_name("parameters") else {
            return Ok(());
        };
        let mut cursor = parameters.walk();
        for parameter in parameters
            .children(&mut cursor)
            .filter(|node| node.is_named())
        {
            if parameter.kind() != "parameter" {
                continue;
            }
            let Some(name_node) = parameter
                .child_by_field_name("pattern")
                .or_else(|| parameter.child_by_field_name("name"))
                .filter(|node| node.kind() == "identifier")
            else {
                continue;
            };
            let Some(type_node) = parameter.child_by_field_name("type") else {
                continue;
            };
            let raw_type_text = self.text(type_node);
            let Some(raw_type) = rust_nominal_type_path(&raw_type_text) else {
                continue;
            };
            let local_type = self.rust_type_context(owner, &raw_type).map(|context| {
                (
                    context.qualified_name.clone(),
                    Some(context.fact_id.clone()),
                )
            });
            let imported_type_target = self
                .imported_target_for_occurrence(owner, &raw_type, type_node.start_byte(), true)
                .cloned()
                .map(|target| (target, None));
            let imported_type = local_type.is_none();
            let Some((target, type_fact_id)) = local_type.or(imported_type_target) else {
                continue;
            };
            let name = self.text(name_node);
            if name.is_empty() || name == "_" {
                continue;
            }
            let binding_id = self.builder.bind(
                BindingKind::LocalAlias,
                &name,
                &target,
                type_fact_id.as_deref(),
                Some(&owner.scope_id),
                range_for_node(self.source_file, name_node),
            )?;
            self.local_bindings
                .entry(owner.scope_id.clone())
                .or_default()
                .insert(name.clone(), binding_id);
            self.local_targets
                .entry(owner.scope_id.clone())
                .or_default()
                .insert(name.clone(), target);
            self.rust_typed_receivers
                .insert((owner.scope_id.clone(), name.clone()));
            if imported_type {
                self.rust_imported_typed_receivers
                    .insert((owner.scope_id.clone(), name));
            }
        }
        Ok(())
    }

    fn rust_declaration_context(&self, fact_id: &str) -> Option<&DeclarationContext> {
        self.declarations
            .values()
            .find(|context| context.fact_id == fact_id)
    }

    fn rust_associated_type_for(
        &self,
        owner: &DeclarationContext,
        name: &str,
    ) -> Option<&DeclarationContext> {
        let mut scope_id = Some(owner.scope_id.as_str());
        for _ in 0..64 {
            let current = scope_id?;
            if let Some(candidates) = self
                .rust_associated_types_by_scope
                .get(current)
                .and_then(|types| types.get(name))
            {
                let [candidate] = candidates.as_slice() else {
                    return None;
                };
                return Some(candidate);
            }
            scope_id = self.scope_parents.get(current).map(String::as_str);
        }
        None
    }

    fn rust_impl_for_node(&self, node: Node<'_>) -> Option<&RustImplContext> {
        let mut current = Some(node);
        while let Some(candidate) = current {
            if candidate.kind() == "impl_item" {
                return self.rust_impls.get(&candidate.id());
            }
            current = candidate.parent();
        }
        None
    }

    fn add_rust_impl(
        &mut self,
        node: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        let Some(type_node) = node.child_by_field_name("type") else {
            return Ok(());
        };
        let type_name = self.text(type_node);
        let type_context = self.rust_type_context(owner, &type_name).cloned();
        let type_qualified_name = type_context.as_ref().map_or_else(
            || rust_qualify_local_path(&owner.qualified_name, &type_name),
            |context| context.qualified_name.clone(),
        );
        let trait_node = node.child_by_field_name("trait");
        let trait_name = trait_node.map(|value| self.text(value));
        let trait_qualified_name = trait_name.as_deref().map(|name| {
            self.rust_type_context(owner, name).map_or_else(
                || rust_qualify_imported_path(self, owner, name, node.start_byte()),
                |context| context.qualified_name.clone(),
            )
        });
        let scope_id = self.builder.open_scope(
            "impl",
            type_context
                .as_ref()
                .map(|context| context.fact_id.as_str()),
            Some(&owner.scope_id),
            range_for_node(self.source_file, node),
        )?;
        self.scope_parents
            .insert(scope_id.clone(), owner.scope_id.clone());
        let implementation = RustImplContext {
            scope_id,
            type_qualified_name,
            trait_qualified_name: trait_qualified_name.clone(),
            owner_declaration_id: type_context.as_ref().map(|context| context.fact_id.clone()),
        };
        self.collect_rust_generic_bounds_in_scope(node, &implementation.scope_id);
        if let Some(type_context) = type_context.as_ref()
            && let Some(bounds) = self
                .rust_generic_bounds
                .get(&type_context.scope_id)
                .cloned()
        {
            for (name, bounds) in bounds {
                self.record_rust_generic_bounds(&implementation.scope_id, &name, bounds);
            }
        }
        if let Some(trait_qualified_name) = trait_qualified_name.as_ref() {
            let receiver = type_context.as_ref().map_or_else(
                || {
                    rust_nominal_type_path(&type_name)
                        .filter(|name| rust_primitive_type(name))
                        .unwrap_or_else(|| implementation.type_qualified_name.clone())
                },
                |context| context.qualified_name.clone(),
            );
            let traits = self.rust_receiver_traits.entry(receiver).or_default();
            traits.push(trait_qualified_name.clone());
            traits.sort_unstable();
            traits.dedup();
        }
        let implementation_qualified_name = self
            .declaration_metadata(node)
            .signature
            .unwrap_or_else(|| format!("impl {}", implementation.type_qualified_name));
        let implementation_owner = DeclarationContext {
            fact_id: type_context
                .as_ref()
                .map_or_else(|| owner.fact_id.clone(), |context| context.fact_id.clone()),
            scope_id: implementation.scope_id.clone(),
            graph_node_id: type_context.as_ref().map_or_else(
                || owner.graph_node_id.clone(),
                |context| context.graph_node_id.clone(),
            ),
            name: type_name.clone(),
            qualified_name: format!("<{implementation_qualified_name}>"),
            kind: type_context
                .as_ref()
                .map_or_else(|| owner.kind.clone(), |context| context.kind.clone()),
            enclosing_type_qualified_name: Some(implementation.type_qualified_name.clone()),
        };
        self.add_rust_type_parameters(node, &implementation_owner)?;
        let generic_type_qualified_name = rust_nominal_type_path(&type_name)
            .map(|name| format!("{}::<{name}>", implementation_owner.qualified_name));
        let generic_type_context = generic_type_qualified_name
            .as_ref()
            .and_then(|qualified_name| {
                self.rust_type_parameters_by_qualified_name
                    .get(qualified_name)
            })
            .cloned();
        if type_context.is_some()
            && let Some(type_arguments) =
                trait_node.and_then(|trait_node| trait_node.child_by_field_name("type_arguments"))
            && !self.overlaps_parser_error(type_arguments)
        {
            let mut targets = Vec::new();
            collect_rust_type_nodes(
                type_arguments,
                type_arguments.end_byte(),
                None,
                &mut targets,
            );
            for target in targets {
                let raw = self.text(target);
                if raw.is_empty() || rust_primitive_type(&raw) {
                    continue;
                }
                self.add_rust_path_candidate(
                    SemanticRole::TypeReference,
                    CandidateRelation::References,
                    &implementation_owner,
                    target,
                    None,
                )?;
            }
        }
        if let (Some(implementer), Some(trait_node), Some(trait_qualified_name)) = (
            type_context.as_ref().or(generic_type_context.as_ref()),
            trait_node,
            trait_qualified_name.as_ref(),
        ) && !self.overlaps_parser_error(type_node)
            && !self.overlaps_parser_error(trait_node)
        {
            self.add_rust_occurrence_candidate(
                SemanticRole::TraitBound,
                CandidateRelation::Implements,
                implementer,
                trait_node,
                Some(trait_qualified_name),
                vec!["trait".to_owned()],
                !rust_identity_is_internal(&self.module_or_package, trait_qualified_name),
                None,
            )?;
        }
        self.rust_impls.insert(node.id(), implementation.clone());
        if let Some(body) = node.child_by_field_name("body") {
            let mut cursor = body.walk();
            for child in body.children(&mut cursor).filter(|child| child.is_named()) {
                self.collect_rust_declarations(
                    child,
                    &implementation_owner,
                    Some(&implementation),
                )?;
            }
        }
        Ok(())
    }

    fn rust_type_context(
        &self,
        owner: &DeclarationContext,
        raw: &str,
    ) -> Option<&DeclarationContext> {
        let raw = rust_nominal_type_path(raw)?;
        let qualified = rust_qualify_local_path(&owner.qualified_name, &raw);
        self.rust_types_by_qualified_name
            .get(&qualified)
            .or_else(|| {
                let leaf = rust_path_leaf(&raw);
                self.rust_types_by_name
                    .get(leaf)
                    .filter(|contexts| contexts.len() == 1)
                    .and_then(|contexts| contexts.first())
            })
    }

    fn record_rust_value_type(
        &mut self,
        scope_id: &str,
        name: &str,
        raw: Option<String>,
        active_from: usize,
    ) {
        if name.is_empty() || name == "_" || name == "self" {
            return;
        }
        self.rust_value_types
            .entry(scope_id.to_owned())
            .or_default()
            .entry(name.to_owned())
            .or_default()
            .push(RustValueTypeVersion { raw, active_from });
    }

    fn rust_value_type_for<'a>(
        &'a self,
        owner: &DeclarationContext,
        name: &str,
        use_start: usize,
        use_node: Option<Node<'_>>,
    ) -> Option<&'a str> {
        let mut node = use_node;
        while let Some(current) = node {
            if rust_is_lexical_scope_node(current.kind()) {
                let scope_id = format!("rust-lexical:{}", current.id());
                if let Some(raw) = self.rust_value_type_in_scope(&scope_id, name, use_start) {
                    return Some(raw);
                }
            }
            node = current.parent();
        }
        let mut scope_id = Some(owner.scope_id.as_str());
        for _ in 0..64 {
            let current = scope_id?;
            if let Some(raw) = self.rust_value_type_in_scope(current, name, use_start) {
                return Some(raw);
            }
            scope_id = self.scope_parents.get(current).map(String::as_str);
        }
        None
    }

    fn rust_value_type_in_scope<'a>(
        &'a self,
        scope_id: &str,
        name: &str,
        use_start: usize,
    ) -> Option<&'a str> {
        self.rust_value_types
            .get(scope_id)
            .and_then(|values| values.get(name))
            .and_then(|versions| {
                versions
                    .iter()
                    .filter(|version| version.active_from <= use_start)
                    .max_by_key(|version| version.active_from)
            })
            .and_then(|version| version.raw.as_deref())
    }

    fn ensure_rust_lexical_scope(&mut self, node: Node<'_>, parent: &str) -> String {
        let scope_id = format!("rust-lexical:{}", node.id());
        self.scope_parents
            .entry(scope_id.clone())
            .or_insert_with(|| parent.to_owned());
        scope_id
    }

    fn rust_field_receiver_type(
        &self,
        owner: &DeclarationContext,
        qualifier: &str,
        use_start: usize,
        use_node: Node<'_>,
    ) -> Option<String> {
        let mut fields = qualifier.split('.');
        let first = fields.next()?.trim();
        let mut current = if first == "self" {
            rust_callable_owner(owner)?.to_owned()
        } else {
            let raw = self.rust_value_type_for(owner, first, use_start, Some(use_node))?;
            let nominal = rust_nominal_type_path(raw)?;
            if rust_primitive_type(&nominal) {
                nominal
            } else {
                rust_qualify_evidence_path(self, owner, &nominal, use_start)?
            }
        };
        for field in fields.map(str::trim).filter(|field| !field.is_empty()) {
            let raw = self.rust_field_types.get(&current)?.get(field)?;
            let nominal = rust_nominal_type_path(raw)?;
            current = if rust_primitive_type(&nominal) {
                nominal
            } else {
                rust_qualify_evidence_path(self, owner, &nominal, use_start)?
            };
        }
        Some(current)
    }

    fn rust_field_receiver_nominal_type(
        &self,
        owner: &DeclarationContext,
        qualifier: &str,
        use_start: usize,
        use_node: Node<'_>,
    ) -> Option<String> {
        let mut fields = qualifier.split('.');
        let first = fields.next()?.trim();
        let mut current = if first == "self" {
            rust_callable_owner(owner)?.to_owned()
        } else {
            self.rust_value_type_for(owner, first, use_start, Some(use_node))?
                .to_owned()
        };
        for field in fields.map(str::trim).filter(|field| !field.is_empty()) {
            let nominal = rust_nominal_type_path(&current)?;
            let qualified = rust_qualify_evidence_path(self, owner, &nominal, use_start)?;
            current = self.rust_field_types.get(&qualified)?.get(field)?.clone();
        }
        rust_nominal_type_path(&current)
    }

    fn collect_rust_parameter_value_types(&mut self, parameters: Node<'_>, scope_id: &str) {
        let mut cursor = parameters.walk();
        for parameter in parameters
            .children(&mut cursor)
            .filter(|child| child.is_named())
        {
            let Some(pattern) = parameter.child_by_field_name("pattern") else {
                continue;
            };
            let raw = parameter
                .child_by_field_name("type")
                .map(|type_node| self.text(type_node));
            let mut names = Vec::new();
            collect_rust_pattern_names(pattern, &mut names, self.source);
            for name in names {
                self.record_rust_value_type(scope_id, &name, raw.clone(), parameter.start_byte());
            }
        }
    }

    fn collect_rust_value_types(&mut self, callable: Node<'_>, owner: &DeclarationContext) {
        let scope_id = owner.scope_id.clone();
        if let Some(parameters) = callable.child_by_field_name("parameters") {
            self.collect_rust_parameter_value_types(parameters, &scope_id);
        }
        let Some(body) = callable.child_by_field_name("body") else {
            return;
        };
        self.collect_rust_value_types_in_body(body, owner, &scope_id, true);
    }

    fn collect_rust_value_types_in_body(
        &mut self,
        node: Node<'_>,
        owner: &DeclarationContext,
        parent_scope: &str,
        root: bool,
    ) {
        if matches!(node.kind(), "function_item" | "function_signature_item") {
            return;
        }
        let scope_id = if !root && rust_is_lexical_scope_node(node.kind()) {
            self.ensure_rust_lexical_scope(node, parent_scope)
        } else {
            parent_scope.to_owned()
        };
        if node.kind() == "closure_expression"
            && let Some(parameters) = node.child_by_field_name("parameters")
        {
            self.collect_rust_parameter_value_types(parameters, &scope_id);
        }
        if node.kind() == "let_declaration"
            && let Some(pattern) = node.child_by_field_name("pattern")
        {
            let raw = node
                .child_by_field_name("type")
                .map(|type_node| self.text(type_node))
                .or_else(|| {
                    node.child_by_field_name("value")
                        .and_then(|value| self.rust_inferred_value_type(value, owner))
                });
            let mut names = Vec::new();
            collect_rust_pattern_names(pattern, &mut names, self.source);
            for name in names {
                self.record_rust_value_type(&scope_id, &name, raw.clone(), pattern.start_byte());
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(|child| child.is_named()) {
            self.collect_rust_value_types_in_body(child, owner, &scope_id, false);
        }
    }

    fn rust_inferred_value_type(
        &self,
        value: Node<'_>,
        owner: &DeclarationContext,
    ) -> Option<String> {
        match value.kind() {
            "call_expression" => {
                let function = value.child_by_field_name("function")?;
                let function = if function.kind() == "generic_function" {
                    function.child_by_field_name("function").unwrap_or(function)
                } else {
                    function
                };
                let raw = self.text(function);
                split_qualified(&raw).0.map(str::to_owned)
            }
            "struct_expression" => value
                .child_by_field_name("name")
                .map(|name| self.text(name)),
            "identifier" => self
                .rust_value_type_for(owner, &self.text(value), value.start_byte(), Some(value))
                .map(str::to_owned),
            _ => None,
        }
    }

    fn rust_enclosing_module(&self, owner: &DeclarationContext) -> String {
        if matches!(owner.kind.as_str(), "file" | "module") {
            return owner.qualified_name.clone();
        }
        let mut scope_id = Some(owner.scope_id.as_str());
        for _ in 0..64 {
            let Some(current) = scope_id else {
                break;
            };
            if let Some(context) = self
                .rust_containers
                .values()
                .find(|context| context.kind == "module" && context.scope_id == current)
            {
                return context.qualified_name.clone();
            }
            scope_id = self.scope_parents.get(current).map(String::as_str);
        }
        self.module_or_package.clone()
    }

    fn rust_canonical_import_target(&self, module: &str, raw: &str) -> String {
        let canonical = rust_canonical_import_target(module, raw);
        let first = canonical.split("::").next().unwrap_or_default();
        let Some(mapped) = self.rust_namespace_aliases.get(first) else {
            return canonical;
        };
        canonical.replacen(first, mapped, 1)
    }

    /// Resolve a path rooted at a local module before treating it as an
    /// external crate path. Rust 2018 permits `pub use api::Thing` at the
    /// crate root; the parser gives us no `crate::` prefix, but the container
    /// inventory already proves that `api` is local.
    fn rust_import_target(&self, owner: &DeclarationContext, raw: &str) -> String {
        let (qualifier, spelling) = split_qualified(raw);
        if let Some(qualifier) = qualifier
            && let Some(target) = self.local_target_for(owner, qualified_binding_head(qualifier))
        {
            return rust_join_qualified(target, spelling);
        }
        self.rust_canonical_import_target(&self.rust_enclosing_module(owner), raw)
    }

    fn add_rust_struct_fields(
        &mut self,
        node: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        let Some(body) = node.child_by_field_name("body") else {
            return Ok(());
        };
        let mut cursor = body.walk();
        let mut tuple_index = 0usize;
        for field in body.children(&mut cursor).filter(|child| child.is_named()) {
            if field.kind() == "field_declaration" {
                self.record_rust_field_type(field, owner, None);
                self.add_rust_field(field, owner, None)?;
            } else if rust_is_type_node(field.kind()) {
                self.record_rust_field_type(field, owner, Some(tuple_index));
                self.add_rust_field(field, owner, Some(tuple_index))?;
                tuple_index = tuple_index.saturating_add(1);
            }
        }
        Ok(())
    }

    fn record_rust_field_type(
        &mut self,
        field: Node<'_>,
        owner: &DeclarationContext,
        tuple_index: Option<usize>,
    ) {
        let name = field
            .child_by_field_name("name")
            .map(|node| self.text(node))
            .unwrap_or_else(|| tuple_index.unwrap_or_default().to_string());
        let type_node = field
            .child_by_field_name("type")
            .or_else(|| (field.kind() != "field_declaration").then_some(field));
        let Some(type_node) = type_node else {
            return;
        };
        let raw = self.text(type_node);
        if name.is_empty() || raw.is_empty() {
            return;
        }
        self.rust_field_types
            .entry(owner.qualified_name.clone())
            .or_default()
            .insert(name, raw);
    }

    fn add_rust_enum_members(
        &mut self,
        node: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        let Some(body) = node.child_by_field_name("body") else {
            return Ok(());
        };
        let mut cursor = body.walk();
        for variant in body
            .children(&mut cursor)
            .filter(|child| child.kind() == "enum_variant")
        {
            let Some(name_node) = variant.child_by_field_name("name") else {
                continue;
            };
            let name = self.text(name_node);
            if name.is_empty() {
                continue;
            }
            let qualified_name = rust_join_qualified(&owner.qualified_name, &name);
            let graph_node_id = self.unique_graph_id(
                make_id(&[&self.module_or_package, &qualified_name]),
                variant,
            );
            let fact_id = self.builder.declare_with_metadata(
                "enum_member",
                &graph_node_id,
                &name,
                &qualified_name,
                Some(&self.module_or_package),
                Some(&owner.scope_id),
                range_for_node(self.source_file, name_node),
                self.declaration_metadata(variant),
            )?;
            let context = DeclarationContext {
                fact_id,
                scope_id: owner.scope_id.clone(),
                graph_node_id,
                name,
                qualified_name,
                kind: "enum_member".to_owned(),
                enclosing_type_qualified_name: Some(owner.qualified_name.clone()),
            };
            self.add_ownership(owner, &context)?;
            self.add_rust_declaration_references(variant, owner)?;
            self.declarations.insert(variant.id(), context);
        }
        Ok(())
    }

    fn add_rust_field(
        &mut self,
        node: Node<'_>,
        owner: &DeclarationContext,
        tuple_index: Option<usize>,
    ) -> Result<(), EvidenceError> {
        let name_node = node.child_by_field_name("name");
        let name = name_node.map_or_else(
            || tuple_index.unwrap_or_default().to_string(),
            |value| self.text(value),
        );
        if name.is_empty() {
            return Ok(());
        }
        let qualified_name = rust_join_qualified(&owner.qualified_name, &name);
        let graph_node_id =
            self.unique_graph_id(make_id(&[&self.module_or_package, &qualified_name]), node);
        let fact_id = self.builder.declare_with_metadata(
            "field",
            &graph_node_id,
            &name,
            &qualified_name,
            Some(&self.module_or_package),
            Some(&owner.scope_id),
            name_node.map_or_else(
                || range_for_node(self.source_file, node),
                |value| range_for_node(self.source_file, value),
            ),
            self.declaration_metadata(node),
        )?;
        let context = DeclarationContext {
            fact_id,
            scope_id: owner.scope_id.clone(),
            graph_node_id,
            name,
            qualified_name,
            kind: "field".to_owned(),
            enclosing_type_qualified_name: Some(owner.qualified_name.clone()),
        };
        self.add_ownership(owner, &context)?;
        self.declarations.insert(node.id(), context);
        Ok(())
    }

    fn collect_rust_imports(
        &mut self,
        node: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        let active = self
            .declarations
            .get(&node.id())
            .filter(|context| matches!(context.kind.as_str(), "module" | "function" | "method"))
            .cloned()
            .unwrap_or_else(|| owner.clone());
        if node.kind() == "use_declaration" {
            if self.rust_import_nodes.insert(node.id()) {
                self.add_rust_use(node, &active)?;
            }
            return Ok(());
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(|child| child.is_named()) {
            self.collect_rust_imports(child, &active)?;
        }
        Ok(())
    }

    fn add_rust_use(
        &mut self,
        node: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        if self.overlaps_parser_error(node) {
            return Ok(());
        }
        let Some(argument) = node.child_by_field_name("argument") else {
            return Ok(());
        };
        let reexport = rust_is_public_use(&self.text(node));
        let raw = self.text(argument);
        let mut flattened = Vec::new();
        expand_rust_use_tree(&raw, "", &mut flattened);
        for (raw_target, alias, glob) in flattened {
            let target = self.rust_import_target(owner, &raw_target);
            if target.is_empty() {
                continue;
            }
            let local = if glob {
                "*".to_owned()
            } else {
                alias.unwrap_or_else(|| rust_path_leaf(&target).to_owned())
            };
            if local.is_empty() {
                continue;
            }
            let range = rust_use_binding_range(
                self.source_file,
                self.source,
                argument,
                &local,
                &raw_target,
            );
            let kind = if reexport {
                BindingKind::Reexport
            } else if !glob && local != rust_path_leaf(&target) {
                BindingKind::ImportAlias
            } else {
                BindingKind::Import
            };
            let binding_id = self.builder.bind(
                kind,
                &local,
                &target,
                None,
                Some(&owner.scope_id),
                range.clone(),
            )?;
            if reexport
                && let Some(platform @ (RustPlatformCfg::Unix | RustPlatformCfg::Windows)) =
                    rust_platform_cfg(node, self.source)
            {
                self.rust_platform_reexport_bindings
                    .insert(binding_id.clone(), platform);
            }
            self.record_import_binding(owner, &local, &target, &binding_id, 0);
            let occurrence_id = self.builder.occur(
                SemanticRole::Import,
                &owner.fact_id,
                &local,
                None,
                Some(&owner.scope_id),
                range,
            )?;
            self.builder.relate(
                if reexport {
                    CandidateRelation::Reexports
                } else {
                    CandidateRelation::Imports
                },
                &owner.fact_id,
                Some(&occurrence_id),
                Some(&binding_id),
                rust_path_leaf(&target),
                ResolutionConstraint {
                    exact_target_declaration_id: None,
                    exact_language: Some(self.language.to_owned()),
                    module_or_package: rust_qualified_parent(&target).map(str::to_owned),
                    scope_id: Some(owner.scope_id.clone()),
                    qualified_name: Some(target.clone()),
                    argument_count: None,
                    argument_types: Vec::new(),
                    allowed_target_kinds: vec![
                        "file".to_owned(),
                        "module".to_owned(),
                        "trait".to_owned(),
                        "struct".to_owned(),
                        "enum".to_owned(),
                        "type_alias".to_owned(),
                        "function".to_owned(),
                        "macro".to_owned(),
                    ],
                    hierarchy: None,
                    allow_external: !rust_identity_is_internal(&self.module_or_package, &target),
                },
            )?;
        }
        Ok(())
    }

    fn walk_rust_evidence(
        &mut self,
        node: Node<'_>,
        owner: &DeclarationContext,
        root: bool,
    ) -> Result<(), EvidenceError> {
        let active = self
            .declarations
            .get(&node.id())
            .cloned()
            .unwrap_or_else(|| owner.clone());
        if self.declarations.contains_key(&node.id()) {
            self.add_rust_declaration_references(node, &active)?;
        }
        match node.kind() {
            "use_declaration" => return Ok(()),
            "call_expression" => self.add_rust_call(node, &active)?,
            "macro_invocation" => self.add_rust_macro_invocation(node, &active)?,
            _ => {}
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(|child| child.is_named()) {
            if !root
                && matches!(child.kind(), "function_item" | "function_signature_item")
                && !self.declarations.contains_key(&child.id())
            {
                continue;
            }
            self.walk_rust_evidence(child, &active, false)?;
        }
        Ok(())
    }

    fn add_rust_declaration_references(
        &mut self,
        node: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        let body_start = if matches!(
            node.kind(),
            "function_item"
                | "function_signature_item"
                | "trait_item"
                | "struct_item"
                | "enum_item"
                | "mod_item"
        ) {
            node.child_by_field_name("body")
                .map_or(node.end_byte(), |body| body.start_byte())
        } else {
            node.end_byte()
        };
        let name_id = node.child_by_field_name("name").map(|name| name.id());
        if let Some(return_type) = node.child_by_field_name("return_type")
            && self.text(return_type).trim() == "Self"
            && let Some(receiver) = self.rust_concrete_callable_receiver(owner)
        {
            self.add_rust_occurrence_candidate(
                SemanticRole::TypeReference,
                CandidateRelation::Returns,
                owner,
                return_type,
                Some(&receiver),
                vec!["struct".to_owned(), "enum".to_owned()],
                false,
                Some("rust-outer-nominal-return"),
            )?;
        }
        let outer_return_id = node
            .child_by_field_name("return_type")
            .and_then(rust_outer_nominal_return_node)
            .map(|target| target.id());
        let mut targets = Vec::new();
        collect_rust_type_nodes(node, body_start, name_id, &mut targets);
        for target in targets {
            let raw = self.text(target);
            if raw.is_empty() || rust_primitive_type(&raw) {
                continue;
            }
            let relation = if node.kind() == "trait_item"
                && rust_node_has_ancestor_before(target, node, "trait_bounds")
                && !rust_node_has_ancestor_before(target, node, "type_arguments")
            {
                CandidateRelation::Extends
            } else if node
                .child_by_field_name("return_type")
                .is_some_and(|return_type| {
                    return_type.start_byte() <= target.start_byte()
                        && target.end_byte() <= return_type.end_byte()
                })
            {
                CandidateRelation::Returns
            } else if matches!(
                node.kind(),
                "field_declaration" | "const_item" | "static_item" | "parameter"
            ) {
                CandidateRelation::TypeOf
            } else {
                CandidateRelation::References
            };
            let role = if relation == CandidateRelation::Extends {
                SemanticRole::TraitBound
            } else {
                SemanticRole::TypeReference
            };
            let allowed_target_kinds = (owner.kind == "parameter"
                && rust_node_has_ancestor_before(target, node, "trait_bounds"))
            .then(|| {
                vec![
                    "trait".to_owned(),
                    "interface".to_owned(),
                    "parameter".to_owned(),
                ]
            });
            self.add_rust_path_candidate_with_context(
                role,
                relation,
                owner,
                target,
                allowed_target_kinds,
                (relation == CandidateRelation::Returns && outer_return_id == Some(target.id()))
                    .then_some("rust-outer-nominal-return"),
            )?;
        }
        let mut generic_uses = Vec::new();
        collect_rust_generic_use_nodes(node, body_start, name_id, &mut generic_uses);
        for target in generic_uses {
            let raw = self.text(target);
            let qualified = rust_qualify_evidence_path(self, owner, &raw, target.start_byte());
            if !qualified.is_some_and(|qualified| {
                self.rust_type_parameters_by_qualified_name
                    .contains_key(&qualified)
            }) {
                continue;
            }
            self.add_rust_path_candidate(
                SemanticRole::TypeReference,
                CandidateRelation::References,
                owner,
                target,
                Some(vec!["parameter".to_owned()]),
            )?;
        }
        Ok(())
    }

    fn add_rust_call(
        &mut self,
        call: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        let Some(function) = call.child_by_field_name("function") else {
            return Ok(());
        };
        let function = if function.kind() == "generic_function" {
            function.child_by_field_name("function").unwrap_or(function)
        } else {
            function
        };
        if !matches!(
            function.kind(),
            "identifier" | "scoped_identifier" | "field_expression"
        ) {
            return Ok(());
        }
        let raw = self.text(function);
        let (qualifier, spelling) = if function.kind() == "field_expression" {
            let qualifier = function
                .child_by_field_name("value")
                .map(|value| self.text(value));
            let spelling = function
                .child_by_field_name("field")
                .map_or_else(String::new, |field| self.text(field));
            (qualifier, spelling)
        } else {
            let (qualifier, spelling) = split_qualified(&raw);
            (qualifier.map(str::to_owned), spelling.to_owned())
        };
        let qualifier = qualifier.as_deref();
        let spelling = spelling.as_str();
        if spelling.is_empty() {
            return Ok(());
        }
        let binding_name = qualifier.map(qualified_binding_head).unwrap_or(spelling);
        let uses_type_namespace = function.kind() == "scoped_identifier";
        let import_binding_is_ambiguous = if uses_type_namespace {
            self.visible_import_binding_is_ambiguous(owner, binding_name)
        } else {
            self.import_binding_is_ambiguous(owner, binding_name)
        };
        let platform_reexport_bindings = import_binding_is_ambiguous
            .then(|| self.rust_platform_reexport_bindings(owner, binding_name))
            .flatten();
        if import_binding_is_ambiguous && platform_reexport_bindings.is_none() {
            return Ok(());
        }
        let direct_binding = platform_reexport_bindings
            .is_none()
            .then(|| {
                if uses_type_namespace {
                    self.import_binding_version_at(owner, binding_name, function.start_byte(), true)
                        .map(|binding| binding.binding_id.clone())
                        .or_else(|| self.local_binding_for(owner, binding_name).cloned())
                } else {
                    self.binding_for_occurrence(owner, binding_name, function.start_byte(), true)
                        .cloned()
                }
            })
            .flatten();
        let wildcard_lookup_eligible = qualifier.is_none()
            || qualifier.is_some_and(|value| {
                qualified_binding_head(value)
                    .chars()
                    .next()
                    .is_some_and(char::is_uppercase)
            });
        let wildcard_binding = (direct_binding.is_none() && wildcard_lookup_eligible)
            .then(|| self.rust_wildcard_binding(owner, function.start_byte()))
            .flatten()
            .cloned();
        let fallback_binding = direct_binding.clone().or_else(|| wildcard_binding.clone());
        let call_result_binding = if platform_reexport_bindings.is_none() {
            self.rust_call_result_binding_for_occurrence(
                owner,
                function,
                fallback_binding.as_deref(),
            )?
        } else {
            None
        };
        let qualified_name = platform_reexport_bindings
            .as_ref()
            .map_or_else(
                || {
                    self.rust_call_qualified_name(
                        owner,
                        qualifier,
                        spelling,
                        function.start_byte(),
                        function,
                    )
                },
                |_| None,
            )
            .or_else(|| {
                wildcard_binding.is_some().then(|| {
                    qualifier.map_or_else(
                        || spelling.to_owned(),
                        |qualifier| rust_join_qualified(qualifier, spelling),
                    )
                })
            });
        let wildcard_bound = wildcard_binding.is_some();
        let wildcard_external_target_is_explicit = qualifier.is_some_and(|value| {
            qualified_binding_head(value)
                .chars()
                .next()
                .is_some_and(char::is_uppercase)
        }) || (qualifier.is_none()
            && spelling.chars().next().is_some_and(char::is_uppercase));
        let has_ambiguous_local_self_methods = qualifier.is_some_and(rust_receiver_is_self)
            && rust_callable_owner(owner).is_some_and(|receiver| {
                self.rust_receiver_methods
                    .get(&(receiver.to_owned(), spelling.to_owned()))
                    .is_some_and(|methods| methods.len() > 1)
            });
        let qualified_name = if has_ambiguous_local_self_methods {
            None
        } else {
            qualified_name
        };
        let binding = call_result_binding.or(fallback_binding);
        let occurrence_id = self.builder.occur_with_context(
            SemanticRole::Call,
            &owner.fact_id,
            spelling,
            qualifier,
            Some(&owner.scope_id),
            platform_reexport_bindings
                .as_ref()
                .map(|_| "rust-platform-cfg-reexport"),
            range_for_node(self.source_file, function),
        )?;
        let constraints = ResolutionConstraint {
            exact_target_declaration_id: None,
            exact_language: Some(self.language.to_owned()),
            module_or_package: qualified_name
                .as_deref()
                .and_then(|qualified| rust_qualified_parent(qualified).map(str::to_owned))
                .or_else(|| Some(self.module_or_package.clone())),
            scope_id: Some(owner.scope_id.clone()),
            qualified_name: qualified_name.clone(),
            argument_count: None,
            argument_types: Vec::new(),
            allowed_target_kinds: vec![
                "enum_member".to_owned(),
                "function".to_owned(),
                "method".to_owned(),
                "struct".to_owned(),
            ],
            hierarchy: None,
            allow_external: qualified_name.as_deref().is_some_and(|qualified| {
                ((wildcard_bound && wildcard_external_target_is_explicit)
                    || (!wildcard_bound && !qualifier.is_some_and(rust_deferred_owner)))
                    && !has_ambiguous_local_self_methods
                    && !rust_identity_is_internal(&self.module_or_package, qualified)
            }),
        };
        let candidate_bindings = platform_reexport_bindings.as_ref().map_or_else(
            || vec![binding],
            |(bindings, has_fallback)| {
                let mut candidates = bindings.iter().cloned().map(Some).collect::<Vec<_>>();
                if *has_fallback {
                    // The source-proven fallback is a lexical declaration,
                    // not an import binding.
                    candidates.push(None);
                }
                candidates
            },
        );
        for candidate_binding in candidate_bindings {
            self.builder.relate(
                CandidateRelation::Calls,
                &owner.fact_id,
                Some(&occurrence_id),
                candidate_binding.as_deref(),
                spelling,
                constraints.clone(),
            )?;
            if self.rust_test_declarations.contains(&owner.fact_id) {
                self.builder.relate(
                    CandidateRelation::Tests,
                    &owner.fact_id,
                    Some(&occurrence_id),
                    candidate_binding.as_deref(),
                    spelling,
                    constraints.clone(),
                )?;
            }
        }
        Ok(())
    }

    fn rust_call_qualified_name(
        &self,
        owner: &DeclarationContext,
        qualifier: Option<&str>,
        spelling: &str,
        use_start: usize,
        use_node: Node<'_>,
    ) -> Option<String> {
        let Some(raw_qualifier) = qualifier else {
            return self
                .imported_target_for_occurrence(owner, spelling, 0, true)
                .cloned();
        };
        let normalized_qualifier = rust_normalize_path(raw_qualifier);
        if let Some(inner) = normalized_qualifier
            .strip_prefix('<')
            .and_then(|value| value.strip_suffix('>'))
            && let Some((type_path, trait_path)) = inner.split_once(" as ")
            && let Some(type_name) = rust_qualify_evidence_path(self, owner, type_path, use_start)
            && let Some(trait_name) = rust_qualify_evidence_path(self, owner, trait_path, use_start)
        {
            return Some(format!("<{type_name} as {trait_name}>::{spelling}"));
        }
        let qualifier = normalized_qualifier.as_str();
        if (qualifier == "Self" || rust_receiver_is_self(qualifier))
            && let Some(enclosing) = rust_callable_owner(owner)
        {
            return self
                .rust_receiver_method_target(enclosing, spelling)
                .or_else(|| Some(rust_join_qualified(enclosing, spelling)));
        }
        if use_node.kind() == "scoped_identifier"
            && let Some(target) =
                self.imported_qualified_target_for(owner, qualifier, use_start, true)
        {
            return Some(rust_join_qualified(&target, spelling));
        }
        let generic_receiver_type = self
            .rust_value_type_for(
                owner,
                qualified_binding_head(qualifier),
                use_start,
                Some(use_node),
            )
            .and_then(rust_nominal_type_path);
        if let Some(receiver_type) = generic_receiver_type.as_deref()
            && let Some(method) =
                self.rust_generic_receiver_method_target(owner, receiver_type, spelling, use_start)
        {
            return Some(method);
        }
        if let Some(receiver_type) =
            self.rust_field_receiver_nominal_type(owner, qualifier, use_start, use_node)
            && let Some(method) =
                self.rust_generic_receiver_method_target(owner, &receiver_type, spelling, use_start)
        {
            return Some(method);
        }
        if let Some(receiver_type) =
            self.rust_field_receiver_type(owner, qualifier, use_start, use_node)
        {
            return self
                .rust_receiver_method_target(&receiver_type, spelling)
                .or_else(|| Some(rust_join_qualified(&receiver_type, spelling)));
        }
        if let Some(nominal_type) = generic_receiver_type
            && let Some(receiver_type) =
                rust_qualify_evidence_path(self, owner, &nominal_type, use_start)
        {
            return Some(rust_join_qualified(&receiver_type, spelling));
        }
        if let Some(target) = self.local_target_for(owner, qualifier) {
            if let Some(method) = self.rust_receiver_method_target(target, spelling) {
                return Some(method);
            }
            if self.rust_typed_receiver_for(owner, qualifier) {
                if self.rust_imported_typed_receiver_for(owner, qualifier) {
                    return Some(rust_join_qualified(target, spelling));
                }
                return Some(rust_join_qualified(qualifier, spelling));
            }
            return Some(rust_join_qualified(target, spelling));
        }
        if let Some(target) = self.imported_qualified_target_for(owner, qualifier, 0, true) {
            return Some(rust_join_qualified(&target, spelling));
        }
        let first = qualified_binding_head(qualifier);
        if matches!(first, "crate" | "self" | "super") {
            return Some(rust_join_qualified(
                &self.rust_canonical_import_target(&self.rust_enclosing_module(owner), qualifier),
                spelling,
            ));
        }
        Some(rust_join_qualified(qualifier, spelling))
    }

    fn rust_concrete_callable_receiver(&self, owner: &DeclarationContext) -> Option<String> {
        let receiver = owner.enclosing_type_qualified_name.as_ref()?;
        self.rust_receiver_methods
            .get(&(receiver.clone(), owner.name.clone()))
            .is_some_and(|methods| methods.iter().any(|method| method == &owner.qualified_name))
            .then(|| receiver.clone())
    }

    fn rust_call_result_binding_for_occurrence(
        &mut self,
        owner: &DeclarationContext,
        function: Node<'_>,
        fallback_binding_id: Option<&str>,
    ) -> Result<Option<String>, EvidenceError> {
        if function.kind() != "field_expression" {
            return Ok(None);
        }
        let Some(result_call) = function
            .child_by_field_name("value")
            .filter(|value| value.kind() == "call_expression")
        else {
            return Ok(None);
        };
        let Some(result_member) = function
            .child_by_field_name("field")
            .map(|field| self.text(field))
        else {
            return Ok(None);
        };
        self.rust_call_result_binding_for_call(
            owner,
            result_call,
            &result_member,
            fallback_binding_id,
            0,
        )
    }

    fn rust_call_result_binding_for_call(
        &mut self,
        owner: &DeclarationContext,
        result_call: Node<'_>,
        result_member: &str,
        fallback_binding_id: Option<&str>,
        depth: usize,
    ) -> Result<Option<String>, EvidenceError> {
        if depth >= 64 {
            return Err(EvidenceError::new(
                EvidenceErrorCode::ResourceLimit,
                "Rust call-result chain exceeds depth limit",
            ));
        }
        let Some(called) = result_call.child_by_field_name("function") else {
            return Ok(None);
        };
        let called = if called.kind() == "generic_function" {
            called.child_by_field_name("function").unwrap_or(called)
        } else {
            called
        };
        if !matches!(
            called.kind(),
            "identifier" | "scoped_identifier" | "field_expression"
        ) {
            return Ok(None);
        }
        let nested_receiver_call = (called.kind() == "field_expression")
            .then(|| called.child_by_field_name("value"))
            .flatten()
            .filter(|value| value.kind() == "call_expression");
        let (qualified_callable, receiver_binding_id, result_type_qualified_name) =
            if let Some(receiver_call) = nested_receiver_call {
                let Some(called_member) = called
                    .child_by_field_name("field")
                    .map(|field| self.text(field))
                else {
                    return Ok(None);
                };
                let Some(receiver_binding_id) = self.rust_call_result_binding_for_call(
                    owner,
                    receiver_call,
                    &called_member,
                    fallback_binding_id,
                    depth.saturating_add(1),
                )?
                else {
                    return Ok(None);
                };
                let receiver_result_type = self
                    .builder
                    .batch
                    .bindings
                    .iter()
                    .find(|binding| binding.id == receiver_binding_id)
                    .and_then(|binding| binding.result_type_qualified_name.as_ref());
                let qualified_callable = receiver_result_type
                    .and_then(|receiver_type| {
                        self.rust_receiver_method_target(receiver_type, &called_member)
                    })
                    .unwrap_or(called_member);
                let result_type_qualified_name = self
                    .rust_callable_return_types
                    .get(&qualified_callable)
                    .filter(|return_types| return_types.len() == 1)
                    .and_then(|return_types| {
                        self.rust_receiver_method_target(&return_types[0], result_member)
                            .map(|_| return_types[0].clone())
                    });
                (
                    qualified_callable,
                    Some(receiver_binding_id),
                    result_type_qualified_name,
                )
            } else {
                let raw_called = self.text(called);
                let (qualifier, called_spelling) = split_qualified(&raw_called);
                let Some(qualified_callable) = self.rust_call_qualified_name(
                    owner,
                    qualifier,
                    called_spelling,
                    called.start_byte(),
                    called,
                ) else {
                    return Ok(None);
                };
                let result_type_qualified_name = if !raw_called.contains("::") {
                    self.rust_callable_return_types
                        .get(&qualified_callable)
                        .filter(|return_types| return_types.len() == 1)
                        .and_then(|return_types| {
                            self.rust_receiver_method_target(&return_types[0], result_member)
                                .map(|_| return_types[0].clone())
                        })
                } else {
                    None
                };
                (qualified_callable, None, result_type_qualified_name)
            };
        let receiver = self.text(result_call);
        let key = (
            owner.scope_id.clone(),
            qualified_callable.clone(),
            result_call.start_byte(),
        );
        if let Some(binding) = self.rust_call_result_bindings.get(&key) {
            return Ok(Some(binding.clone()));
        }
        let range = range_for_node(self.source_file, result_call);
        let binding = self.builder.bind_chained_call_result(
            &receiver,
            &qualified_callable,
            result_type_qualified_name.as_deref(),
            receiver_binding_id.as_deref(),
            fallback_binding_id,
            Some(&owner.scope_id),
            range,
        )?;
        self.rust_call_result_bindings.insert(key, binding.clone());
        Ok(Some(binding))
    }

    fn rust_generic_receiver_method_target(
        &self,
        owner: &DeclarationContext,
        receiver_type: &str,
        method: &str,
        use_start: usize,
    ) -> Option<String> {
        let mut bounds = Vec::new();
        let mut scope_id = Some(owner.scope_id.as_str());
        for _ in 0..64 {
            let Some(current) = scope_id else {
                break;
            };
            if let Some(values) = self
                .rust_generic_bounds
                .get(current)
                .and_then(|values| values.get(receiver_type))
            {
                bounds.extend(
                    values.iter().filter_map(|bound| {
                        rust_qualify_evidence_path(self, owner, bound, use_start)
                    }),
                );
                break;
            }
            scope_id = self.scope_parents.get(current).map(String::as_str);
        }
        bounds.sort_unstable();
        bounds.dedup();
        if bounds.is_empty() {
            return None;
        }

        let mut targets = bounds
            .iter()
            .filter_map(|bound| {
                self.rust_trait_methods
                    .get(&(bound.clone(), method.to_owned()))
                    .and_then(|methods| {
                        let [target] = methods.as_slice() else {
                            return None;
                        };
                        Some(target.clone())
                    })
            })
            .collect::<Vec<_>>();
        targets.sort_unstable();
        targets.dedup();
        if targets.len() == 1 {
            return targets.pop();
        }
        if bounds.len() == 1 {
            return Some(rust_join_qualified(&bounds[0], method));
        }
        None
    }

    fn rust_receiver_method_target(&self, receiver: &str, method: &str) -> Option<String> {
        let key = (receiver.to_owned(), method.to_owned());
        if let Some(targets) = self.rust_receiver_methods.get(&key) {
            let [target] = targets.as_slice() else {
                return None;
            };
            return Some(target.clone());
        }
        let mut targets = self
            .rust_receiver_traits
            .get(receiver)?
            .iter()
            .filter_map(|trait_name| {
                let targets = self
                    .rust_receiver_methods
                    .get(&(trait_name.clone(), method.to_owned()))?;
                let [target] = targets.as_slice() else {
                    return None;
                };
                Some(target.clone())
            })
            .collect::<Vec<_>>();
        targets.sort_unstable();
        targets.dedup();
        let [target] = targets.as_slice() else {
            return None;
        };
        Some(target.clone())
    }

    fn rust_typed_receiver_for(&self, owner: &DeclarationContext, name: &str) -> bool {
        let mut scope_id = Some(owner.scope_id.as_str());
        for _ in 0..64 {
            let Some(current) = scope_id else {
                break;
            };
            if self
                .rust_typed_receivers
                .contains(&(current.to_owned(), name.to_owned()))
            {
                return true;
            }
            scope_id = self.scope_parents.get(current).map(String::as_str);
        }
        false
    }

    fn rust_imported_typed_receiver_for(&self, owner: &DeclarationContext, name: &str) -> bool {
        let mut scope_id = Some(owner.scope_id.as_str());
        for _ in 0..64 {
            let Some(current) = scope_id else {
                break;
            };
            if self
                .rust_imported_typed_receivers
                .contains(&(current.to_owned(), name.to_owned()))
            {
                return true;
            }
            scope_id = self.scope_parents.get(current).map(String::as_str);
        }
        false
    }

    fn add_rust_macro_invocation(
        &mut self,
        node: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        let raw = self.text(node);
        let Some(raw_path) = raw
            .split('!')
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return Ok(());
        };
        let path_end = node.start_byte().saturating_add(raw_path.len());
        let range = range_for_byte_span(self.source_file, self.source, node.start_byte(), path_end);
        let (qualifier, spelling) = split_qualified(raw_path);
        let binding_name = qualifier.map(qualified_binding_head).unwrap_or(spelling);
        let binding = self
            .binding_for_occurrence(owner, binding_name, node.start_byte(), true)
            .or_else(|| self.rust_wildcard_binding(owner, node.start_byte()))
            .cloned();
        let qualified_name = qualifier
            .and_then(|qualifier| {
                self.imported_qualified_target_for(owner, qualifier, node.start_byte(), true)
            })
            .map(|target| rust_join_qualified(&target, spelling))
            .or_else(|| {
                self.imported_target_for_occurrence(owner, spelling, node.start_byte(), true)
                    .cloned()
            })
            .or_else(|| {
                qualifier
                    .filter(|value| value.contains("::"))
                    .map(|value| rust_join_qualified(value, spelling))
            });
        let occurrence_id = self.builder.occur(
            SemanticRole::MacroInvocation,
            &owner.fact_id,
            spelling,
            qualifier,
            Some(&owner.scope_id),
            range,
        )?;
        self.builder.relate(
            CandidateRelation::InvokesMacro,
            &owner.fact_id,
            Some(&occurrence_id),
            binding.as_deref(),
            spelling,
            ResolutionConstraint {
                exact_target_declaration_id: None,
                exact_language: Some(self.language.to_owned()),
                module_or_package: qualified_name
                    .as_deref()
                    .and_then(|qualified| rust_qualified_parent(qualified).map(str::to_owned))
                    .or_else(|| Some(self.module_or_package.clone())),
                scope_id: Some(owner.scope_id.clone()),
                qualified_name: qualified_name.clone(),
                argument_count: None,
                argument_types: Vec::new(),
                allowed_target_kinds: vec!["macro".to_owned()],
                hierarchy: None,
                allow_external: qualified_name.as_deref().is_some_and(|qualified| {
                    !rust_identity_is_internal(&self.module_or_package, qualified)
                }),
            },
        )?;
        Ok(())
    }

    fn add_rust_path_candidate(
        &mut self,
        role: SemanticRole,
        relation: CandidateRelation,
        owner: &DeclarationContext,
        node: Node<'_>,
        allowed_target_kinds: Option<Vec<String>>,
    ) -> Result<(), EvidenceError> {
        self.add_rust_path_candidate_with_context(
            role,
            relation,
            owner,
            node,
            allowed_target_kinds,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn add_rust_path_candidate_with_context(
        &mut self,
        role: SemanticRole,
        relation: CandidateRelation,
        owner: &DeclarationContext,
        node: Node<'_>,
        allowed_target_kinds: Option<Vec<String>>,
        context: Option<&str>,
    ) -> Result<(), EvidenceError> {
        let raw = self.text(node);
        let (qualifier, spelling) = split_qualified(&raw);
        let qualified_name = rust_qualify_evidence_path(self, owner, &raw, node.start_byte());
        self.add_rust_occurrence_candidate(
            role,
            relation,
            owner,
            node,
            qualified_name.as_deref(),
            allowed_target_kinds.unwrap_or_else(|| target_kinds_for_relation(relation)),
            qualified_name.as_deref().is_some_and(|qualified| {
                !rust_identity_is_internal(&self.module_or_package, qualified)
            }),
            context,
        )?;
        let _ = (qualifier, spelling);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn add_rust_occurrence_candidate(
        &mut self,
        role: SemanticRole,
        relation: CandidateRelation,
        owner: &DeclarationContext,
        node: Node<'_>,
        qualified_name: Option<&str>,
        allowed_target_kinds: Vec<String>,
        allow_external: bool,
        context: Option<&str>,
    ) -> Result<(), EvidenceError> {
        let raw = self.text(node);
        let (qualifier, spelling) = split_qualified(&raw);
        if spelling.is_empty() {
            return Ok(());
        }
        let binding_name = qualifier.map(qualified_binding_head).unwrap_or(spelling);
        let binding = self
            .binding_for_occurrence(owner, binding_name, node.start_byte(), true)
            .or_else(|| self.rust_wildcard_binding(owner, node.start_byte()))
            .cloned();
        let exact_type_parameter = qualified_name
            .and_then(|qualified| self.rust_type_parameters_by_qualified_name.get(qualified))
            .map(|parameter| parameter.fact_id.clone());
        let exact_associated_type = qualified_name
            .and_then(|qualified| self.rust_associated_types_by_qualified_name.get(qualified))
            .and_then(|types| {
                let [associated_type] = types.as_slice() else {
                    return None;
                };
                Some(associated_type.fact_id.clone())
            });
        let hierarchy = if relation == CandidateRelation::Extends && owner.kind == "trait" {
            let complete = rust_ancestor_of_kind(node, "trait_item")
                .is_some_and(|declaration| !self.overlaps_parser_error(declaration));
            Some(HierarchyConstraint::DirectBase {
                base_set_complete: complete,
            })
        } else if qualifier == Some("Self") && qualified_name.is_none() {
            self.rust_impl_for_node(node).and_then(|implementation| {
                implementation
                    .owner_declaration_id
                    .as_ref()
                    .and_then(|receiver_id| {
                        implementation
                            .trait_qualified_name
                            .as_ref()
                            .map(|trait_name| HierarchyConstraint::RustAssociatedType {
                                receiver_declaration_id: receiver_id.clone(),
                                receiver_qualified_name: implementation.type_qualified_name.clone(),
                                trait_qualified_name: trait_name.clone(),
                            })
                    })
            })
        } else {
            None
        };
        let occurrence_id = self.builder.occur_with_context(
            role,
            &owner.fact_id,
            spelling,
            qualifier,
            Some(&owner.scope_id),
            context,
            range_for_node(self.source_file, node),
        )?;
        self.builder.relate(
            relation,
            &owner.fact_id,
            Some(&occurrence_id),
            binding.as_deref(),
            spelling,
            ResolutionConstraint {
                exact_target_declaration_id: exact_type_parameter.or(exact_associated_type),
                exact_language: Some(self.language.to_owned()),
                module_or_package: qualified_name
                    .and_then(|qualified| rust_qualified_parent(qualified).map(str::to_owned))
                    .or_else(|| Some(self.module_or_package.clone())),
                scope_id: Some(owner.scope_id.clone()),
                qualified_name: qualified_name.map(str::to_owned),
                argument_count: None,
                argument_types: Vec::new(),
                allowed_target_kinds,
                hierarchy,
                allow_external,
            },
        )?;
        Ok(())
    }

    fn rust_wildcard_binding(
        &self,
        owner: &DeclarationContext,
        use_start: usize,
    ) -> Option<&String> {
        if self.import_binding_is_ambiguous(owner, "*") {
            return None;
        }
        self.import_binding_version_at(owner, "*", use_start, true)
            .map(|binding| &binding.binding_id)
    }

    fn extract_go(&mut self, root: Node<'_>) -> Result<(), EvidenceError> {
        let file = self.file.clone().ok_or_else(|| {
            EvidenceError::new(
                EvidenceErrorCode::InvalidFact,
                "non-empty Go source has no file evidence",
            )
        })?;
        self.collect_go_collection_types(root, &file);
        self.collect_go_declarations(root, &file)?;
        self.walk_go_evidence(root, &file, true)
    }

    fn collect_go_collection_types(&mut self, node: Node<'_>, file: &DeclarationContext) {
        if matches!(
            node.kind(),
            "function_declaration" | "method_declaration" | "func_literal"
        ) {
            return;
        }
        if matches!(node.kind(), "type_spec" | "type_alias")
            && let Some(name_node) = node.child_by_field_name("name")
            && let Some(type_node) = node.child_by_field_name("type")
            && let Some(target) = go_range_value_type_target(type_node)
            && let Some(element_type) = self.go_qualified_type_target(file, target)
        {
            self.go_collection_element_types.insert(
                format!("{}.{}", self.module_or_package, self.text(name_node)),
                element_type,
            );
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(|child| child.is_named()) {
            self.collect_go_collection_types(child, file);
        }
    }

    fn collect_go_declarations(
        &mut self,
        node: Node<'_>,
        file: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        if matches!(node.kind(), "type_spec" | "type_alias") {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = self.text(name_node);
                let qualified_name = format!("{}.{}", self.module_or_package, name);
                let graph_node_id =
                    self.unique_graph_id(make_id(&[&self.module_or_package, &name]), node);
                let kind = if node.kind() == "type_alias" {
                    "type_alias"
                } else if has_descendant(node, "struct_type") {
                    "struct"
                } else if has_descendant(node, "interface_type") {
                    "interface"
                } else {
                    "type_alias"
                };
                let metadata = self.declaration_metadata(node);
                let fact_id = self.builder.declare_with_metadata(
                    kind,
                    &graph_node_id,
                    &name,
                    &qualified_name,
                    Some(&self.module_or_package),
                    Some(&file.scope_id),
                    range_for_node(self.source_file, name_node),
                    metadata,
                )?;
                let scope_id = self.builder.open_scope(
                    kind,
                    Some(&fact_id),
                    Some(&file.scope_id),
                    range_for_node(self.source_file, node),
                )?;
                self.scope_parents
                    .insert(scope_id.clone(), file.scope_id.clone());
                let context = DeclarationContext {
                    fact_id,
                    scope_id,
                    graph_node_id,
                    name,
                    qualified_name,
                    kind: kind.to_owned(),
                    enclosing_type_qualified_name: None,
                };
                self.add_ownership(file, &context)?;
                self.declarations.insert(node.id(), context.clone());
                if kind == "interface" {
                    let mut interfaces = Vec::new();
                    collect_nodes(node, "interface_type", &mut interfaces);
                    for interface in interfaces {
                        self.collect_go_interface_methods(interface, &context)?;
                    }
                }
            }
            return Ok(());
        }
        if matches!(node.kind(), "function_declaration" | "method_declaration") {
            let Some(name_node) = node.child_by_field_name("name") else {
                return Ok(());
            };
            let name = self.text(name_node);
            let receiver = (node.kind() == "method_declaration")
                .then(|| go_receiver_name(node, self.source))
                .flatten();
            let receiver_graph = receiver
                .as_deref()
                .map(|receiver| make_id(&[&self.module_or_package, receiver]));
            let graph_node_id = receiver_graph.as_deref().map_or_else(
                || make_id(&[&self.stem, &name]),
                |receiver| make_id(&[receiver, &name]),
            );
            let graph_node_id = self.unique_graph_id(graph_node_id, node);
            let qualified_name = receiver.as_deref().map_or_else(
                || format!("{}.{}", self.module_or_package, name),
                |receiver| format!("{}.{}::{name}", self.module_or_package, receiver),
            );
            let kind = if receiver.is_some() {
                "method"
            } else {
                "function"
            };
            let mut metadata = self.declaration_metadata(node);
            let (parameter_count, variadic) = go_parameter_signature(node);
            metadata.parameter_count = Some(parameter_count);
            metadata.variadic = variadic;
            let fact_id = self.builder.declare_with_metadata(
                kind,
                &graph_node_id,
                &name,
                &qualified_name,
                Some(&self.module_or_package),
                Some(&file.scope_id),
                range_for_node(self.source_file, name_node),
                metadata,
            )?;
            let scope_id = self.builder.open_scope(
                kind,
                Some(&fact_id),
                Some(&file.scope_id),
                range_for_node(self.source_file, node),
            )?;
            self.scope_parents
                .insert(scope_id.clone(), file.scope_id.clone());
            let context = DeclarationContext {
                fact_id,
                scope_id,
                graph_node_id,
                name,
                qualified_name,
                kind: kind.to_owned(),
                enclosing_type_qualified_name: None,
            };
            self.add_ownership(file, &context)?;
            self.record_go_return_types(&context, file, node);
            self.declarations.insert(node.id(), context);
            return Ok(());
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(|child| child.is_named()) {
            self.collect_go_declarations(child, file)?;
        }
        Ok(())
    }

    fn record_go_return_types(
        &mut self,
        declaration: &DeclarationContext,
        owner: &DeclarationContext,
        node: Node<'_>,
    ) {
        let Some(result) = node.child_by_field_name("result") else {
            return;
        };
        let result_types = go_result_type_nodes(result);
        if result_types.is_empty() {
            return;
        }
        let direct_types = result_types
            .iter()
            .map(|type_node| {
                type_node
                    .and_then(go_direct_type_target)
                    .and_then(|target| self.go_qualified_type_target(owner, target))
            })
            .collect::<Vec<_>>();
        if direct_types.iter().any(Option::is_some) {
            self.go_return_types
                .insert(declaration.qualified_name.clone(), direct_types);
        }
        let range_types = result_types
            .iter()
            .map(|type_node| {
                type_node.and_then(|type_node| self.go_collection_element_type(owner, type_node))
            })
            .collect::<Vec<_>>();
        if range_types.iter().any(Option::is_some) {
            self.go_range_return_types
                .insert(declaration.qualified_name.clone(), range_types);
        }
    }

    fn collect_go_interface_methods(
        &mut self,
        interface: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        let mut methods = Vec::new();
        collect_nodes(interface, "method_elem", &mut methods);
        for method in methods {
            let Some(name_node) = method.child_by_field_name("name") else {
                continue;
            };
            let name = self.text(name_node);
            if name.is_empty() {
                continue;
            }
            let qualified_name = format!("{}::{name}", owner.qualified_name);
            let graph_node_id =
                self.unique_graph_id(make_id(&[&owner.graph_node_id, &name]), method);
            let mut metadata = self.declaration_metadata(method);
            let (parameter_count, variadic) = go_parameter_signature(method);
            metadata.parameter_count = Some(parameter_count);
            metadata.variadic = variadic;
            let fact_id = self.builder.declare_with_metadata(
                "method",
                &graph_node_id,
                &name,
                &qualified_name,
                Some(&self.module_or_package),
                Some(&owner.scope_id),
                range_for_node(self.source_file, name_node),
                metadata,
            )?;
            let scope_id = self.builder.open_scope(
                "method",
                Some(&fact_id),
                Some(&owner.scope_id),
                range_for_node(self.source_file, method),
            )?;
            self.scope_parents
                .insert(scope_id.clone(), owner.scope_id.clone());
            let context = DeclarationContext {
                fact_id,
                scope_id,
                graph_node_id,
                name,
                qualified_name,
                kind: "method".to_owned(),
                enclosing_type_qualified_name: Some(owner.qualified_name.clone()),
            };
            self.add_ownership(owner, &context)?;
            self.record_go_return_types(&context, owner, method);
            self.declarations.insert(method.id(), context);
        }
        Ok(())
    }

    fn walk_go_evidence(
        &mut self,
        node: Node<'_>,
        owner: &DeclarationContext,
        root: bool,
    ) -> Result<(), EvidenceError> {
        let mut active = self
            .declarations
            .get(&node.id())
            .cloned()
            .unwrap_or_else(|| owner.clone());
        if node.kind() == "func_literal" {
            let parent_scope_id = active.scope_id.clone();
            let scope_id = self.builder.open_scope(
                "closure",
                None,
                Some(&parent_scope_id),
                range_for_node(self.source_file, node),
            )?;
            self.scope_parents.insert(scope_id.clone(), parent_scope_id);
            active.scope_id = scope_id;
            self.add_go_typed_bindings(node, &active)?;
        }
        if matches!(
            node.kind(),
            "function_declaration" | "method_declaration" | "method_elem"
        ) {
            self.add_go_typed_bindings(node, &active)?;
        }
        match node.kind() {
            "import_declaration" => {
                self.add_go_imports(node, &active)?;
                return Ok(());
            }
            "call_expression" => self.add_call(node, &active, "call_expression")?,
            "method_declaration" => self.add_go_receiver(node, &active)?,
            "field_declaration" => self.add_go_field_types(node, &active)?,
            "type_elem" => self.add_go_embedded_types(node, &active)?,
            _ => {}
        }
        if matches!(
            node.kind(),
            "function_declaration" | "method_declaration" | "method_elem" | "func_literal"
        ) || (matches!(node.kind(), "type_spec" | "type_alias")
            && !has_descendant(node, "struct_type")
            && !has_descendant(node, "interface_type"))
        {
            self.add_go_type_references(node, &active)?;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(|child| child.is_named()) {
            if !root
                && matches!(
                    child.kind(),
                    "function_declaration" | "method_declaration" | "type_spec" | "type_alias"
                )
                && !self.declarations.contains_key(&child.id())
            {
                continue;
            }
            self.walk_go_evidence(child, &active, false)?;
        }
        Ok(())
    }

    fn add_go_typed_bindings(
        &mut self,
        declaration: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        for field in ["receiver", "parameters", "result"] {
            let Some(parameters) = declaration.child_by_field_name(field) else {
                continue;
            };
            let mut parameter_declarations = Vec::new();
            collect_nodes(
                parameters,
                "parameter_declaration",
                &mut parameter_declarations,
            );
            collect_nodes(
                parameters,
                "variadic_parameter_declaration",
                &mut parameter_declarations,
            );
            for parameter in parameter_declarations {
                let mut cursor = parameter.walk();
                let names = parameter
                    .children_by_field_name("name", &mut cursor)
                    .filter_map(|name_node| {
                        let name = self.text(name_node);
                        (!name.is_empty() && name != "_").then_some((name, name_node))
                    })
                    .collect::<Vec<_>>();
                if names.is_empty() {
                    continue;
                }
                self.local_shadows
                    .entry(owner.scope_id.clone())
                    .or_default()
                    .extend(names.iter().map(|(name, _)| name.clone()));
                let Some(type_node) = parameter.child_by_field_name("type") else {
                    continue;
                };
                if parameter.kind() == "variadic_parameter_declaration" {
                    let Some(element_target) = go_direct_type_target(type_node) else {
                        continue;
                    };
                    let Some(element_type) = self.go_qualified_type_target(owner, element_target)
                    else {
                        continue;
                    };
                    self.go_collection_binding_element_types
                        .entry(owner.scope_id.clone())
                        .or_default()
                        .extend(
                            names
                                .into_iter()
                                .map(|(name, _)| (name, element_type.clone())),
                        );
                    continue;
                }
                let mut targets = Vec::new();
                collect_named_targets(
                    type_node,
                    &["type_identifier", "qualified_type"],
                    &mut targets,
                );
                let [target] = targets.as_slice() else {
                    continue;
                };
                let raw_target = self.text(*target);
                let (qualifier, spelling) = split_qualified(&raw_target);
                if spelling.is_empty() || is_go_predeclared_type(spelling) {
                    continue;
                }
                let qualified_target = qualifier
                    .and_then(|qualifier| {
                        self.imported_target_for_occurrence(
                            owner,
                            qualifier,
                            parameter.start_byte(),
                            true,
                        )
                    })
                    .map_or_else(
                        || format!("{}.{}", self.module_or_package, spelling),
                        |module| format!("{module}.{spelling}"),
                    );
                for (name, name_node) in names {
                    let binding_id = self.builder.bind(
                        BindingKind::LocalAlias,
                        &name,
                        &qualified_target,
                        None,
                        Some(&owner.scope_id),
                        range_for_node(self.source_file, name_node),
                    )?;
                    self.local_bindings
                        .entry(owner.scope_id.clone())
                        .or_default()
                        .insert(name.clone(), binding_id);
                    self.local_targets
                        .entry(owner.scope_id.clone())
                        .or_default()
                        .insert(name, qualified_target.clone());
                }
            }
        }
        Ok(())
    }

    fn add_go_imports(
        &mut self,
        declaration: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        let mut specs = Vec::new();
        collect_nodes(declaration, "import_spec", &mut specs);
        for spec in specs {
            let Some(path_node) = spec.child_by_field_name("path") else {
                continue;
            };
            let target = self.text(path_node).trim_matches('"').to_owned();
            if target.is_empty() {
                continue;
            }
            let explicit = spec
                .child_by_field_name("name")
                .map(|name| self.text(name))
                .filter(|name| !matches!(name.as_str(), "" | "_" | "."));
            let local = explicit
                .clone()
                .unwrap_or_else(|| target.rsplit('/').next().unwrap_or_default().to_owned());
            if local.is_empty() {
                continue;
            }
            let kind = if explicit.is_some() {
                BindingKind::ImportAlias
            } else {
                BindingKind::Package
            };
            let binding_id = self.builder.bind(
                kind,
                &local,
                &target,
                None,
                Some(&owner.scope_id),
                range_for_node(self.source_file, spec),
            )?;
            self.record_import_binding(owner, &local, &target, &binding_id, spec.end_byte());
            let occurrence_id = self.builder.occur(
                SemanticRole::Import,
                &owner.fact_id,
                &local,
                None,
                Some(&owner.scope_id),
                range_for_node(self.source_file, spec),
            )?;
            let target_spelling = target.rsplit('/').next().unwrap_or(&target).to_owned();
            self.builder.relate(
                CandidateRelation::Imports,
                &owner.fact_id,
                Some(&occurrence_id),
                Some(&binding_id),
                &target_spelling,
                ResolutionConstraint {
                    exact_target_declaration_id: None,
                    exact_language: Some(self.language.to_owned()),
                    module_or_package: Some(target.clone()),
                    scope_id: Some(owner.scope_id.clone()),
                    qualified_name: Some(target),
                    argument_count: None,
                    argument_types: Vec::new(),
                    allowed_target_kinds: vec!["file".to_owned(), "package".to_owned()],
                    hierarchy: None,
                    allow_external: true,
                },
            )?;
        }
        Ok(())
    }

    fn add_go_receiver(
        &mut self,
        declaration: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        let Some(receiver) = declaration.child_by_field_name("receiver") else {
            return Ok(());
        };
        let Some(name) = go_receiver_name(declaration, self.source) else {
            return Ok(());
        };
        self.add_relationship_occurrence(
            SemanticRole::Receiver,
            CandidateRelation::References,
            owner,
            &name,
            None,
            receiver,
        )
    }

    fn add_go_field_types(
        &mut self,
        field: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        let has_name = has_descendant(field, "field_identifier");
        let Some(type_node) = field.child_by_field_name("type").or_else(|| {
            let mut cursor = field.walk();
            field
                .children(&mut cursor)
                .find(|child| child.is_named() && child.kind() != "field_identifier")
        }) else {
            return Ok(());
        };
        if !has_name
            && let Some(target) = go_direct_type_target(type_node)
            && let Some(qualified_target) = self.go_qualified_type_target(owner, target)
        {
            let raw = self.text(target);
            let (_, name) = split_qualified(&raw);
            if !name.is_empty() && name != "_" {
                self.builder.bind(
                    BindingKind::Member,
                    name,
                    &qualified_target,
                    None,
                    Some(&owner.scope_id),
                    range_for_node(self.source_file, target),
                )?;
                self.go_member_types.insert(
                    (owner.qualified_name.clone(), name.to_owned()),
                    qualified_target,
                );
            }
        }
        if has_name
            && let Some(target) = go_direct_type_target(type_node)
            && let Some(qualified_target) = self.go_qualified_type_target(owner, target)
        {
            let mut names = Vec::new();
            collect_nodes(field, "field_identifier", &mut names);
            for name_node in names {
                let name = self.text(name_node);
                if !name.is_empty() && name != "_" {
                    self.builder.bind(
                        BindingKind::Member,
                        &name,
                        &qualified_target,
                        None,
                        Some(&owner.scope_id),
                        range_for_node(self.source_file, name_node),
                    )?;
                    self.go_member_types.insert(
                        (owner.qualified_name.clone(), name),
                        qualified_target.clone(),
                    );
                }
            }
        }
        if has_name
            && let Some(qualified_target) = self.go_collection_element_type(owner, type_node)
        {
            let mut names = Vec::new();
            collect_nodes(field, "field_identifier", &mut names);
            for name_node in names {
                let name = self.text(name_node);
                if !name.is_empty() && name != "_" {
                    self.go_range_member_types.insert(
                        (owner.qualified_name.clone(), name),
                        qualified_target.clone(),
                    );
                }
            }
        }
        let mut targets = Vec::new();
        collect_named_targets(
            type_node,
            &["type_identifier", "qualified_type"],
            &mut targets,
        );
        for target in targets {
            let raw = self.text(target);
            let (qualifier, spelling) = split_qualified(&raw);
            let (role, relation) = if has_name {
                (SemanticRole::TypeReference, CandidateRelation::References)
            } else {
                if !matches!(
                    owner.kind.as_str(),
                    "class" | "struct" | "interface" | "trait" | "type_alias"
                ) {
                    continue;
                }
                (SemanticRole::Embedding, CandidateRelation::Embeds)
            };
            self.add_relationship_occurrence(role, relation, owner, spelling, qualifier, target)?;
        }
        Ok(())
    }

    fn go_qualified_type_target(
        &self,
        owner: &DeclarationContext,
        target: Node<'_>,
    ) -> Option<String> {
        let raw = self.text(target);
        let (qualifier, spelling) = split_qualified(&raw);
        if spelling.is_empty() || is_go_predeclared_type(spelling) {
            return None;
        }
        Some(
            qualifier
                .and_then(|qualifier| {
                    self.imported_target_for_occurrence(owner, qualifier, target.start_byte(), true)
                })
                .map_or_else(
                    || format!("{}.{}", self.module_or_package, spelling),
                    |module| format!("{module}.{spelling}"),
                ),
        )
    }

    fn add_go_embedded_types(
        &mut self,
        element: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        if !matches!(
            owner.kind.as_str(),
            "class" | "struct" | "interface" | "trait" | "type_alias"
        ) {
            return Ok(());
        }
        let mut targets = Vec::new();
        collect_named_targets(
            element,
            &["type_identifier", "qualified_type"],
            &mut targets,
        );
        for target in targets {
            let raw = self.text(target);
            if raw == owner.name || is_go_predeclared_type(&raw) {
                continue;
            }
            let (qualifier, spelling) = split_qualified(&raw);
            if let Some(qualified_target) = self.go_qualified_type_target(owner, target) {
                self.builder.bind(
                    BindingKind::Member,
                    spelling,
                    &qualified_target,
                    None,
                    Some(&owner.scope_id),
                    range_for_node(self.source_file, target),
                )?;
                self.go_member_types.insert(
                    (owner.qualified_name.clone(), spelling.to_owned()),
                    qualified_target,
                );
            }
            self.add_relationship_occurrence(
                SemanticRole::Embedding,
                CandidateRelation::Embeds,
                owner,
                spelling,
                qualifier,
                target,
            )?;
        }
        Ok(())
    }

    fn add_go_type_references(
        &mut self,
        declaration: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        let mut roots = Vec::new();
        if matches!(
            declaration.kind(),
            "function_declaration" | "method_declaration" | "method_elem" | "func_literal"
        ) {
            roots.extend(
                ["receiver", "parameters"]
                    .into_iter()
                    .filter_map(|field| declaration.child_by_field_name(field))
                    .map(|root| (root, CandidateRelation::References)),
            );
            roots.extend(declaration.child_by_field_name("result").map(|root| {
                // Anonymous Go functions currently have a lexical scope but no
                // published callable declaration. Attributing their result to
                // the enclosing declaration would invent a return contract (and
                // at package scope would produce an invalid file -> type edge).
                // Retain the source-backed type dependency as a reference until
                // closures have their own stable graph identity.
                let relation = if declaration.kind() == "func_literal" {
                    CandidateRelation::References
                } else {
                    CandidateRelation::Returns
                };
                (root, relation)
            }));
        } else if let Some(type_node) = declaration.child_by_field_name("type") {
            roots.push((type_node, CandidateRelation::References));
        }
        for (root, relation) in roots {
            let mut targets = Vec::new();
            collect_named_targets(root, &["type_identifier", "qualified_type"], &mut targets);
            for target in targets {
                let raw = self.text(target);
                let self_reference = matches!(
                    owner.kind.as_str(),
                    "class" | "struct" | "interface" | "trait" | "type_alias"
                ) && raw == owner.name;
                if self_reference || is_go_predeclared_type(&raw) {
                    continue;
                }
                let (qualifier, spelling) = split_qualified(&raw);
                let qualified_name_override = (relation == CandidateRelation::Returns)
                    .then(|| self.go_qualified_type_target(owner, target))
                    .flatten();
                self.add_relationship_occurrence_with_hierarchy(
                    SemanticRole::TypeReference,
                    relation,
                    owner,
                    spelling,
                    qualifier,
                    target,
                    qualified_name_override,
                    None,
                )?;
            }
        }
        Ok(())
    }

    fn add_call(
        &mut self,
        call: Node<'_>,
        owner: &DeclarationContext,
        call_kind: &str,
    ) -> Result<(), EvidenceError> {
        let function = call
            .child_by_field_name("function")
            .or_else(|| call.child_by_field_name("func"));
        let Some(function) = function else {
            return Ok(());
        };
        let raw = self.text(function);
        let (qualifier, spelling) = split_qualified(&raw);
        if spelling.is_empty() {
            return Ok(());
        }
        let python_super_receiver = (self.language == "python")
            .then(|| python_super_receiver(function, self.source))
            .flatten();
        let super_dispatch = if let Some(receiver) = python_super_receiver {
            if !python_super_call_is_builtin(receiver, call, owner, self) {
                return Ok(());
            }
            let Some(receiver_qualified_name) = owner.enclosing_type_qualified_name.clone() else {
                return Ok(());
            };
            Some(HierarchyConstraint::ReceiverDispatch {
                receiver_qualified_name,
                strategy: ReceiverDispatchStrategy::C3AfterReceiver,
            })
        } else {
            None
        };
        let python_bound_receiver = if super_dispatch.is_none() && self.language == "python" {
            qualifier
                .filter(|qualifier| matches!(*qualifier, "self" | "cls"))
                .map(|qualifier| {
                    self.python_bound_method_receiver(call, qualifier, function.start_byte())
                })
                .transpose()?
                .flatten()
        } else {
            None
        };
        if self.language == "python"
            && qualifier.is_some_and(|qualifier| matches!(qualifier, "self" | "cls"))
            && python_bound_receiver.is_none()
        {
            return Ok(());
        }
        let local_python_receiver = if super_dispatch.is_none() && self.language == "python" {
            if let Some(qualifier) = qualifier {
                let mut receiver =
                    self.python_local_class_receiver(owner, qualifier, function.start_byte(), call);
                if receiver.is_none() {
                    receiver = self.python_local_initializer_receiver(
                        owner,
                        qualifier,
                        function.start_byte(),
                        call,
                    )?;
                }
                receiver.or_else(|| {
                    self.python_module_singleton_receiver(owner, qualifier, function.start_byte())
                })
            } else {
                None
            }
        } else {
            None
        };
        let receiver_dispatch = super_dispatch
            .or_else(|| {
                python_bound_receiver.map(|receiver_qualified_name| {
                    HierarchyConstraint::ReceiverDispatch {
                        receiver_qualified_name,
                        strategy: ReceiverDispatchStrategy::C3FromReceiver,
                    }
                })
            })
            .or_else(|| {
                local_python_receiver
                    .as_ref()
                    .map(
                        |receiver_qualified_name| HierarchyConstraint::ReceiverDispatch {
                            receiver_qualified_name: receiver_qualified_name.clone(),
                            strategy: ReceiverDispatchStrategy::C3FromReceiver,
                        },
                    )
            });
        let lookup_name = qualifier.map(qualified_binding_head).unwrap_or(spelling);
        let allow_later_file_binding =
            self.language == "python" && matches!(owner.kind.as_str(), "function" | "method");
        if self.import_binding_declared_but_not_visible(
            owner,
            lookup_name,
            function.start_byte(),
            allow_later_file_binding,
        ) {
            return Ok(());
        }
        if self.import_binding_is_ambiguous(owner, lookup_name) {
            return Ok(());
        }
        if self.language == "go"
            && qualifier.is_none()
            && self.go_name_is_locally_bound(call, spelling)
        {
            return Ok(());
        }
        if self.language == "python"
            && qualifier.is_none()
            && self.python_name_is_statically_local(owner, spelling)
        {
            return Ok(());
        }
        let (role, relation) = (SemanticRole::Call, CandidateRelation::Calls);
        let (argument_count, argument_types) = if self.language == "go" {
            (
                call.child_by_field_name("arguments")
                    .and_then(|arguments| u32::try_from(arguments.named_child_count()).ok()),
                Vec::new(),
            )
        } else if self.language == "python" {
            python_call_argument_shape(call)
        } else {
            (None, Vec::new())
        };
        let binding_name = qualifier.map(qualified_binding_head).unwrap_or(spelling);
        let call_result_binding = if self.language == "go" {
            qualifier
                .map(|qualifier| {
                    self.go_call_result_binding_for_occurrence(owner, function, qualifier)
                })
                .transpose()?
                .flatten()
        } else {
            None
        };
        let binding = call_result_binding.or_else(|| {
            self.binding_for_occurrence(
                owner,
                binding_name,
                function.start_byte(),
                allow_later_file_binding,
            )
            .or_else(|| {
                self.python_wildcard_binding(owner, function.start_byte(), allow_later_file_binding)
            })
            .cloned()
        });
        let qualified_name = if receiver_dispatch.is_some() {
            None
        } else {
            qualifier
                .and_then(|qualifier| {
                    self.local_target_for(owner, qualifier)
                        .map(|target| format!("{target}::{spelling}"))
                })
                .or_else(|| {
                    (self.language == "go")
                        .then(|| self.go_selector_receiver_type(owner, function))
                        .flatten()
                        .map(|target| format!("{target}::{spelling}"))
                })
                .or_else(|| {
                    qualifier
                        .and_then(|qualifier| {
                            self.imported_qualified_target_for(
                                owner,
                                qualifier,
                                function.start_byte(),
                                allow_later_file_binding,
                            )
                        })
                        .map(|target| format!("{target}.{spelling}"))
                })
                .or_else(|| {
                    qualifier.is_none().then(|| {
                        self.imported_target_for_occurrence(
                            owner,
                            spelling,
                            function.start_byte(),
                            allow_later_file_binding,
                        )
                        .cloned()
                    })?
                })
        };
        let occurrence_id = self.builder.occur(
            role,
            &owner.fact_id,
            spelling,
            qualifier,
            Some(&owner.scope_id),
            range_for_node(self.source_file, function),
        )?;
        self.builder.relate(
            relation,
            &owner.fact_id,
            Some(&occurrence_id),
            binding.as_deref(),
            spelling,
            ResolutionConstraint {
                exact_target_declaration_id: None,
                exact_language: Some(self.language.to_owned()),
                module_or_package: qualified_name
                    .as_deref()
                    .and_then(|qualified| {
                        qualified
                            .rsplit_once('.')
                            .map(|(module, _)| module.to_owned())
                    })
                    .or_else(|| Some(self.module_or_package.clone())),
                scope_id: Some(owner.scope_id.clone()),
                qualified_name: qualified_name.clone(),
                argument_count,
                argument_types,
                allowed_target_kinds: if self.language == "python" {
                    vec![
                        "function".to_owned(),
                        "method".to_owned(),
                        "class".to_owned(),
                        "type_alias".to_owned(),
                    ]
                } else if self.language == "go" {
                    vec![
                        "function".to_owned(),
                        "method".to_owned(),
                        "struct".to_owned(),
                        "interface".to_owned(),
                        "type_alias".to_owned(),
                    ]
                } else {
                    vec!["function".to_owned(), "method".to_owned()]
                },
                hierarchy: receiver_dispatch,
                allow_external: qualified_name.is_some() && python_super_receiver.is_none(),
            },
        )?;
        let _ = call_kind;
        Ok(())
    }

    fn python_local_class_receiver(
        &self,
        owner: &DeclarationContext,
        receiver: &str,
        use_start: usize,
        call: Node<'_>,
    ) -> Option<String> {
        let use_start_u64 = u64::try_from(use_start).ok()?;
        let mut scope_id = Some(owner.scope_id.as_str());
        for _ in 0..64 {
            let current = scope_id?;
            let mut matches = self
                .builder
                .batch
                .declarations
                .iter()
                .filter(|declaration| {
                    declaration.kind == "class"
                        && declaration.name == receiver
                        && declaration.scope_id.as_deref() == Some(current)
                        && declaration.range.end_byte <= use_start_u64
                })
                .take(2);
            let declaration = matches.next();
            if matches.next().is_some() {
                return None;
            }
            if let Some(declaration) = declaration {
                let declaration_end = usize::try_from(declaration.range.end_byte).ok()?;
                let mut syntax_scope = Some(call);
                while let Some(node) = syntax_scope {
                    if matches!(node.kind(), "function_definition" | "class_definition")
                        && self
                            .declarations
                            .get(&node.id())
                            .is_some_and(|context| context.fact_id == owner.fact_id)
                    {
                        let body = node.child_by_field_name("body").unwrap_or(node);
                        if self.python_name_rebound_between(
                            body,
                            declaration_end,
                            use_start,
                            receiver,
                        ) {
                            return None;
                        }
                        return Some(declaration.qualified_name.clone());
                    }
                    syntax_scope = node.parent();
                }
                return None;
            }
            scope_id = self.scope_parents.get(current).map(String::as_str);
        }
        None
    }

    fn python_local_initializer_receiver(
        &mut self,
        owner: &DeclarationContext,
        receiver: &str,
        use_start: usize,
        call: Node<'_>,
    ) -> Result<Option<String>, EvidenceError> {
        let inferred = (|| {
            if !valid_python_identifier(receiver) || matches!(receiver, "self" | "cls") {
                return None;
            }
            let mut scope_id = Some(owner.scope_id.as_str());
            for _ in 0..64 {
                let Some(current) = scope_id else {
                    break;
                };
                if self
                    .python_global_names
                    .get(current)
                    .is_some_and(|names| names.contains(receiver))
                {
                    return None;
                }
                scope_id = self.scope_parents.get(current).map(String::as_str);
            }

            let mut syntax_scope = Some(call);
            let function = loop {
                let node = syntax_scope?;
                if node.kind() == "function_definition"
                    && self
                        .declarations
                        .get(&node.id())
                        .is_some_and(|context| context.fact_id == owner.fact_id)
                {
                    break node;
                }
                syntax_scope = node.parent();
            };
            let body = function.child_by_field_name("body")?;
            let mut cursor = body.walk();
            let mut bindings = body
                .named_children(&mut cursor)
                .filter(|statement| statement.start_byte() < use_start)
                .filter(|statement| {
                    crate::engine::python_bound_names(*statement, self.source, true)
                        .contains(receiver)
                });
            let assignment = bindings.next()?;
            if bindings.next().is_some() || assignment.kind() != "assignment" {
                return None;
            }
            let left = assignment
                .child_by_field_name("left")
                .filter(|node| node.kind() == "identifier")?;
            if self.text(left) != receiver {
                return None;
            }
            let initializer = assignment
                .child_by_field_name("right")
                .filter(|node| node.kind() == "call")?;
            let called = initializer.child_by_field_name("function")?;
            let raw = self.text(called);
            let (qualifier, spelling) = split_qualified(&raw);
            if spelling.is_empty() {
                return None;
            }
            if qualifier.is_none() && self.python_name_is_statically_local(owner, spelling) {
                return None;
            }
            let local_declaration = qualifier
                .is_none()
                .then(|| {
                    self.python_unique_visible_declaration(
                        owner,
                        spelling,
                        called.start_byte(),
                        &["class", "function"],
                    )
                })
                .flatten();
            let local_target = qualifier
                .is_none()
                .then(|| {
                    self.local_target_for(owner, spelling).cloned().or_else(|| {
                        local_declaration
                            .as_ref()
                            .map(|declaration| declaration.qualified_name.clone())
                    })
                })
                .flatten();
            let initializer_target = qualifier
                .and_then(|qualifier| {
                    self.imported_qualified_target_for(owner, qualifier, called.start_byte(), true)
                        .map(|target| format!("{target}.{spelling}"))
                })
                .or_else(|| {
                    qualifier.is_none().then(|| {
                        local_target.or_else(|| {
                            self.imported_target_for_occurrence(
                                owner,
                                spelling,
                                called.start_byte(),
                                true,
                            )
                            .cloned()
                        })
                    })?
                });
            let receiver_type = initializer_target.and_then(|target| {
                if let Some(returned) = self.python_callable_return_types.get(&target) {
                    Some(returned.clone())
                } else if local_declaration
                    .as_ref()
                    .is_some_and(|declaration| declaration.kind != "class")
                {
                    None
                } else {
                    Some(target)
                }
            })?;
            Some((receiver_type, left, assignment.id()))
        })();
        let Some((receiver_type, left, assignment_id)) = inferred else {
            return Ok(None);
        };
        let binding_key = (owner.scope_id.clone(), receiver.to_owned(), assignment_id);
        if !self
            .python_call_result_binding_ids
            .contains_key(&binding_key)
        {
            let binding_id = self.builder.bind(
                BindingKind::CallResult,
                receiver,
                &receiver_type,
                None,
                Some(&owner.scope_id),
                range_for_node(self.source_file, left),
            )?;
            self.python_call_result_binding_ids
                .insert(binding_key, binding_id);
        }
        Ok(Some(receiver_type))
    }

    fn python_unique_visible_declaration(
        &self,
        owner: &DeclarationContext,
        name: &str,
        use_start: usize,
        kinds: &[&str],
    ) -> Option<DeclarationContext> {
        let mut visible_scopes = HashSet::new();
        let mut scope_id = Some(owner.scope_id.as_str());
        for _ in 0..64 {
            let Some(current) = scope_id else {
                break;
            };
            visible_scopes.insert(current.to_owned());
            scope_id = self.scope_parents.get(current).map(String::as_str);
        }
        let allow_later = matches!(owner.kind.as_str(), "function" | "method");
        let candidates = self
            .builder
            .batch
            .declarations
            .iter()
            .filter(|declaration| {
                declaration.language == "python"
                    && declaration.name == name
                    && kinds.contains(&declaration.kind.as_str())
                    && declaration
                        .scope_id
                        .as_ref()
                        .is_some_and(|scope| visible_scopes.contains(scope))
                    && (allow_later
                        || declaration.range.start_byte
                            < u64::try_from(use_start).unwrap_or(u64::MAX))
            })
            .filter_map(|declaration| {
                self.declarations
                    .values()
                    .find(|context| context.fact_id == declaration.id)
                    .cloned()
            })
            .collect::<Vec<_>>();
        match candidates.as_slice() {
            [candidate] => Some(candidate.clone()),
            _ => None,
        }
    }

    fn python_module_singleton_receiver(
        &self,
        owner: &DeclarationContext,
        receiver: &str,
        use_start: usize,
    ) -> Option<String> {
        if !valid_python_identifier(receiver)
            || self.python_name_is_statically_local(owner, receiver)
        {
            return None;
        }
        let mut scope_id = Some(owner.scope_id.as_str());
        for _ in 0..64 {
            let Some(current) = scope_id else {
                break;
            };
            if self
                .python_global_names
                .get(current)
                .is_some_and(|names| names.contains(receiver))
            {
                return None;
            }
            scope_id = self.scope_parents.get(current).map(String::as_str);
        }

        let file_scope = self.file.as_ref()?.scope_id.as_str();
        let allow_later_file_binding = matches!(owner.kind.as_str(), "function" | "method");
        let mut variables = self
            .builder
            .batch
            .declarations
            .iter()
            .filter(|declaration| {
                declaration.kind == "variable"
                    && declaration.name == receiver
                    && declaration.scope_id.as_deref() == Some(file_scope)
                    && (allow_later_file_binding
                        || usize::try_from(declaration.range.end_byte)
                            .is_ok_and(|end| end <= use_start))
            });
        let variable = variables.next()?;
        if variables.next().is_some() {
            return None;
        }

        let mut initializer_types = self
            .builder
            .batch
            .candidates
            .iter()
            .filter_map(|candidate| {
                (candidate.relation == CandidateRelation::TypeOf
                    && candidate.source_declaration_id == variable.id)
                    .then_some(candidate.constraints.exact_target_declaration_id.as_deref())
                    .flatten()
            });
        let initializer_type = initializer_types.next()?;
        if initializer_types.next().is_some() {
            return None;
        }
        self.builder
            .batch
            .declarations
            .iter()
            .find(|declaration| declaration.id == initializer_type && declaration.kind == "class")
            .map(|declaration| declaration.qualified_name.clone())
    }

    fn python_bound_method_receiver(
        &self,
        call: Node<'_>,
        receiver: &str,
        use_start: usize,
    ) -> Result<Option<String>, EvidenceError> {
        let mut ancestor = Some(call);
        while let Some(node) = ancestor {
            if node.kind() == "lambda"
                && crate::engine::python_bound_names(node, self.source, false).contains(receiver)
            {
                return Ok(None);
            }
            if matches!(
                node.kind(),
                "list_comprehension"
                    | "dictionary_comprehension"
                    | "set_comprehension"
                    | "generator_expression"
            ) && crate::engine::python_bound_names(node, self.source, true).contains(receiver)
            {
                return Ok(None);
            }
            if node.kind() == "function_definition" {
                let Some(context) = self.declarations.get(&node.id()) else {
                    return Ok(None);
                };
                if context.kind == "method" {
                    let Some(parameters) = node.child_by_field_name("parameters") else {
                        return Ok(None);
                    };
                    let mut cursor = parameters.walk();
                    let name = parameters
                        .named_children(&mut cursor)
                        .find_map(|parameter| {
                            if parameter.kind() == "identifier" {
                                Some(parameter)
                            } else {
                                parameter.child_by_field_name("name").or_else(|| {
                                    let mut cursor = parameter.walk();
                                    parameter
                                        .children(&mut cursor)
                                        .find(|child| child.kind() == "identifier")
                                })
                            }
                        });
                    let Some(name) = name.filter(|name| self.text(*name) == receiver) else {
                        return Ok(None);
                    };
                    if python_has_decorator(node, self.source, "staticmethod") {
                        return Ok(None);
                    }
                    if receiver == "cls" && !python_has_decorator(node, self.source, "classmethod")
                    {
                        return Ok(None);
                    }
                    let body = node.child_by_field_name("body").unwrap_or(node);
                    if self.python_name_rebound_between(body, name.end_byte(), use_start, receiver)
                    {
                        return Ok(None);
                    }
                    return Ok(context.enclosing_type_qualified_name.clone());
                }
                if crate::engine::python_bound_names(node, self.source, false).contains(receiver) {
                    return Ok(None);
                }
            }
            if node.kind() == "class_definition" {
                return Ok(None);
            }
            ancestor = node.parent();
        }
        Ok(None)
    }

    fn go_selector_receiver_type(
        &self,
        owner: &DeclarationContext,
        function: Node<'_>,
    ) -> Option<String> {
        let operand = function.child_by_field_name("operand")?;
        self.go_expression_type(owner, operand, 0, &mut HashSet::new())
    }

    fn go_call_result_binding_for_occurrence(
        &mut self,
        owner: &DeclarationContext,
        function: Node<'_>,
        qualifier: &str,
    ) -> Result<Option<String>, EvidenceError> {
        let Some(operand) = function.child_by_field_name("operand") else {
            return Ok(None);
        };
        let (result_call, output_index) = if operand.kind() == "call_expression" {
            (Some(operand), None)
        } else if operand.kind() == "identifier" && self.text(operand) == qualifier {
            let (initializer, output_index) =
                go_local_initializer_with_index_before(operand, qualifier, self.source)
                    .unwrap_or((operand, None));
            (
                (initializer.kind() == "call_expression").then_some(initializer),
                output_index,
            )
        } else {
            (None, None)
        };
        let Some(result_call) = result_call else {
            return Ok(None);
        };
        let Some(called) = result_call.child_by_field_name("function") else {
            return Ok(None);
        };
        let Some(qualified_callable) =
            self.go_callable_qualified_name(owner, called, 0, &mut HashSet::new())
        else {
            return Ok(None);
        };
        let key = (
            owner.scope_id.clone(),
            qualifier.to_owned(),
            result_call.start_byte(),
        );
        if let Some(binding) = self.go_call_result_bindings.get(&key) {
            return Ok(Some(binding.clone()));
        }
        let binding = self.builder.bind_with_output_index(
            BindingKind::CallResult,
            qualifier,
            &qualified_callable,
            None,
            Some(&owner.scope_id),
            output_index,
            None,
            None,
            None,
            range_for_node(self.source_file, result_call),
        )?;
        self.go_call_result_bindings.insert(key, binding.clone());
        Ok(Some(binding))
    }

    fn go_callable_qualified_name(
        &self,
        owner: &DeclarationContext,
        function: Node<'_>,
        depth: usize,
        visited: &mut HashSet<String>,
    ) -> Option<String> {
        if depth >= GO_TYPE_INFERENCE_DEPTH_LIMIT {
            return None;
        }
        match function.kind() {
            "identifier" => {
                let spelling = self.text(function);
                self.imported_target_for_occurrence(owner, &spelling, function.start_byte(), true)
                    .cloned()
                    .or_else(|| Some(format!("{}.{}", self.module_or_package, spelling)))
            }
            "selector_expression" => {
                let operand = function.child_by_field_name("operand")?;
                let field = function.child_by_field_name("field")?;
                let spelling = self.text(field);
                self.go_expression_type(owner, operand, depth + 1, visited)
                    .map(|receiver| format!("{receiver}::{spelling}"))
                    .or_else(|| {
                        (operand.kind() == "identifier")
                            .then(|| self.text(operand))
                            .and_then(|package| {
                                self.imported_target_for_occurrence(
                                    owner,
                                    &package,
                                    operand.start_byte(),
                                    true,
                                )
                            })
                            .map(|package| format!("{package}.{spelling}"))
                    })
            }
            _ => None,
        }
    }

    fn go_collection_element_type(
        &self,
        owner: &DeclarationContext,
        collection_type: Node<'_>,
    ) -> Option<String> {
        if let Some(target) = go_range_value_type_target(collection_type) {
            return self.go_qualified_type_target(owner, target);
        }
        if let Some(target) = go_direct_type_target(collection_type) {
            if target.kind() == "type_identifier"
                && go_local_type_declaration_before(target, &self.text(target), self.source)
            {
                return None;
            }
            let qualified_collection = self.go_qualified_type_target(owner, target)?;
            return self
                .go_collection_element_types
                .get(&qualified_collection)
                .cloned();
        }
        match collection_type.kind() {
            "parameter_declaration" => collection_type
                .child_by_field_name("type")
                .and_then(|node| self.go_collection_element_type(owner, node)),
            "parameter_list" => {
                let mut cursor = collection_type.walk();
                let mut children = collection_type
                    .children(&mut cursor)
                    .filter(|child| child.is_named());
                let child = children.next()?;
                children
                    .next()
                    .is_none()
                    .then(|| self.go_collection_element_type(owner, child))?
            }
            _ => None,
        }
    }

    fn go_local_value_type_inner(
        &self,
        owner: &DeclarationContext,
        use_node: Node<'_>,
        name: &str,
        depth: usize,
        visited: &mut HashSet<String>,
    ) -> Option<String> {
        if depth >= GO_TYPE_INFERENCE_DEPTH_LIMIT || !visited.insert(name.to_owned()) {
            return None;
        }
        let result = go_enclosing_range_value(use_node, name, self.source)
            .and_then(|range| self.go_range_expression_type(owner, range, depth + 1, visited))
            .or_else(|| self.local_target_for(owner, name).cloned())
            .or_else(|| {
                let (initializer, output_index) =
                    go_local_initializer_with_index_before(use_node, name, self.source)?;
                self.go_expression_type_at_output(
                    owner,
                    initializer,
                    depth + 1,
                    visited,
                    output_index,
                )
            });
        visited.remove(name);
        result
    }

    fn go_expression_type(
        &self,
        owner: &DeclarationContext,
        expression: Node<'_>,
        depth: usize,
        visited: &mut HashSet<String>,
    ) -> Option<String> {
        self.go_expression_type_at_output(owner, expression, depth, visited, None)
    }

    fn go_expression_type_at_output(
        &self,
        owner: &DeclarationContext,
        expression: Node<'_>,
        depth: usize,
        visited: &mut HashSet<String>,
        output_index: Option<u32>,
    ) -> Option<String> {
        if depth >= GO_TYPE_INFERENCE_DEPTH_LIMIT {
            return None;
        }
        match expression.kind() {
            "type_identifier" | "qualified_type" | "pointer_type" | "slice_type" | "array_type"
            | "map_type" | "channel_type" => go_direct_type_target(expression)
                .and_then(|target| self.go_qualified_type_target(owner, target)),
            "identifier" => self.go_local_value_type_inner(
                owner,
                expression,
                &self.text(expression),
                depth + 1,
                visited,
            ),
            "composite_literal" => expression
                .child_by_field_name("type")
                .and_then(go_direct_type_target)
                .and_then(|target| self.go_qualified_type_target(owner, target)),
            "unary_expression" | "parenthesized_expression" => expression
                .child_by_field_name("operand")
                .or_else(|| expression.named_child(0))
                .and_then(|inner| self.go_expression_type(owner, inner, depth + 1, visited)),
            "index_expression" => {
                expression
                    .child_by_field_name("operand")
                    .and_then(|collection| {
                        self.go_range_expression_type(owner, collection, depth + 1, visited)
                    })
            }
            "selector_expression" => {
                let operand = expression.child_by_field_name("operand")?;
                let field = expression.child_by_field_name("field")?;
                let receiver = self.go_expression_type(owner, operand, depth + 1, visited)?;
                self.go_member_types
                    .get(&(receiver, self.text(field)))
                    .cloned()
            }
            "call_expression" => {
                let function = expression.child_by_field_name("function")?;
                let qualified_callable = match function.kind() {
                    "identifier" => {
                        format!("{}.{}", self.module_or_package, self.text(function))
                    }
                    "selector_expression" => {
                        let operand = function.child_by_field_name("operand")?;
                        let field = function.child_by_field_name("field")?;
                        let receiver =
                            self.go_expression_type(owner, operand, depth + 1, visited)?;
                        format!("{receiver}::{}", self.text(field))
                    }
                    _ => return None,
                };
                self.go_return_types
                    .get(&qualified_callable)
                    .and_then(|types| go_output_type(types, output_index))
            }
            "type_assertion_expression" | "type_assertion" => expression
                .child_by_field_name("type")
                .and_then(go_direct_type_target)
                .and_then(|target| self.go_qualified_type_target(owner, target)),
            _ => None,
        }
    }

    fn go_range_expression_type(
        &self,
        owner: &DeclarationContext,
        expression: Node<'_>,
        depth: usize,
        visited: &mut HashSet<String>,
    ) -> Option<String> {
        self.go_range_expression_type_at_output(owner, expression, depth, visited, None)
    }

    fn go_range_expression_type_at_output(
        &self,
        owner: &DeclarationContext,
        expression: Node<'_>,
        depth: usize,
        visited: &mut HashSet<String>,
        output_index: Option<u32>,
    ) -> Option<String> {
        if depth >= GO_TYPE_INFERENCE_DEPTH_LIMIT {
            return None;
        }
        match expression.kind() {
            "identifier" => {
                let name = self.text(expression);
                if !visited.insert(name.clone()) {
                    return None;
                }
                let result = self
                    .local_target_for(owner, &name)
                    .and_then(|collection| self.go_collection_element_types.get(collection))
                    .cloned()
                    .or_else(|| {
                        self.local_value_for(
                            &self.go_collection_binding_element_types,
                            owner,
                            &name,
                        )
                        .cloned()
                    })
                    .or_else(|| {
                        go_local_initializer_with_index_before(expression, &name, self.source)
                            .and_then(|(initializer, output_index)| {
                                self.go_range_expression_type_at_output(
                                    owner,
                                    initializer,
                                    depth + 1,
                                    visited,
                                    output_index,
                                )
                            })
                    });
                visited.remove(&name);
                result
            }
            "call_expression" => {
                let function = expression.child_by_field_name("function")?;
                if function.kind() == "identifier" && self.text(function) == "make" {
                    let arguments = expression.child_by_field_name("arguments")?;
                    let collection_type = arguments.named_child(0)?;
                    return self.go_collection_element_type(owner, collection_type);
                }
                let qualified_callable = match function.kind() {
                    "identifier" => {
                        format!("{}.{}", self.module_or_package, self.text(function))
                    }
                    "selector_expression" => {
                        let operand = function.child_by_field_name("operand")?;
                        let field = function.child_by_field_name("field")?;
                        let receiver =
                            self.go_expression_type(owner, operand, depth + 1, visited)?;
                        format!("{receiver}::{}", self.text(field))
                    }
                    _ => return None,
                };
                self.go_range_return_types
                    .get(&qualified_callable)
                    .and_then(|types| go_output_type(types, output_index))
            }
            "selector_expression" => {
                let operand = expression.child_by_field_name("operand")?;
                let field = expression.child_by_field_name("field")?;
                let receiver = self.go_expression_type(owner, operand, depth + 1, visited)?;
                self.go_range_member_types
                    .get(&(receiver, self.text(field)))
                    .cloned()
            }
            "composite_literal" => expression
                .child_by_field_name("type")
                .and_then(go_range_value_type_target)
                .and_then(|target| self.go_qualified_type_target(owner, target)),
            "unary_expression" | "parenthesized_expression" => expression
                .child_by_field_name("operand")
                .or_else(|| expression.named_child(0))
                .and_then(|inner| self.go_range_expression_type(owner, inner, depth + 1, visited)),
            _ => None,
        }
    }

    fn python_base_qualified_name(
        &self,
        owner: &DeclarationContext,
        qualifier: Option<&str>,
        spelling: &str,
        use_start: usize,
    ) -> Option<String> {
        if spelling.is_empty() || is_python_builtin_type(spelling) {
            return None;
        }
        match qualifier {
            None => self
                .imported_target_for_occurrence(owner, spelling, use_start, false)
                .cloned()
                .or_else(|| Some(format!("{}.{}", self.module_or_package, spelling))),
            Some(qualifier) => self
                .imported_qualified_target_for(owner, qualifier, use_start, false)
                .map(|target| format!("{target}.{spelling}")),
        }
    }

    fn add_relationship_occurrence(
        &mut self,
        role: SemanticRole,
        relation: CandidateRelation,
        owner: &DeclarationContext,
        spelling: &str,
        qualifier: Option<&str>,
        node: Node<'_>,
    ) -> Result<(), EvidenceError> {
        self.add_relationship_occurrence_with_hierarchy(
            role, relation, owner, spelling, qualifier, node, None, None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn add_relationship_occurrence_with_hierarchy(
        &mut self,
        role: SemanticRole,
        relation: CandidateRelation,
        owner: &DeclarationContext,
        spelling: &str,
        qualifier: Option<&str>,
        node: Node<'_>,
        qualified_name_override: Option<String>,
        hierarchy: Option<HierarchyConstraint>,
    ) -> Result<(), EvidenceError> {
        if spelling.is_empty() {
            return Ok(());
        }
        let lookup_name = qualifier.map(qualified_binding_head).unwrap_or(spelling);
        let allow_later_file_binding = self.language != "python";
        if self.import_binding_declared_but_not_visible(
            owner,
            lookup_name,
            node.start_byte(),
            allow_later_file_binding,
        ) {
            return Ok(());
        }
        if self.import_binding_is_ambiguous(owner, lookup_name) {
            return Ok(());
        }
        let binding = self
            .binding_for_occurrence(
                owner,
                qualifier.map(qualified_binding_head).unwrap_or(spelling),
                node.start_byte(),
                allow_later_file_binding,
            )
            .or_else(|| {
                self.python_wildcard_binding(owner, node.start_byte(), allow_later_file_binding)
            })
            .cloned();
        let qualified_name = qualified_name_override
            .or_else(|| {
                qualifier.and_then(|qualifier| {
                    self.local_target_for(owner, qualifier)
                        .map(|target| format!("{target}::{spelling}"))
                })
            })
            .or_else(|| {
                qualifier
                    .and_then(|qualifier| {
                        self.imported_qualified_target_for(
                            owner,
                            qualifier,
                            node.start_byte(),
                            allow_later_file_binding,
                        )
                    })
                    .map(|target| format!("{target}.{spelling}"))
            })
            .or_else(|| {
                self.imported_target_for_occurrence(
                    owner,
                    spelling,
                    node.start_byte(),
                    allow_later_file_binding,
                )
                .cloned()
            });
        let occurrence_id = self.builder.occur(
            role,
            &owner.fact_id,
            spelling,
            qualifier,
            Some(&owner.scope_id),
            range_for_node(self.source_file, node),
        )?;
        self.builder.relate(
            relation,
            &owner.fact_id,
            Some(&occurrence_id),
            binding.as_deref(),
            spelling,
            ResolutionConstraint {
                exact_target_declaration_id: None,
                exact_language: Some(self.language.to_owned()),
                module_or_package: qualified_name
                    .as_deref()
                    .and_then(|qualified| {
                        qualified
                            .rsplit_once('.')
                            .map(|(module, _)| module.to_owned())
                    })
                    .or_else(|| Some(self.module_or_package.clone())),
                scope_id: Some(owner.scope_id.clone()),
                qualified_name: qualified_name.clone(),
                argument_count: None,
                argument_types: Vec::new(),
                allowed_target_kinds: target_kinds_for_relation(relation),
                hierarchy,
                allow_external: qualified_name.is_some(),
            },
        )?;
        Ok(())
    }

    fn go_name_is_locally_bound(&mut self, call: Node<'_>, spelling: &str) -> bool {
        let Some(callable) = go_enclosing_callable(call) else {
            return false;
        };
        let source = self.source;
        self.go_lexical_bindings
            .entry(callable.id())
            .or_insert_with(|| go_lexical_bindings(callable, source))
            .iter()
            .any(|binding| {
                binding.name == spelling
                    && binding.active_from <= call.start_byte()
                    && call.start_byte() < binding.active_until
            })
    }

    fn python_name_is_statically_local(&self, owner: &DeclarationContext, spelling: &str) -> bool {
        let mut scope_id = Some(owner.scope_id.as_str());
        for _ in 0..64 {
            let Some(current) = scope_id else {
                break;
            };
            if self
                .python_global_names
                .get(current)
                .is_some_and(|names| names.contains(spelling))
            {
                return false;
            }
            if self
                .python_local_bound_names
                .get(current)
                .is_some_and(|names| names.contains(spelling))
            {
                return true;
            }
            scope_id = self.scope_parents.get(current).map(String::as_str);
        }
        false
    }

    fn record_import_binding(
        &mut self,
        owner: &DeclarationContext,
        local: &str,
        target: &str,
        binding_id: &str,
        active_from: usize,
    ) {
        let scope_id = owner.scope_id.clone();
        let versions = self
            .import_bindings
            .entry(scope_id.clone())
            .or_default()
            .entry(local.to_owned())
            .or_default();
        let same_package = versions.iter().any(|version| {
            let existing_root = version
                .target
                .split_once('.')
                .map(|(root, _)| root)
                .unwrap_or(version.target.as_str());
            let new_root = target
                .split_once('.')
                .map(|(root, _)| root)
                .unwrap_or(target);
            existing_root == new_root
        });

        if (local == "*" && !versions.is_empty())
            || (self.language != "python" && !versions.is_empty())
            || (self.language == "python"
                && same_package
                && versions.iter().any(|version| version.target != target))
        {
            self.ambiguous_bindings
                .insert((scope_id.clone(), local.to_owned()));
        }
        versions.push(ImportBindingVersion {
            binding_id: binding_id.to_owned(),
            target: target.to_owned(),
            active_from,
        });
    }

    fn import_binding_scope<'a>(
        &'a self,
        owner: &'a DeclarationContext,
        name: &str,
    ) -> Option<&'a str> {
        let mut scope_id = Some(owner.scope_id.as_str());
        for _ in 0..64 {
            let current = scope_id?;
            if self
                .import_bindings
                .get(current)
                .is_some_and(|bindings| bindings.contains_key(name))
            {
                return Some(current);
            }
            scope_id = self.scope_parents.get(current).map(String::as_str);
        }
        None
    }

    fn rust_platform_reexport_bindings(
        &self,
        owner: &DeclarationContext,
        name: &str,
    ) -> Option<(Vec<String>, bool)> {
        let scope_id = self.import_binding_scope(owner, name)?;
        let versions = self.import_bindings.get(scope_id)?.get(name)?;
        if versions.len() != 2 {
            return None;
        }
        let mut platforms = BTreeSet::new();
        let mut targets = BTreeSet::new();
        let mut bindings = Vec::with_capacity(versions.len());
        for version in versions {
            platforms.insert(
                *self
                    .rust_platform_reexport_bindings
                    .get(&version.binding_id)?,
            );
            targets.insert(version.target.as_str());
            bindings.push(version.binding_id.clone());
        }
        if platforms != BTreeSet::from([RustPlatformCfg::Unix, RustPlatformCfg::Windows])
            || targets.len() != 2
        {
            return None;
        }
        let has_local_target = self
            .local_targets
            .get(scope_id)
            .is_some_and(|targets| targets.contains_key(name));
        let has_fallback = self
            .rust_platform_fallbacks
            .contains(&(scope_id.to_owned(), name.to_owned()));
        if has_local_target && !has_fallback {
            return None;
        }
        bindings.sort_unstable();
        Some((bindings, has_fallback))
    }

    fn import_binding_version_at(
        &self,
        owner: &DeclarationContext,
        name: &str,
        use_start: usize,
        allow_later_file_binding: bool,
    ) -> Option<&ImportBindingVersion> {
        let scope_id = self.import_binding_scope(owner, name)?;
        let versions = self.import_bindings.get(scope_id)?.get(name)?;
        if scope_id != owner.scope_id && allow_later_file_binding {
            return versions.last();
        }
        versions
            .iter()
            .rev()
            .find(|binding| binding.active_from <= use_start)
    }

    fn import_binding_declared_but_not_visible(
        &self,
        owner: &DeclarationContext,
        name: &str,
        use_start: usize,
        allow_later_file_binding: bool,
    ) -> bool {
        self.import_binding_scope(owner, name).is_some()
            && self
                .import_binding_version_at(owner, name, use_start, allow_later_file_binding)
                .is_none()
    }

    fn binding_for_occurrence(
        &self,
        owner: &DeclarationContext,
        name: &str,
        use_start: usize,
        allow_later_file_binding: bool,
    ) -> Option<&String> {
        self.local_binding_for(owner, name).or_else(|| {
            self.import_binding_version_at(owner, name, use_start, allow_later_file_binding)
                .map(|binding| &binding.binding_id)
        })
    }

    fn python_wildcard_binding(
        &self,
        owner: &DeclarationContext,
        use_start: usize,
        allow_later_file_binding: bool,
    ) -> Option<&String> {
        if self.language != "python" || self.import_binding_is_ambiguous(owner, "*") {
            return None;
        }
        self.import_binding_version_at(owner, "*", use_start, allow_later_file_binding)
            .map(|binding| &binding.binding_id)
    }

    fn imported_target_for_occurrence(
        &self,
        owner: &DeclarationContext,
        name: &str,
        use_start: usize,
        allow_later_file_binding: bool,
    ) -> Option<&String> {
        self.import_binding_version_at(owner, name, use_start, allow_later_file_binding)
            .map(|binding| &binding.target)
    }

    fn imported_qualified_target_for(
        &self,
        owner: &DeclarationContext,
        qualifier: &str,
        use_start: usize,
        allow_later_file_binding: bool,
    ) -> Option<String> {
        let (head, suffix) = split_qualified_head(qualifier);
        let separator = if qualifier.contains("::") { "::" } else { "." };
        self.imported_target_for_occurrence(owner, head, use_start, allow_later_file_binding)
            .map(|target| {
                suffix.map_or_else(
                    || target.clone(),
                    |suffix| format!("{target}{separator}{suffix}"),
                )
            })
    }

    fn import_binding_is_ambiguous(&self, owner: &DeclarationContext, name: &str) -> bool {
        if self.local_binding_for(owner, name).is_some() {
            return false;
        }
        self.visible_import_binding_is_ambiguous(owner, name)
    }

    fn visible_import_binding_is_ambiguous(&self, owner: &DeclarationContext, name: &str) -> bool {
        let mut scope_id = Some(owner.scope_id.as_str());
        for _ in 0..64 {
            let Some(current) = scope_id else {
                break;
            };
            if self
                .import_bindings
                .get(current)
                .is_some_and(|bindings| bindings.contains_key(name))
            {
                return self
                    .ambiguous_bindings
                    .contains(&(current.to_owned(), name.to_owned()));
            }
            scope_id = self.scope_parents.get(current).map(String::as_str);
        }
        false
    }

    fn local_target_for(&self, owner: &DeclarationContext, name: &str) -> Option<&String> {
        self.local_value_for(&self.local_targets, owner, name)
    }

    fn local_binding_for(&self, owner: &DeclarationContext, name: &str) -> Option<&String> {
        let mut scope_id = Some(owner.scope_id.as_str());
        for _ in 0..64 {
            let current = scope_id?;
            if self
                .ambiguous_bindings
                .contains(&(current.to_owned(), name.to_owned()))
            {
                return None;
            }
            if let Some(value) = self
                .local_bindings
                .get(current)
                .and_then(|scope| scope.get(name))
            {
                return Some(value);
            }
            scope_id = self.scope_parents.get(current).map(String::as_str);
        }
        None
    }

    fn local_value_for<'a>(
        &'a self,
        values: &'a HashMap<String, HashMap<String, String>>,
        owner: &DeclarationContext,
        name: &str,
    ) -> Option<&'a String> {
        let mut scope_id = Some(owner.scope_id.as_str());
        for _ in 0..64 {
            let current = scope_id?;
            if let Some(value) = values.get(current).and_then(|scope| scope.get(name)) {
                return Some(value);
            }
            if self
                .local_shadows
                .get(current)
                .is_some_and(|shadows| shadows.contains(name))
            {
                return None;
            }
            scope_id = self.scope_parents.get(current).map(String::as_str);
        }
        None
    }

    fn add_ownership(
        &mut self,
        owner: &DeclarationContext,
        child: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        self.builder.relate(
            CandidateRelation::Contains,
            &owner.fact_id,
            None,
            None,
            &child.name,
            ResolutionConstraint {
                exact_target_declaration_id: Some(child.fact_id.clone()),
                exact_language: Some(self.language.to_owned()),
                module_or_package: Some(self.module_or_package.clone()),
                scope_id: Some(owner.scope_id.clone()),
                qualified_name: Some(child.qualified_name.clone()),
                argument_count: None,
                argument_types: Vec::new(),
                allowed_target_kinds: vec![child.kind.clone()],
                hierarchy: None,
                allow_external: false,
            },
        )?;
        Ok(())
    }

    fn unique_graph_id(&mut self, base: String, node: Node<'_>) -> String {
        if self.graph_ids.insert(base.clone()) {
            return base;
        }
        let unique = make_id(&[
            &base,
            "overload",
            &(node.start_position().row + 1).to_string(),
        ]);
        self.graph_ids.insert(unique.clone());
        unique
    }

    fn text(&self, node: Node<'_>) -> String {
        node.utf8_text(self.source)
            .unwrap_or_default()
            .trim()
            .to_owned()
    }
}

fn java_package_identity(source: &[u8]) -> Option<String> {
    let source = std::str::from_utf8(source).ok()?;
    let mut in_block_comment = false;
    for line in source.lines() {
        let mut line = line.trim();
        if in_block_comment {
            if let Some((_, rest)) = line.split_once("*/") {
                in_block_comment = false;
                line = rest.trim();
            } else {
                continue;
            }
        }
        while let Some(rest) = line.strip_prefix("/*") {
            if let Some((_, suffix)) = rest.split_once("*/") {
                line = suffix.trim();
            } else {
                in_block_comment = true;
                break;
            }
        }
        if in_block_comment || line.starts_with("//") || line.is_empty() {
            continue;
        }
        let Some(package) = line.strip_prefix("package ") else {
            continue;
        };
        let package = package
            .split(';')
            .next()
            .unwrap_or_default()
            .split_whitespace()
            .next()
            .unwrap_or_default();
        if !package.is_empty()
            && package
                .split('.')
                .all(|part| !part.is_empty() && java_identifier(part))
        {
            return Some(package.to_owned());
        }
    }
    None
}

fn java_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|character| character == '_' || character == '$' || character.is_alphabetic())
        && chars
            .all(|character| character == '_' || character == '$' || character.is_alphanumeric())
}

fn java_container_kind(kind: &str) -> Option<&'static str> {
    match kind {
        "class_declaration" => Some("class"),
        "interface_declaration" => Some("interface"),
        "enum_declaration" => Some("enum"),
        "record_declaration" => Some("record"),
        "annotation_type_declaration" => Some("annotation_type"),
        _ => None,
    }
}

fn java_child_qualified_name(owner: &DeclarationContext, name: &str) -> String {
    if owner.kind == "file" {
        match owner.qualified_name.as_str() {
            "" => name.to_owned(),
            package => format!("{package}.{name}"),
        }
    } else {
        format!("{}::{name}", owner.qualified_name)
    }
}

fn java_qualified_parent(target: &str) -> Option<&str> {
    target
        .trim_end_matches(".*")
        .rsplit_once('.')
        .map(|(parent, _)| parent)
}

fn go_parameter_signature(node: Node<'_>) -> (u32, bool) {
    let Some(parameters) = node.child_by_field_name("parameters") else {
        return (0, false);
    };
    let mut count = 0_u32;
    let mut variadic = false;
    let mut cursor = parameters.walk();
    for parameter in parameters
        .children(&mut cursor)
        .filter(|child| child.is_named())
    {
        let is_variadic = parameter.kind() == "variadic_parameter_declaration";
        if !matches!(
            parameter.kind(),
            "parameter_declaration" | "variadic_parameter_declaration"
        ) {
            continue;
        }
        let mut parameter_cursor = parameter.walk();
        let names = parameter
            .children_by_field_name("name", &mut parameter_cursor)
            .count();
        let slots = names.max(1);
        count = count.saturating_add(u32::try_from(slots).unwrap_or(u32::MAX));
        variadic |= is_variadic;
    }
    (count, variadic)
}

fn last_java_import_name(node: Node<'_>) -> Option<Node<'_>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .filter(|child| child.is_named())
        .last()
}

fn java_parameter_signature(node: Node<'_>, source: &[u8]) -> (String, u32, bool, Vec<String>) {
    let Some(parameters) = node.child_by_field_name("parameters") else {
        return (String::new(), 0, false, Vec::new());
    };
    let mut types = Vec::new();
    let mut canonical_inputs = Vec::new();
    let mut variadic = false;
    let mut cursor = parameters.walk();
    for parameter in parameters
        .children(&mut cursor)
        .filter(|child| child.is_named())
    {
        if !matches!(
            parameter.kind(),
            "formal_parameter" | "spread_parameter" | "receiver_parameter"
        ) {
            continue;
        }
        variadic |= parameter.kind() == "spread_parameter";
        let Some(type_node) = parameter.child_by_field_name("type") else {
            continue;
        };
        let raw = type_node.utf8_text(source).unwrap_or_default();
        let mut normalized = java_normalize_type(raw);
        canonical_inputs.push(normalized.clone());
        if parameter.kind() == "spread_parameter" {
            normalized.push_str("...");
        }
        if let Some(dimensions) = parameter.child_by_field_name("dimensions") {
            normalized.push_str(
                &dimensions
                    .utf8_text(source)
                    .unwrap_or_default()
                    .chars()
                    .filter(|character| !character.is_whitespace())
                    .collect::<String>(),
            );
        }
        types.push(normalized);
    }
    (
        types.join(","),
        u32::try_from(types.len()).unwrap_or(u32::MAX),
        variadic,
        canonical_inputs,
    )
}

fn java_normalize_type(raw: &str) -> String {
    let mut normalized = String::with_capacity(raw.len());
    let mut angle_depth = 0_u32;
    for character in raw.chars() {
        match character {
            '<' => angle_depth = angle_depth.saturating_add(1),
            '>' => angle_depth = angle_depth.saturating_sub(1),
            _ if angle_depth > 0 => {}
            character if character.is_whitespace() => {}
            '[' | ']' | '.' | '$' | '_' => normalized.push(character),
            character if character.is_alphanumeric() => normalized.push(character),
            _ => {}
        }
    }
    normalized.trim_end_matches("...").to_owned()
}

fn collect_direct_or_nested_nodes<'tree>(
    node: Node<'tree>,
    kind: &str,
    output: &mut Vec<Node<'tree>>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        if child.kind() == kind {
            output.push(child);
        } else if !matches!(
            child.kind(),
            "class_declaration"
                | "interface_declaration"
                | "enum_declaration"
                | "record_declaration"
                | "annotation_type_declaration"
                | "method_declaration"
                | "constructor_declaration"
        ) {
            collect_direct_or_nested_nodes(child, kind, output);
        }
    }
}

fn collect_java_annotations<'tree>(node: Node<'tree>, output: &mut Vec<Node<'tree>>) {
    if matches!(node.kind(), "annotation" | "marker_annotation") {
        output.push(node);
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_java_annotations(child, output);
    }
}

fn first_java_type_name(node: Node<'_>) -> Option<Node<'_>> {
    if matches!(
        node.kind(),
        "type_identifier" | "scoped_type_identifier" | "identifier"
    ) {
        return Some(node);
    }
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .filter(|child| child.is_named())
        .find_map(first_java_type_name)
}

fn collect_java_type_nodes<'tree>(node: Node<'tree>, output: &mut Vec<Node<'tree>>) {
    if matches!(node.kind(), "type_identifier" | "scoped_type_identifier") {
        output.push(node);
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_java_type_nodes(child, output);
    }
}

fn collect_java_direct_supertype_nodes<'tree>(node: Node<'tree>, output: &mut Vec<Node<'tree>>) {
    if matches!(node.kind(), "type_identifier" | "scoped_type_identifier") {
        output.push(node);
        return;
    }
    if node.kind() == "generic_type" {
        if let Some(target) = node
            .child_by_field_name("type")
            .or_else(|| first_java_type_name(node))
        {
            output.push(target);
        }
        return;
    }
    if matches!(
        node.kind(),
        "type_arguments" | "annotation" | "marker_annotation"
    ) {
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_java_direct_supertype_nodes(child, output);
    }
}

fn collect_java_parameter_type_nodes<'tree>(
    parameters: Node<'tree>,
    output: &mut Vec<Node<'tree>>,
) {
    let mut cursor = parameters.walk();
    for parameter in parameters
        .children(&mut cursor)
        .filter(|child| child.is_named())
    {
        if matches!(parameter.kind(), "formal_parameter" | "spread_parameter")
            && let Some(type_node) = parameter.child_by_field_name("type")
        {
            collect_java_type_nodes(type_node, output);
        }
    }
}

fn java_argument_count(node: Node<'_>) -> u32 {
    let Some(arguments) = node.child_by_field_name("arguments") else {
        return 0;
    };
    let mut cursor = arguments.walk();
    u32::try_from(
        arguments
            .children(&mut cursor)
            .filter(|child| child.is_named())
            .count(),
    )
    .unwrap_or(u32::MAX)
}

fn python_call_argument_shape(call: Node<'_>) -> (Option<u32>, Vec<Option<String>>) {
    let Some(arguments) = call.child_by_field_name("arguments") else {
        return (Some(0), Vec::new());
    };
    let mut cursor = arguments.walk();
    let arguments = arguments
        .children(&mut cursor)
        .filter(|child| child.is_named())
        .collect::<Vec<_>>();
    if arguments
        .iter()
        .any(|argument| matches!(argument.kind(), "list_splat" | "dictionary_splat"))
    {
        return (None, Vec::new());
    }
    let Some(argument_count) = u32::try_from(arguments.len()).ok() else {
        return (None, Vec::new());
    };
    let has_keywords = arguments
        .iter()
        .any(|argument| argument.kind() == "keyword_argument");
    let argument_types = if has_keywords {
        vec![None; arguments.len()]
    } else {
        arguments
            .into_iter()
            .map(python_literal_type)
            .collect::<Vec<_>>()
    };
    (Some(argument_count), argument_types)
}

fn python_literal_type(expression: Node<'_>) -> Option<String> {
    let qualified = match expression.kind() {
        "string" | "concatenated_string" => "builtins.str",
        "integer" => "builtins.int",
        "float" => "builtins.float",
        "true" | "false" => "builtins.bool",
        "none" => "builtins.NoneType",
        "list" | "list_comprehension" => "builtins.list",
        "dictionary" | "dictionary_comprehension" => "builtins.dict",
        "set" | "set_comprehension" => "builtins.set",
        "tuple" => "builtins.tuple",
        _ => return None,
    };
    Some(qualified.to_owned())
}

fn java_primitive_type(name: &str) -> bool {
    matches!(
        name,
        "byte"
            | "short"
            | "int"
            | "long"
            | "float"
            | "double"
            | "boolean"
            | "char"
            | "void"
            | "var"
    )
}

fn java_lang_type(name: &str) -> Option<String> {
    crate::builtins::is_language_builtin_global("java", name).then(|| format!("java.lang.{name}"))
}

fn rust_container_kind(kind: &str) -> Option<&'static str> {
    match kind {
        "mod_item" => Some("module"),
        "trait_item" => Some("trait"),
        "struct_item" => Some("struct"),
        "enum_item" => Some("enum"),
        _ => None,
    }
}

fn rust_join_qualified(owner: &str, name: &str) -> String {
    match (owner.trim_matches(':'), name.trim_matches(':')) {
        ("", name) => name.to_owned(),
        (owner, "") => owner.to_owned(),
        (owner, name) => format!("{owner}::{name}"),
    }
}

fn rust_path_leaf(path: &str) -> &str {
    path.trim()
        .trim_end_matches("::*")
        .trim_end_matches("::")
        .rsplit("::")
        .next()
        .unwrap_or_default()
}

fn rust_strip_generic_arguments(raw: &str) -> String {
    let raw = raw.trim();
    let mut normalized = String::with_capacity(raw.len());
    let mut angle_depth = 0_u32;
    for character in raw.chars() {
        match character {
            '<' => {
                if angle_depth == 0 && normalized.ends_with("::") {
                    normalized.truncate(normalized.len().saturating_sub(2));
                }
                angle_depth = angle_depth.saturating_add(1);
            }
            '>' if angle_depth > 0 => angle_depth = angle_depth.saturating_sub(1),
            _ if angle_depth > 0 => {}
            character => normalized.push(character),
        }
    }
    normalized.trim().trim_end_matches("::").to_owned()
}

fn rust_normalize_path(raw: &str) -> String {
    let raw = raw.trim();
    if let Some(inner) = raw
        .strip_prefix('<')
        .and_then(|value| value.strip_suffix('>'))
        && let Some((type_path, trait_path)) = inner.split_once(" as ")
    {
        return format!(
            "<{} as {}>",
            rust_strip_generic_arguments(type_path),
            rust_strip_generic_arguments(trait_path)
        );
    }
    rust_strip_generic_arguments(raw)
}

fn rust_nominal_type_path(raw: &str) -> Option<String> {
    let mut raw = raw.trim();
    loop {
        if let Some(suffix) = raw.strip_prefix('&') {
            raw = suffix.trim_start();
            if let Some(lifetime) = raw.strip_prefix('\'') {
                let end = lifetime.find(char::is_whitespace).unwrap_or(lifetime.len());
                raw = lifetime[end..].trim_start();
            }
            continue;
        }
        if let Some(suffix) = raw
            .strip_prefix("mut ")
            .or_else(|| raw.strip_prefix("*mut "))
            .or_else(|| raw.strip_prefix("*const "))
        {
            raw = suffix.trim_start();
            continue;
        }
        break;
    }
    if raw.is_empty() || raw.starts_with(['<', '[', '(']) {
        return None;
    }
    let nominal = rust_normalize_path(raw);
    (!nominal.is_empty()
        && nominal
            .chars()
            .all(|character| character.is_alphanumeric() || matches!(character, '_' | ':')))
    .then_some(nominal)
}

fn rust_return_receiver_type_path(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.starts_with("*mut ") || raw.starts_with("*const ") {
        return None;
    }
    rust_nominal_type_path(raw)
}

fn rust_outer_nominal_return_node(mut node: Node<'_>) -> Option<Node<'_>> {
    for _ in 0..16 {
        match node.kind() {
            "type_identifier" | "scoped_type_identifier" => return Some(node),
            "generic_type" | "reference_type" | "parenthesized_type" => {
                node = node.child_by_field_name("type")?;
            }
            _ => return None,
        }
    }
    None
}

fn rust_trait_bound_path(raw: &str) -> Option<String> {
    let raw = raw.trim().trim_start_matches('?').trim();
    let raw = raw
        .strip_prefix("for<")
        .and_then(|value| value.split_once('>').map(|(_, remainder)| remainder))
        .map_or(raw, str::trim);
    rust_nominal_type_path(raw)
}

fn rust_qualified_parent(path: &str) -> Option<&str> {
    path.rsplit_once("::").map(|(parent, _)| parent)
}

const RUST_MANIFEST_MAX_BYTES: u64 = 2 * 1024 * 1024;
const RUST_MANIFEST_ANCESTOR_LIMIT: usize = 64;

/// Read one local Cargo manifest without following an unbounded dependency
/// graph.  Manifest metadata is optional enrichment: malformed, oversized, or
/// symlinked files simply leave the ordinary source-derived namespace intact.
fn read_rust_manifest(path: &Path) -> Option<toml::Value> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > RUST_MANIFEST_MAX_BYTES
    {
        return None;
    }
    let source = fs::read_to_string(path).ok()?;
    toml::from_str(&source).ok()
}

fn rust_package_manifest(path: &Path) -> Option<(PathBuf, toml::Value)> {
    // A virtual source path must not accidentally resolve against the
    // extractor process's own Cargo workspace.  An absolute path is enough to
    // anchor manifest discovery; the caller may legitimately provide source
    // bytes before creating the corresponding file.
    if !path.is_absolute() {
        return None;
    }
    let mut directory = path.parent();
    for _ in 0..RUST_MANIFEST_ANCESTOR_LIMIT {
        let Some(current) = directory else {
            break;
        };
        let manifest = current.join("Cargo.toml");
        if let Some(value) = read_rust_manifest(&manifest)
            && value
                .get("package")
                .and_then(toml::Value::as_table)
                .is_some()
        {
            return Some((manifest, value));
        }
        directory = current.parent();
    }
    None
}

fn rust_workspace_manifest(package_manifest: &Path) -> Option<(PathBuf, toml::Value)> {
    let mut directory = package_manifest.parent();
    for _ in 0..RUST_MANIFEST_ANCESTOR_LIMIT {
        let Some(current) = directory else {
            break;
        };
        let manifest = current.join("Cargo.toml");
        if let Some(value) = read_rust_manifest(&manifest)
            && value
                .get("workspace")
                .and_then(toml::Value::as_table)
                .is_some()
        {
            return Some((manifest, value));
        }
        directory = current.parent();
    }
    None
}

fn rust_dependency_manifest(manifest_dir: &Path, dependency_path: &str) -> Option<PathBuf> {
    let path = Path::new(dependency_path);
    if path.is_absolute() || dependency_path.is_empty() {
        return None;
    }
    let candidate = manifest_dir.join(path);
    if candidate
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("Cargo.toml"))
    {
        Some(candidate)
    } else {
        Some(candidate.join("Cargo.toml"))
    }
}

fn rust_workspace_dependency<'a>(
    workspace: Option<&'a toml::Value>,
    alias: &str,
) -> Option<&'a toml::Value> {
    let dependencies = workspace?
        .get("workspace")
        .and_then(toml::Value::as_table)?
        .get("dependencies")
        .and_then(toml::Value::as_table)?;
    dependencies.get(alias).or_else(|| {
        dependencies.iter().find_map(|(name, value)| {
            (name.replace('-', "_") == alias.replace('-', "_")).then_some(value)
        })
    })
}

fn rust_dependency_target(
    alias: &str,
    specification: &toml::Value,
    manifest_dir: &Path,
    workspace: Option<&(PathBuf, toml::Value)>,
) -> String {
    let table = specification.as_table();
    let inherited = table
        .and_then(|table| table.get("workspace"))
        .and_then(toml::Value::as_bool)
        .unwrap_or(false);
    let workspace_specification = inherited
        .then(|| rust_workspace_dependency(workspace.map(|(_, value)| value), alias))
        .flatten();
    let target = workspace_specification
        .and_then(|value| value.as_table())
        .and_then(|table| table.get("package"))
        .and_then(toml::Value::as_str)
        .or_else(|| {
            table
                .and_then(|table| table.get("package"))
                .and_then(toml::Value::as_str)
        })
        .unwrap_or(alias);
    let dependency_path = workspace_specification
        .and_then(|value| value.as_table())
        .and_then(|table| table.get("path"))
        .and_then(toml::Value::as_str)
        .or_else(|| {
            table
                .and_then(|table| table.get("path"))
                .and_then(toml::Value::as_str)
        });
    if let Some(dependency_path) = dependency_path
        && let Some(manifest) = rust_dependency_manifest(
            workspace
                .map(|(path, _)| path.parent().unwrap_or(manifest_dir))
                .unwrap_or(manifest_dir),
            dependency_path,
        )
        && let Some(value) = read_rust_manifest(&manifest)
        && let Some(name) = rust_manifest_crate_name_value(&value)
    {
        return name;
    }
    target.replace('-', "_")
}

fn rust_module_identity(path: &Path, source_file: &str) -> String {
    let portable = Path::new(source_file);
    let portable_components = portable
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .collect::<Vec<_>>();
    let source_root = portable_components
        .iter()
        .rposition(|component| *component == "src");
    let workspace_crate_root = (source_root.is_none()
        && portable_components.first().copied() == Some("crates")
        && portable_components.len() > 2)
        .then_some(2_usize);
    let fallback_crate_name = source_root
        .and_then(|index| index.checked_sub(1))
        .and_then(|index| portable_components.get(index).copied())
        .filter(|component| !matches!(*component, "crates" | "src"))
        .or_else(|| {
            workspace_crate_root.and_then(|index| portable_components.get(index - 1).copied())
        })
        .unwrap_or("crate");
    let manifest_context = rust_package_manifest(path);
    let crate_name = manifest_context
        .as_ref()
        .and_then(|(_, value)| rust_manifest_crate_name_value(value))
        .unwrap_or_else(|| fallback_crate_name.replace('-', "_"));
    let manifest_relative = manifest_context.as_ref().and_then(|(manifest, value)| {
        let library_path = value
            .get("lib")
            .and_then(toml::Value::as_table)
            .and_then(|lib| lib.get("path"))
            .and_then(toml::Value::as_str)
            .unwrap_or("src/lib.rs");
        let library = manifest.parent()?.join(library_path);
        let module_root = library.parent()?;
        path.strip_prefix(module_root).ok().map(|relative| {
            if path == library {
                return Vec::new();
            }
            relative
                .components()
                .filter_map(|component| component.as_os_str().to_str())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
    });
    let relative = if let Some(relative) = manifest_relative {
        relative
    } else if let Some(index) = source_root {
        portable_components[index.saturating_add(1)..]
            .iter()
            .map(|component| (*component).to_owned())
            .collect()
    } else if let Some(index) = workspace_crate_root {
        portable_components[index..]
            .iter()
            .map(|component| (*component).to_owned())
            .collect()
    } else {
        path.file_name()
            .and_then(|value| value.to_str())
            .map_or_else(Vec::new, |value| vec![value.to_owned()])
    };
    let mut components = relative;
    if let Some(file) = components.pop() {
        let stem = Path::new(&file)
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if !matches!(stem, "lib" | "main" | "mod") && !stem.is_empty() {
            components.push(stem.to_owned());
        }
    }
    if components.is_empty() {
        crate_name.to_owned()
    } else {
        format!("{crate_name}::{}", components.join("::"))
    }
}

fn rust_manifest_crate_name_value(value: &toml::Value) -> Option<String> {
    let package_name = value
        .get("package")
        .and_then(toml::Value::as_table)
        .and_then(|package| package.get("name"))
        .and_then(toml::Value::as_str)?;
    let lib_name = value
        .get("lib")
        .and_then(toml::Value::as_table)
        .and_then(|lib| lib.get("name"))
        .and_then(toml::Value::as_str);
    Some(
        lib_name
            .unwrap_or(package_name)
            .replace('-', "_")
            .to_owned(),
    )
}

fn rust_manifest_dependency_aliases(path: &Path) -> HashMap<String, String> {
    let Some((manifest, value)) = rust_package_manifest(path) else {
        return HashMap::new();
    };
    let workspace = rust_workspace_manifest(&manifest);
    let manifest_dir = manifest.parent().unwrap_or_else(|| Path::new("."));
    let mut aliases = HashMap::new();
    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        let Some(table) = value.get(section).and_then(toml::Value::as_table) else {
            continue;
        };
        for (alias, specification) in table {
            aliases.insert(
                alias.replace('-', "_"),
                rust_dependency_target(alias, specification, manifest_dir, workspace.as_ref()),
            );
        }
    }
    aliases
}

fn rust_qualify_local_path(module: &str, raw: &str) -> String {
    let raw = rust_normalize_path(raw.trim().trim_start_matches(['&', '*']));
    if raw.is_empty() {
        return module.to_owned();
    }
    if raw == "Self" {
        return module.to_owned();
    }
    rust_canonical_import_target(module, &raw)
}

fn rust_qualify_imported_path(
    state: &DirectEvidenceState<'_>,
    owner: &DeclarationContext,
    raw: &str,
    use_start: usize,
) -> String {
    let raw = rust_normalize_path(raw);
    let (qualifier, spelling) = split_qualified(&raw);
    if let Some(target) = state.imported_target_for_occurrence(
        owner,
        qualifier.map(qualified_binding_head).unwrap_or(spelling),
        use_start,
        true,
    ) {
        if qualifier.is_some() {
            return rust_join_qualified(target, spelling);
        }
        return target.clone();
    }
    state.rust_canonical_import_target(&state.rust_enclosing_module(owner), &raw)
}

fn rust_qualify_evidence_path(
    state: &DirectEvidenceState<'_>,
    owner: &DeclarationContext,
    raw: &str,
    use_start: usize,
) -> Option<String> {
    let raw = rust_normalize_path(raw.trim().trim_start_matches(['&', '*']));
    if raw.is_empty() || rust_primitive_type(&raw) {
        return None;
    }
    let (qualifier, spelling) = split_qualified(&raw);
    if qualifier == Some("Self") {
        return state
            .rust_associated_type_for(owner, spelling)
            .map(|associated_type| associated_type.qualified_name.clone());
    }
    let binding_name = qualifier.map(qualified_binding_head).unwrap_or(spelling);
    if let Some(target) = state.imported_target_for_occurrence(owner, binding_name, use_start, true)
    {
        return Some(if qualifier.is_some() {
            rust_join_qualified(target, spelling)
        } else {
            target.clone()
        });
    }
    if let Some(target) = state.local_target_for(owner, binding_name) {
        return Some(if qualifier.is_some() {
            rust_join_qualified(target, spelling)
        } else {
            target.clone()
        });
    }
    if qualifier.is_none() {
        let prelude_type = match binding_name {
            "Option" => Some("std::option::Option"),
            "Result" => Some("std::result::Result"),
            _ => None,
        };
        if let Some(prelude_type) = prelude_type {
            return Some(prelude_type.to_owned());
        }
    }
    if matches!(binding_name, "crate" | "self" | "super") {
        return Some(state.rust_canonical_import_target(&state.rust_enclosing_module(owner), &raw));
    }
    if qualifier.is_some_and(|value| {
        value.contains("::") || value.chars().next().is_some_and(char::is_lowercase)
    }) {
        let target = state.rust_canonical_import_target(&state.rust_enclosing_module(owner), &raw);
        return Some(target);
    }
    Some(rust_join_qualified(
        &state.rust_enclosing_module(owner),
        &raw,
    ))
}

fn rust_canonical_import_target(module: &str, raw: &str) -> String {
    let raw = raw.trim().trim_start_matches("::").trim_end_matches("::*");
    if raw.is_empty() {
        return String::new();
    }
    let crate_name = module.split("::").next().unwrap_or("crate");
    if raw == "crate" {
        return crate_name.to_owned();
    }
    if let Some(suffix) = raw.strip_prefix("crate::") {
        return rust_join_qualified(crate_name, suffix);
    }
    if raw == "self" {
        return module.to_owned();
    }
    if let Some(suffix) = raw.strip_prefix("self::") {
        return rust_join_qualified(module, suffix);
    }
    if raw == "super" || raw.starts_with("super::") {
        let mut base = module;
        let mut remainder = raw;
        while let Some(suffix) = remainder.strip_prefix("super") {
            base = rust_qualified_parent(base).unwrap_or("crate");
            remainder = suffix.strip_prefix("::").unwrap_or(suffix);
            if !remainder.starts_with("super") {
                break;
            }
        }
        return if remainder.is_empty() {
            base.to_owned()
        } else {
            rust_join_qualified(base, remainder)
        };
    }
    raw.to_owned()
}

fn expand_rust_use_tree(raw: &str, prefix: &str, output: &mut Vec<(String, Option<String>, bool)>) {
    let raw = raw.trim().trim_matches(',');
    if raw.is_empty() {
        return;
    }
    if let Some(open) = top_level_byte(raw, b'{')
        && let Some(close) = matching_brace(raw, open)
    {
        let base = raw[..open].trim().trim_end_matches("::");
        let next_prefix = rust_use_join(prefix, base);
        for item in split_top_level_rust_items(&raw[open + 1..close]) {
            expand_rust_use_tree(item, &next_prefix, output);
        }
        return;
    }
    if raw == "self" && !prefix.is_empty() {
        output.push((prefix.to_owned(), None, false));
        return;
    }
    if raw == "*" {
        output.push((prefix.to_owned(), None, true));
        return;
    }
    if let Some(base) = raw.strip_suffix("::*") {
        output.push((rust_use_join(prefix, base), None, true));
        return;
    }
    if let Some(index) = top_level_as(raw) {
        let target = rust_use_join(prefix, raw[..index].trim());
        let alias = raw[index + 4..].trim();
        if !target.is_empty() && !alias.is_empty() {
            output.push((target, Some(alias.to_owned()), false));
        }
        return;
    }
    output.push((rust_use_join(prefix, raw), None, false));
}

fn rust_use_join(prefix: &str, suffix: &str) -> String {
    let prefix = prefix.trim().trim_end_matches("::");
    let suffix = suffix.trim().trim_start_matches("::");
    if prefix.is_empty() {
        suffix.to_owned()
    } else if suffix.is_empty() {
        prefix.to_owned()
    } else {
        format!("{prefix}::{suffix}")
    }
}

fn top_level_byte(raw: &str, target: u8) -> Option<usize> {
    let mut depth = 0usize;
    for (index, byte) in raw.bytes().enumerate() {
        match byte {
            b'{' | b'(' | b'[' if byte != target => depth = depth.saturating_add(1),
            b'}' | b')' | b']' => depth = depth.saturating_sub(1),
            _ if byte == target && depth == 0 => return Some(index),
            _ => {}
        }
    }
    None
}

fn matching_brace(raw: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (offset, byte) in raw.as_bytes()[open..].iter().copied().enumerate() {
        match byte {
            b'{' => depth = depth.saturating_add(1),
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(open + offset);
                }
            }
            _ => {}
        }
    }
    None
}

fn split_top_level_rust_items(raw: &str) -> Vec<&str> {
    let mut output = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (index, byte) in raw.bytes().enumerate() {
        match byte {
            b'{' | b'(' | b'[' => depth = depth.saturating_add(1),
            b'}' | b')' | b']' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                output.push(&raw[start..index]);
                start = index.saturating_add(1);
            }
            _ => {}
        }
    }
    output.push(&raw[start..]);
    output
}

fn top_level_as(raw: &str) -> Option<usize> {
    let bytes = raw.as_bytes();
    let mut depth = 0usize;
    let mut index = 0usize;
    while index + 4 <= bytes.len() {
        match bytes[index] {
            b'{' | b'(' | b'[' => depth = depth.saturating_add(1),
            b'}' | b')' | b']' => depth = depth.saturating_sub(1),
            b' ' if depth == 0 && &bytes[index..index + 4] == b" as " => {
                return Some(index);
            }
            _ => {}
        }
        index = index.saturating_add(1);
    }
    None
}

fn rust_use_binding_range(
    source_file: &str,
    source: &[u8],
    argument: Node<'_>,
    local: &str,
    raw_target: &str,
) -> EvidenceRange {
    let bytes = &source[argument.start_byte()..argument.end_byte()];
    let needle = if local == "*" {
        "*"
    } else if !local.is_empty() {
        local
    } else {
        rust_path_leaf(raw_target)
    };
    let offset = std::str::from_utf8(bytes)
        .ok()
        .and_then(|text| text.rfind(needle))
        .unwrap_or_default();
    let start = argument.start_byte().saturating_add(offset);
    range_for_byte_span(
        source_file,
        source,
        start,
        start.saturating_add(needle.len()),
    )
}

pub(super) fn range_for_byte_span(
    source_file: &str,
    source: &[u8],
    start_byte: usize,
    end_byte: usize,
) -> EvidenceRange {
    let (start_line, start_column) = source_position(source, start_byte);
    let (end_line, end_column) = source_position(source, end_byte);
    EvidenceRange {
        source_file: source_file.to_owned(),
        start_byte: u64::try_from(start_byte).unwrap_or(u64::MAX),
        end_byte: u64::try_from(end_byte).unwrap_or(u64::MAX),
        start_line,
        start_column,
        end_line,
        end_column,
    }
}

fn source_position(source: &[u8], byte: usize) -> (u32, u32) {
    let byte = byte.min(source.len());
    let prefix = &source[..byte];
    let line = prefix.iter().filter(|value| **value == b'\n').count();
    let column = prefix
        .iter()
        .rposition(|value| *value == b'\n')
        .map_or(byte, |position| {
            byte.saturating_sub(position.saturating_add(1))
        });
    (
        u32::try_from(line.saturating_add(1)).unwrap_or(u32::MAX),
        u32::try_from(column).unwrap_or(u32::MAX),
    )
}

fn collect_rust_type_nodes<'tree>(
    node: Node<'tree>,
    body_start: usize,
    declaration_name: Option<usize>,
    output: &mut Vec<Node<'tree>>,
) {
    if node.start_byte() >= body_start || declaration_name == Some(node.id()) {
        return;
    }
    if matches!(
        node.kind(),
        "type_parameter" | "lifetime_parameter" | "const_parameter"
    ) && rust_generic_parameter_name(node)
        .is_some_and(|name| declaration_name != Some(name.id()))
    {
        return;
    }
    match node.kind() {
        "scoped_type_identifier" => {
            output.push(node);
            return;
        }
        "type_identifier" => {
            output.push(node);
            return;
        }
        "primitive_type" => return,
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_rust_type_nodes(child, body_start, declaration_name, output);
    }
}

fn collect_rust_generic_use_nodes<'tree>(
    node: Node<'tree>,
    body_start: usize,
    declaration_name: Option<usize>,
    output: &mut Vec<Node<'tree>>,
) {
    if node.start_byte() >= body_start || declaration_name == Some(node.id()) {
        return;
    }
    if matches!(
        node.kind(),
        "type_parameter" | "lifetime_parameter" | "const_parameter"
    ) && rust_generic_parameter_name(node)
        .is_some_and(|name| declaration_name != Some(name.id()))
    {
        return;
    }
    if matches!(node.kind(), "identifier" | "lifetime") {
        output.push(node);
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_rust_generic_use_nodes(child, body_start, declaration_name, output);
    }
}

fn rust_node_has_ancestor_before(node: Node<'_>, boundary: Node<'_>, kind: &str) -> bool {
    let mut parent = node.parent();
    while let Some(current) = parent {
        if current.id() == boundary.id() {
            return false;
        }
        if current.kind() == kind {
            return true;
        }
        parent = current.parent();
    }
    false
}

fn rust_ancestor_of_kind<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    let mut current = Some(node);
    while let Some(candidate) = current {
        if candidate.kind() == kind {
            return Some(candidate);
        }
        current = candidate.parent();
    }
    None
}

fn rust_primitive_type(raw: &str) -> bool {
    matches!(
        raw,
        "bool"
            | "char"
            | "str"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
            | "f32"
            | "f64"
            | "Self"
            | "self"
    )
}

fn rust_prelude_symbol(raw: &str) -> bool {
    crate::builtins::is_language_builtin_global("rust", raw)
}

fn rust_deferred_owner(raw: &str) -> bool {
    !raw.contains("::") && !rust_primitive_type(raw) && !rust_prelude_symbol(raw)
}

fn rust_is_public_use(raw: &str) -> bool {
    raw.trim_start().strip_prefix("pub").is_some_and(|suffix| {
        suffix
            .chars()
            .next()
            .is_some_and(|character| character.is_whitespace() || character == '(')
    })
}

fn rust_receiver_is_self(raw: &str) -> bool {
    let normalized = raw
        .chars()
        .filter(|character| {
            !character.is_whitespace() && !matches!(character, '(' | ')' | '&' | '*')
        })
        .collect::<String>();
    normalized.strip_prefix("mut").unwrap_or(&normalized) == "self"
}

fn rust_callable_owner(owner: &DeclarationContext) -> Option<&str> {
    owner.enclosing_type_qualified_name.as_deref().or_else(|| {
        (owner.kind == "method")
            .then(|| rust_qualified_parent(&owner.qualified_name))
            .flatten()
    })
}

fn rust_identity_is_internal(module: &str, qualified: &str) -> bool {
    let crate_name = module.split("::").next().unwrap_or("crate");
    qualified == crate_name
        || qualified.starts_with(&format!("{crate_name}::"))
        || qualified.starts_with(&format!("<{crate_name}::"))
}

fn rust_is_type_node(kind: &str) -> bool {
    matches!(
        kind,
        "type_identifier"
            | "scoped_type_identifier"
            | "generic_type"
            | "reference_type"
            | "primitive_type"
            | "tuple_type"
            | "array_type"
    )
}

fn first_named_child_of_kind<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| child.is_named() && child.kind() == kind)
}

fn rust_generic_parameter_name(parameter: Node<'_>) -> Option<Node<'_>> {
    parameter.child_by_field_name("name").or_else(|| {
        let kind = match parameter.kind() {
            "type_parameter" => "type_identifier",
            "lifetime_parameter" => "lifetime",
            "const_parameter" => "identifier",
            _ => return None,
        };
        first_named_child_of_kind(parameter, kind)
    })
}

fn split_qualified(raw: &str) -> (Option<&str>, &str) {
    let raw = raw.trim().trim_start_matches(['*', '&']);
    raw.rsplit_once("::")
        .or_else(|| raw.rsplit_once('.'))
        .map_or((None, raw), |(qualifier, spelling)| {
            (Some(qualifier), spelling)
        })
}

fn split_qualified_head(raw: &str) -> (&str, Option<&str>) {
    raw.split_once("::")
        .or_else(|| raw.split_once('.'))
        .map_or((raw, None), |(head, suffix)| (head, Some(suffix)))
}

fn python_super_receiver<'tree>(function: Node<'tree>, source: &[u8]) -> Option<Node<'tree>> {
    if function.kind() != "attribute" {
        return None;
    }
    let receiver = function.child_by_field_name("object")?;
    if receiver.kind() != "call" {
        return None;
    }
    let callable = receiver.child_by_field_name("function")?;
    (callable.kind() == "identifier"
        && callable.utf8_text(source).ok().map(str::trim) == Some("super"))
    .then_some(receiver)
}

fn python_has_decorator(definition: Node<'_>, source: &[u8], expected: &str) -> bool {
    let Some(decorated) = definition
        .parent()
        .filter(|node| node.kind() == "decorated_definition")
    else {
        return false;
    };
    let mut cursor = decorated.walk();
    decorated
        .children(&mut cursor)
        .filter(|child| child.kind() == "decorator")
        .filter_map(|decorator| decorator.utf8_text(source).ok())
        .map(|decorator| {
            decorator
                .trim()
                .trim_start_matches('@')
                .trim()
                .split_once('(')
                .map_or_else(
                    || decorator.trim().trim_start_matches('@').trim(),
                    |(name, _)| name.trim(),
                )
        })
        .any(|decorator| decorator == expected || decorator.ends_with(&format!(".{expected}")))
}

fn python_parameter_nodes(declaration: Node<'_>) -> Vec<PythonParameter<'_>> {
    let Some(parameters) = declaration.child_by_field_name("parameters") else {
        return Vec::new();
    };
    let mut cursor = parameters.walk();
    parameters
        .children(&mut cursor)
        .filter(|child| child.is_named())
        .filter_map(python_parameter)
        .collect()
}

fn python_parameter(parameter: Node<'_>) -> Option<PythonParameter<'_>> {
    let defaulted = matches!(
        parameter.kind(),
        "default_parameter" | "typed_default_parameter"
    ) || has_descendant(parameter, "default_parameter")
        || has_descendant(parameter, "typed_default_parameter");
    let annotation = python_parameter_annotation(parameter);
    let binding = parameter.child_by_field_name("name").unwrap_or(parameter);
    let binding = if matches!(
        binding.kind(),
        "typed_parameter" | "typed_default_parameter"
    ) {
        binding.child_by_field_name("name").unwrap_or(binding)
    } else {
        binding
    };
    let list_splat =
        binding.kind() == "list_splat_pattern" || has_descendant(binding, "list_splat_pattern");
    let dictionary_splat = binding.kind() == "dictionary_splat_pattern"
        || has_descendant(binding, "dictionary_splat_pattern");
    let name = if binding.kind() == "identifier" {
        binding
    } else {
        python_first_descendant(binding, "identifier")?
    };
    Some(PythonParameter {
        syntax: parameter,
        name,
        annotation,
        defaulted,
        list_splat,
        dictionary_splat,
    })
}

fn python_parameter_annotation(parameter: Node<'_>) -> Option<Node<'_>> {
    if let Some(annotation) = parameter.child_by_field_name("type") {
        return Some(annotation);
    }
    let mut cursor = parameter.walk();
    parameter
        .children(&mut cursor)
        .filter(|child| child.is_named())
        .find_map(python_parameter_annotation)
}

fn python_first_descendant<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        if child.kind() == kind {
            return Some(child);
        }
        if let Some(found) = python_first_descendant(child, kind) {
            return Some(found);
        }
    }
    None
}

fn split_python_annotation_top_level(raw: &str, delimiter: char) -> Option<Vec<&str>> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut brackets = 0_u16;
    let mut parentheses = 0_u16;
    let mut quote = None;
    let mut escaped = false;
    for (offset, character) in raw.char_indices() {
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == active_quote {
                quote = None;
            }
            continue;
        }
        match character {
            '\'' | '"' => quote = Some(character),
            '[' => brackets = brackets.checked_add(1)?,
            ']' => brackets = brackets.checked_sub(1)?,
            '(' => parentheses = parentheses.checked_add(1)?,
            ')' => parentheses = parentheses.checked_sub(1)?,
            character if character == delimiter && brackets == 0 && parentheses == 0 => {
                let part = raw.get(start..offset)?.trim();
                if part.is_empty() {
                    return None;
                }
                parts.push(part);
                start = offset.saturating_add(character.len_utf8());
            }
            _ => {}
        }
    }
    if quote.is_some() || brackets != 0 || parentheses != 0 {
        return None;
    }
    let tail = raw.get(start..)?.trim();
    if tail.is_empty() {
        return None;
    }
    parts.push(tail);
    Some(parts)
}

fn python_annotation_generic_open(raw: &str) -> Option<usize> {
    if !raw.ends_with(']') {
        return None;
    }
    let mut quote = None;
    let mut escaped = false;
    for (offset, character) in raw.char_indices() {
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == active_quote {
                quote = None;
            }
            continue;
        }
        match character {
            '\'' | '"' => quote = Some(character),
            '[' => return Some(offset),
            _ => {}
        }
    }
    None
}

fn python_builtin_annotation(raw: &str) -> Option<&'static str> {
    match raw {
        "None" | "NoneType" => Some("builtins.NoneType"),
        "str" => Some("builtins.str"),
        "int" => Some("builtins.int"),
        "float" => Some("builtins.float"),
        "bool" => Some("builtins.bool"),
        "bytes" => Some("builtins.bytes"),
        "bytearray" => Some("builtins.bytearray"),
        "complex" => Some("builtins.complex"),
        "object" => Some("builtins.object"),
        "type" => Some("builtins.type"),
        "list" => Some("builtins.list"),
        "dict" => Some("builtins.dict"),
        "set" => Some("builtins.set"),
        "frozenset" => Some("builtins.frozenset"),
        "tuple" => Some("builtins.tuple"),
        _ => None,
    }
}

fn python_normalize_typing_alias(target: &str) -> String {
    match target {
        "typing.List" => "builtins.list",
        "typing.Dict" => "builtins.dict",
        "typing.Set" => "builtins.set",
        "typing.FrozenSet" => "builtins.frozenset",
        "typing.Tuple" => "builtins.tuple",
        "typing.Type" => "builtins.type",
        _ => target,
    }
    .to_owned()
}

fn python_super_call_is_builtin(
    receiver: Node<'_>,
    call: Node<'_>,
    owner: &DeclarationContext,
    state: &DirectEvidenceState<'_>,
) -> bool {
    if owner.kind != "method" || owner.enclosing_type_qualified_name.is_none() {
        return false;
    }
    if receiver
        .child_by_field_name("arguments")
        .is_none_or(|arguments| arguments.named_child_count() != 0)
    {
        return false;
    }
    if state
        .binding_for_occurrence(owner, "super", receiver.start_byte(), true)
        .is_some()
        || state.python_module_bound_names.contains("super")
        || state.declarations.values().any(|declaration| {
            declaration.qualified_name == format!("{}.super", state.module_or_package)
        })
    {
        return false;
    }

    let mut ancestor = call.parent();
    while let Some(node) = ancestor {
        if matches!(
            node.kind(),
            "lambda"
                | "list_comprehension"
                | "dictionary_comprehension"
                | "set_comprehension"
                | "generator_expression"
        ) {
            return false;
        }
        if node.kind() == "function_definition" {
            let owned_by_current_method = state
                .declarations
                .get(&node.id())
                .is_some_and(|declaration| declaration.fact_id == owner.fact_id);
            let has_first_argument = node
                .child_by_field_name("parameters")
                .is_some_and(|parameters| parameters.named_child_count() != 0);
            return owned_by_current_method
                && has_first_argument
                && !crate::engine::python_bound_names(node, state.source, false).contains("super");
        }
        if node.kind() == "class_definition" {
            return false;
        }
        ancestor = node.parent();
    }
    false
}

fn resolve_python_module(current: &str, imported: &str, current_is_package: bool) -> String {
    let dots = imported
        .chars()
        .take_while(|character| *character == '.')
        .count();
    if dots == 0 {
        return imported.to_owned();
    }
    let mut parts = current.split('.').collect::<Vec<_>>();
    let parents_to_remove = if current_is_package {
        dots.saturating_sub(1)
    } else {
        dots
    };
    for _ in 0..parents_to_remove {
        parts.pop();
    }
    let suffix = imported.trim_start_matches('.');
    if !suffix.is_empty() {
        parts.push(suffix);
    }
    parts.join(".")
}

fn python_module_identity(path: &Path, source_file: &str) -> String {
    if source_file.contains('/') {
        return source_file
            .strip_suffix(".pyi")
            .or_else(|| source_file.strip_suffix(".py"))
            .unwrap_or(source_file)
            .trim_end_matches("/__init__")
            .replace('/', ".");
    }
    let stem = source_file
        .strip_suffix(".pyi")
        .or_else(|| source_file.strip_suffix(".py"))
        .unwrap_or(source_file);
    let parent = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if matches!(source_file, "__init__.py" | "__init__.pyi") {
        return if parent.is_empty() { stem } else { parent }.to_owned();
    }
    if !parent.is_empty()
        && !parent.starts_with('.')
        && !parent.starts_with("tmp")
        && parent != "repo"
    {
        format!("{parent}.{stem}")
    } else {
        stem.to_owned()
    }
}

fn go_package_identity(
    path: &Path,
    source_file: &str,
    source_bytes: &[u8],
    root: Node<'_>,
) -> String {
    let source = source_file.replace('\\', "/");
    let directory = source
        .rsplit_once('/')
        .map(|(directory, _)| directory.trim_matches('/'))
        .filter(|directory| !directory.is_empty() && *directory != ".");
    let path_identity = directory.map_or_else(
        || {
            path.parent()
                .and_then(Path::file_name)
                .and_then(|value| value.to_str())
                .map_or_else(|| file_stem(path), str::to_owned)
        },
        str::to_owned,
    );
    let mut clauses = Vec::new();
    collect_nodes(root, "package_clause", &mut clauses);
    let [clause] = clauses.as_slice() else {
        return path_identity;
    };
    let declared = clause
        .child_by_field_name("name")
        .or_else(|| clause.named_child(0))
        .and_then(|name| name.utf8_text(source_bytes).ok())
        .unwrap_or_default()
        .trim();
    let path_package = path_identity.rsplit('/').next().unwrap_or(&path_identity);
    if declared.is_empty() || declared == path_package {
        return path_identity;
    }
    path_identity.rsplit_once('/').map_or_else(
        || declared.to_owned(),
        |(parent, _)| format!("{parent}/{declared}"),
    )
}

fn collect_parser_error_ranges(node: Node<'_>, output: &mut Vec<(usize, usize)>) {
    if node.is_error() {
        let mut cursor = node.walk();
        let children = node.children(&mut cursor).collect::<Vec<_>>();
        if children.is_empty() {
            output.push((node.start_byte(), node.end_byte()));
        } else {
            for child in children {
                collect_parser_error_ranges(child, output);
            }
        }
        return;
    }
    if node.is_missing() {
        if let Some(parent) = node.parent() {
            output.push((parent.start_byte(), parent.end_byte()));
        } else {
            output.push((node.start_byte(), node.end_byte()));
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_parser_error_ranges(child, output);
    }
}

fn valid_python_import_target(target: &str) -> bool {
    target.split('.').all(valid_python_identifier)
}

fn valid_python_identifier(identifier: &str) -> bool {
    let mut characters = identifier.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    (first == '_' || first.is_alphabetic())
        && characters.all(|character| character == '_' || character.is_alphanumeric())
        && !is_python_hard_keyword(identifier)
}

fn direct_python_function<'tree>(
    root: Node<'tree>,
    name: &str,
    source: &[u8],
) -> Option<Node<'tree>> {
    direct_python_definition(root, name, "function_definition", source)
}

fn direct_python_definition<'tree>(
    root: Node<'tree>,
    name: &str,
    kind: &str,
    source: &[u8],
) -> Option<Node<'tree>> {
    let mut cursor = root.walk();
    let mut matches = root.children(&mut cursor).filter_map(|child| {
        let definition = if child.kind() == kind {
            Some(child)
        } else if child.kind() == "decorated_definition" {
            let mut nested = child.walk();
            child
                .children(&mut nested)
                .find(|candidate| candidate.kind() == kind)
        } else {
            None
        }?;
        definition
            .child_by_field_name("name")
            .and_then(|name_node| source.get(name_node.start_byte()..name_node.end_byte()))
            .is_some_and(|spelling| spelling == name.as_bytes())
            .then_some(definition)
    });
    let found = matches.next()?;
    matches.next().is_none().then_some(found)
}

fn python_scope_directive_names(
    function: Node<'_>,
    source: &[u8],
) -> (HashSet<String>, HashSet<String>) {
    fn walk(
        node: Node<'_>,
        source: &[u8],
        root: bool,
        global_names: &mut HashSet<String>,
        nonlocal_names: &mut HashSet<String>,
    ) {
        if !root && matches!(node.kind(), "function_definition" | "class_definition") {
            return;
        }
        let output = match node.kind() {
            "global_statement" => Some(&mut *global_names),
            "nonlocal_statement" => Some(&mut *nonlocal_names),
            _ => None,
        };
        if let Some(output) = output {
            let mut cursor = node.walk();
            for child in node
                .children(&mut cursor)
                .filter(|child| child.kind() == "identifier")
            {
                if let Ok(name) = child.utf8_text(source) {
                    output.insert(name.to_owned());
                }
            }
            return;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            walk(child, source, false, global_names, nonlocal_names);
        }
    }

    let mut global_names = HashSet::new();
    let mut nonlocal_names = HashSet::new();
    if let Some(body) = function.child_by_field_name("body") {
        walk(body, source, true, &mut global_names, &mut nonlocal_names);
    }
    (global_names, nonlocal_names)
}

fn collect_python_module_declaration_names(
    node: Node<'_>,
    source: &[u8],
    output: &mut HashSet<String>,
) {
    if matches!(node.kind(), "function_definition" | "class_definition") {
        if let Some(name) = node
            .child_by_field_name("name")
            .and_then(|name_node| source.get(name_node.start_byte()..name_node.end_byte()))
            .and_then(|name| std::str::from_utf8(name).ok())
            .filter(|name| valid_python_identifier(name))
        {
            output.insert(name.to_owned());
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_python_module_declaration_names(child, source, output);
    }
}

fn collect_python_module_mutations(
    node: Node<'_>,
    source: &[u8],
    binding_statements: &mut HashMap<String, HashSet<usize>>,
    deleted_names: &mut HashSet<String>,
    statement_id: usize,
) {
    if matches!(
        node.kind(),
        "function_definition" | "class_definition" | "decorated_definition"
    ) {
        collect_python_definition_time_bindings(node, source, binding_statements, statement_id);
        return;
    }
    if matches!(node.kind(), "annotated_assignment" | "augmented_assignment") {
        let mut names = HashSet::new();
        collect_python_binding_target_names(node.child_by_field_name("left"), source, &mut names);
        for name in names {
            binding_statements
                .entry(name)
                .or_default()
                .insert(statement_id);
        }
    } else if node.kind() == "delete_statement" {
        let mut cursor = node.walk();
        for target in node.children(&mut cursor).filter(|child| child.is_named()) {
            collect_python_binding_target_names(Some(target), source, deleted_names);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_python_module_mutations(
            child,
            source,
            binding_statements,
            deleted_names,
            statement_id,
        );
    }
}

fn collect_python_definition_time_bindings(
    node: Node<'_>,
    source: &[u8],
    binding_statements: &mut HashMap<String, HashSet<usize>>,
    statement_id: usize,
) {
    if node.kind() == "named_expression" {
        let mut names = HashSet::new();
        collect_python_binding_target_names(node.child_by_field_name("name"), source, &mut names);
        for name in names {
            binding_statements
                .entry(name)
                .or_default()
                .insert(statement_id);
        }
        return;
    }
    let body_id = node.child_by_field_name("body").map(|body| body.id());
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        if Some(child.id()) != body_id {
            collect_python_definition_time_bindings(
                child,
                source,
                binding_statements,
                statement_id,
            );
        }
    }
}

fn collect_python_binding_target_names(
    node: Option<Node<'_>>,
    source: &[u8],
    output: &mut HashSet<String>,
) {
    let Some(node) = node else {
        return;
    };
    if node.kind() == "identifier" {
        if let Some(name) = source
            .get(node.start_byte()..node.end_byte())
            .and_then(|name| std::str::from_utf8(name).ok())
            .filter(|name| valid_python_identifier(name))
        {
            output.insert(name.to_owned());
        }
    } else if matches!(
        node.kind(),
        "expression_list" | "pattern_list" | "tuple_pattern" | "list_pattern"
    ) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(|child| child.is_named()) {
            collect_python_binding_target_names(Some(child), source, output);
        }
    }
}

fn is_python_hard_keyword(identifier: &str) -> bool {
    matches!(
        identifier,
        "False"
            | "None"
            | "True"
            | "and"
            | "as"
            | "assert"
            | "async"
            | "await"
            | "break"
            | "class"
            | "continue"
            | "def"
            | "del"
            | "elif"
            | "else"
            | "except"
            | "finally"
            | "for"
            | "from"
            | "global"
            | "if"
            | "import"
            | "in"
            | "is"
            | "lambda"
            | "nonlocal"
            | "not"
            | "or"
            | "pass"
            | "raise"
            | "return"
            | "try"
            | "while"
            | "with"
            | "yield"
    )
}

fn valid_python_import_whitespace(statement: &str) -> bool {
    statement.chars().all(|character| {
        !character.is_whitespace() || matches!(character, ' ' | '\t' | '\r' | '\n' | '\u{000c}')
    })
}

fn valid_python_line_continuations(statement: &str) -> bool {
    let bytes = statement.as_bytes();
    let mut index = 0_usize;
    while index < bytes.len() {
        if bytes[index] != b'\\' {
            index += 1;
            continue;
        }
        index += 1;
        if bytes.get(index) == Some(&b'\r') {
            index += 1;
        }
        if bytes.get(index) != Some(&b'\n') {
            return false;
        }
        index += 1;
    }
    true
}

fn python_import_contains_wildcard(statement: &str) -> bool {
    let mut comment = false;
    for byte in statement.bytes() {
        match byte {
            b'\n' | b'\r' => comment = false,
            b'#' if !comment => comment = true,
            b'*' if !comment => return true,
            _ => {}
        }
    }
    false
}

fn python_wildcard_import_span(statement: &str) -> Option<(usize, usize)> {
    let uncommented_end = statement.find('#').unwrap_or(statement.len());
    let uncommented = statement.get(..uncommented_end)?;
    let star = uncommented.find('*')?;
    if uncommented.get(star.saturating_add(1)..)?.trim().is_empty()
        && uncommented.get(..star)?.trim_end().ends_with("import")
        && !uncommented.get(..star)?.contains('*')
    {
        Some((star, star.saturating_add(1)))
    } else {
        None
    }
}

fn collect_nodes<'tree>(node: Node<'tree>, kind: &str, output: &mut Vec<Node<'tree>>) {
    if node.kind() == kind {
        output.push(node);
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_nodes(child, kind, output);
    }
}

fn collect_named_targets<'tree>(node: Node<'tree>, kinds: &[&str], output: &mut Vec<Node<'tree>>) {
    if kinds.contains(&node.kind()) {
        output.push(node);
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_named_targets(child, kinds, output);
    }
}

fn go_result_type_nodes<'tree>(result: Node<'tree>) -> Vec<Option<Node<'tree>>> {
    if result.kind() != "parameter_list" {
        return vec![Some(result)];
    }
    let mut cursor = result.walk();
    let mut output = Vec::new();
    for parameter in result
        .children(&mut cursor)
        .filter(|child| child.is_named())
    {
        if !matches!(
            parameter.kind(),
            "parameter_declaration" | "variadic_parameter_declaration"
        ) {
            continue;
        }
        let mut parameter_cursor = parameter.walk();
        let name_count = parameter
            .children_by_field_name("name", &mut parameter_cursor)
            .count()
            .max(1);
        let type_node = parameter.child_by_field_name("type");
        output.extend(std::iter::repeat_n(type_node, name_count));
    }
    output
}

fn go_output_type(types: &[Option<String>], output_index: Option<u32>) -> Option<String> {
    if output_index.is_none() && types.len() != 1 {
        return None;
    }
    let index = output_index
        .and_then(|index| usize::try_from(index).ok())
        .unwrap_or_default();
    types.get(index).and_then(Clone::clone)
}

fn go_direct_type_target(node: Node<'_>) -> Option<Node<'_>> {
    match node.kind() {
        "type_identifier" | "qualified_type" => Some(node),
        "pointer_type" | "parenthesized_type" => {
            let mut cursor = node.walk();
            let mut children = node.children(&mut cursor).filter(|child| child.is_named());
            let child = children.next()?;
            children
                .next()
                .is_none()
                .then(|| go_direct_type_target(child))?
        }
        "generic_type" => node
            .child_by_field_name("type")
            .and_then(go_direct_type_target),
        _ => None,
    }
}

fn go_range_value_type_target(node: Node<'_>) -> Option<Node<'_>> {
    match node.kind() {
        "slice_type"
        | "array_type"
        | "implicit_length_array_type"
        | "map_type"
        | "channel_type" => {
            let mut cursor = node.walk();
            node.children(&mut cursor)
                .filter(|child| child.is_named())
                .last()
                .and_then(go_direct_type_target)
        }
        "pointer_type" | "parenthesized_type" => {
            let mut cursor = node.walk();
            let mut children = node.children(&mut cursor).filter(|child| child.is_named());
            let child = children.next()?;
            children
                .next()
                .is_none()
                .then(|| go_range_value_type_target(child))?
        }
        "parameter_declaration" => node
            .child_by_field_name("type")
            .and_then(go_range_value_type_target),
        "parameter_list" => {
            let mut cursor = node.walk();
            let mut children = node.children(&mut cursor).filter(|child| child.is_named());
            let child = children.next()?;
            children
                .next()
                .is_none()
                .then(|| go_range_value_type_target(child))?
        }
        _ => None,
    }
}

fn go_enclosing_range_value<'tree>(
    use_node: Node<'tree>,
    name: &str,
    source: &[u8],
) -> Option<Node<'tree>> {
    fn named_children(node: Node<'_>) -> Vec<Node<'_>> {
        let mut cursor = node.walk();
        node.children(&mut cursor)
            .filter(|child| child.is_named())
            .collect()
    }

    let mut ancestor = use_node.parent();
    while let Some(node) = ancestor {
        if node.kind() == "for_statement"
            && let Some(range) = named_children(node)
                .into_iter()
                .find(|child| child.kind() == "range_clause")
            && let Some(left) = range.child_by_field_name("left")
        {
            let variables = if left.kind() == "expression_list" {
                named_children(left)
            } else {
                vec![left]
            };
            let matching_variable = variables.iter().position(|variable| {
                variable.kind() == "identifier" && variable.utf8_text(source).ok() == Some(name)
            });
            if let Some(index) = matching_variable {
                return (variables.len() == 2 && index == 1)
                    .then(|| range.child_by_field_name("right"))
                    .flatten();
            }
        }
        if matches!(
            node.kind(),
            "function_declaration" | "method_declaration" | "func_literal"
        ) {
            break;
        }
        ancestor = node.parent();
    }
    None
}

fn has_descendant(node: Node<'_>, kind: &str) -> bool {
    if node.kind() == kind {
        return true;
    }
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .any(|child| has_descendant(child, kind))
}

fn go_receiver_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    let receiver = node.child_by_field_name("receiver")?;
    let mut targets = Vec::new();
    collect_named_targets(
        receiver,
        &["type_identifier", "qualified_type"],
        &mut targets,
    );
    targets
        .into_iter()
        .next()
        .and_then(|target| target.utf8_text(source).ok())
        .map(|name| {
            name.trim()
                .trim_start_matches('*')
                .rsplit('.')
                .next()
                .unwrap_or_default()
                .to_owned()
        })
        .filter(|name| !name.is_empty())
}

struct GoLexicalBinding {
    name: String,
    active_from: usize,
    active_until: usize,
}

fn go_local_initializer_with_index_before<'tree>(
    use_node: Node<'tree>,
    name: &str,
    source: &[u8],
) -> Option<(Node<'tree>, Option<u32>)> {
    fn named_children<'tree>(node: Node<'tree>) -> Vec<Node<'tree>> {
        let mut cursor = node.walk();
        node.children(&mut cursor)
            .filter(|child| child.is_named())
            .collect()
    }

    fn paired_initializer<'tree>(
        names: Node<'tree>,
        values: Node<'tree>,
        name: &str,
        source: &[u8],
    ) -> Option<(Node<'tree>, Option<u32>)> {
        let names = if names.kind() == "expression_list" {
            named_children(names)
        } else {
            vec![names]
        };
        let values = if values.kind() == "expression_list" {
            named_children(values)
        } else {
            vec![values]
        };
        let index = names.iter().position(|candidate| {
            candidate.kind() == "identifier" && candidate.utf8_text(source).ok() == Some(name)
        })?;
        if names.len() > 1 && values.len() == 1 && values[0].kind() == "call_expression" {
            return Some((values[0], u32::try_from(index).ok()));
        }
        values.get(index).copied().map(|value| (value, None))
    }

    fn in_statement<'tree>(
        statement: Node<'tree>,
        name: &str,
        source: &[u8],
    ) -> Option<(Node<'tree>, Option<u32>)> {
        match statement.kind() {
            "short_var_declaration" => paired_initializer(
                statement.child_by_field_name("left")?,
                statement.child_by_field_name("right")?,
                name,
                source,
            ),
            "var_spec" => {
                let mut cursor = statement.walk();
                let names = statement
                    .children_by_field_name("name", &mut cursor)
                    .collect::<Vec<_>>();
                let index = names
                    .iter()
                    .position(|candidate| candidate.utf8_text(source).ok() == Some(name))?;
                if let Some(type_node) = statement.child_by_field_name("type") {
                    return Some((type_node, None));
                }
                let values = statement.child_by_field_name("value")?;
                let values = if values.kind() == "expression_list" {
                    named_children(values)
                } else {
                    vec![values]
                };
                if names.len() > 1 && values.len() == 1 && values[0].kind() == "call_expression" {
                    return Some((values[0], u32::try_from(index).ok()));
                }
                values.get(index).copied().map(|value| (value, None))
            }
            "var_declaration" => named_children(statement)
                .into_iter()
                .rev()
                .find_map(|child| in_statement(child, name, source)),
            _ => None,
        }
    }

    let use_start = use_node.start_byte();
    let mut ancestor = use_node.parent();
    while let Some(scope) = ancestor {
        let for_initializer = if scope.kind() == "for_clause" {
            scope.child_by_field_name("initializer")
        } else if scope.kind() == "for_statement" {
            let mut cursor = scope.walk();
            scope
                .children(&mut cursor)
                .filter(|child| child.is_named() && child.kind() == "for_clause")
                .find_map(|clause| clause.child_by_field_name("initializer"))
        } else {
            None
        };
        if let Some(initializer) = for_initializer
            && initializer.end_byte() <= use_start
            && let Some(found) = in_statement(initializer, name, source)
        {
            return Some(found);
        }
        if matches!(scope.kind(), "block" | "statement_list") {
            let mut statements = named_children(scope);
            statements.reverse();
            if let Some(initializer) = statements
                .into_iter()
                .filter(|statement| statement.end_byte() <= use_start)
                .find_map(|statement| in_statement(statement, name, source))
            {
                return Some(initializer);
            }
        }
        if matches!(scope.kind(), "function_declaration" | "method_declaration") {
            break;
        }
        ancestor = scope.parent();
    }
    None
}

fn go_local_type_declaration_before(use_node: Node<'_>, name: &str, source: &[u8]) -> bool {
    fn declares_name(node: Node<'_>, name: &str, source: &[u8]) -> bool {
        fn matches_name(node: Node<'_>, name: &str, source: &[u8]) -> bool {
            matches!(node.kind(), "type_spec" | "type_alias")
                && node
                    .child_by_field_name("name")
                    .and_then(|candidate| candidate.utf8_text(source).ok())
                    == Some(name)
        }
        if matches_name(node, name, source) {
            return true;
        }
        let mut cursor = node.walk();
        node.children(&mut cursor)
            .filter(|child| child.is_named())
            .any(|child| matches_name(child, name, source))
    }

    let use_start = use_node.start_byte();
    let mut ancestor = use_node.parent();
    while let Some(scope) = ancestor {
        if matches!(scope.kind(), "block" | "statement_list") {
            let mut cursor = scope.walk();
            if scope
                .children(&mut cursor)
                .filter(|statement| statement.is_named() && statement.end_byte() <= use_start)
                .any(|statement| declares_name(statement, name, source))
            {
                return true;
            }
        }
        if matches!(scope.kind(), "function_declaration" | "method_declaration") {
            break;
        }
        ancestor = scope.parent();
    }
    false
}

fn go_enclosing_callable(call: Node<'_>) -> Option<Node<'_>> {
    let mut ancestor = call.parent();
    let mut outer_literal = None;
    while let Some(node) = ancestor {
        match node.kind() {
            "function_declaration" | "method_declaration" => return Some(node),
            "func_literal" => outer_literal = Some(node),
            _ => {}
        }
        ancestor = node.parent();
    }
    outer_literal
}

fn go_lexical_bindings(callable: Node<'_>, source: &[u8]) -> Vec<GoLexicalBinding> {
    fn collect_binding_identifiers(node: Node<'_>, source: &[u8], names: &mut Vec<String>) {
        if node.kind() == "identifier"
            && let Ok(name) = node.utf8_text(source)
            && !name.is_empty()
        {
            names.push(name.to_owned());
            return;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(|child| child.is_named()) {
            collect_binding_identifiers(child, source, names);
        }
    }

    fn push_bindings(
        names_node: Node<'_>,
        source: &[u8],
        active_from: usize,
        active_until: usize,
        bindings: &mut Vec<GoLexicalBinding>,
    ) {
        let mut names = Vec::new();
        collect_binding_identifiers(names_node, source, &mut names);
        names.sort();
        names.dedup();
        bindings.extend(names.into_iter().map(|name| GoLexicalBinding {
            name,
            active_from,
            active_until,
        }));
    }

    fn walk(node: Node<'_>, source: &[u8], scope_end: usize, bindings: &mut Vec<GoLexicalBinding>) {
        if node.kind() == "func_literal"
            && let Some(body) = node.child_by_field_name("body")
        {
            if let Some(parameters) = node.child_by_field_name("parameters") {
                push_bindings(
                    parameters,
                    source,
                    body.start_byte(),
                    body.end_byte(),
                    bindings,
                );
            }
            walk(body, source, body.end_byte(), bindings);
            return;
        }
        let scope_end = if matches!(
            node.kind(),
            "block"
                | "if_statement"
                | "for_statement"
                | "expression_switch_statement"
                | "type_switch_statement"
                | "select_statement"
                | "communication_case"
                | "expression_case"
                | "type_case"
        ) {
            node.end_byte()
        } else {
            scope_end
        };
        match node.kind() {
            "var_spec" => {
                let type_node = node.child_by_field_name("type");
                let value_node = node.child_by_field_name("value");
                let mut cursor = node.walk();
                for child in node.children(&mut cursor).filter(|child| {
                    child.is_named() && Some(*child) != type_node && Some(*child) != value_node
                }) {
                    if child.kind() == "identifier" {
                        push_bindings(child, source, node.end_byte(), scope_end, bindings);
                    }
                }
            }
            "short_var_declaration" => {
                if let Some(left) = node.child_by_field_name("left") {
                    push_bindings(left, source, node.end_byte(), scope_end, bindings);
                }
            }
            "for_statement" => {
                let body = node.child_by_field_name("body");
                let mut cursor = node.walk();
                if let Some(range) = node
                    .children(&mut cursor)
                    .find(|child| child.kind() == "range_clause")
                    && range.utf8_text(source).unwrap_or_default().contains(":=")
                    && let Some(left) = range.child_by_field_name("left")
                {
                    push_bindings(
                        left,
                        source,
                        range.end_byte(),
                        body.map_or(node.end_byte(), |body| body.end_byte()),
                        bindings,
                    );
                }
            }
            _ => {}
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(|child| child.is_named()) {
            walk(child, source, scope_end, bindings);
        }
    }

    let Some(body) = callable.child_by_field_name("body") else {
        return Vec::new();
    };
    let mut bindings = Vec::new();
    for field in ["receiver", "parameters", "result"] {
        if let Some(parameters) = callable.child_by_field_name(field) {
            push_bindings(
                parameters,
                source,
                body.start_byte(),
                body.end_byte(),
                &mut bindings,
            );
        }
    }
    walk(body, source, body.end_byte(), &mut bindings);
    bindings
}

fn is_python_builtin_type(name: &str) -> bool {
    matches!(
        name.rsplit('.').next().unwrap_or(name),
        "bool"
            | "bytes"
            | "complex"
            | "dict"
            | "float"
            | "frozenset"
            | "int"
            | "list"
            | "None"
            | "object"
            | "set"
            | "str"
            | "tuple"
    )
}

fn is_go_predeclared_type(name: &str) -> bool {
    matches!(
        name.rsplit('.').next().unwrap_or(name),
        "any"
            | "bool"
            | "byte"
            | "comparable"
            | "complex64"
            | "complex128"
            | "error"
            | "float32"
            | "float64"
            | "int"
            | "int8"
            | "int16"
            | "int32"
            | "int64"
            | "rune"
            | "string"
            | "uint"
            | "uint8"
            | "uint16"
            | "uint32"
            | "uint64"
            | "uintptr"
    )
}

fn qualified_binding_head(qualifier: &str) -> &str {
    split_qualified_head(qualifier).0
}

fn collect_rust_pattern_names(node: Node<'_>, names: &mut Vec<String>, source: &[u8]) {
    if node.kind() == "identifier" {
        let name = source
            .get(node.start_byte()..node.end_byte())
            .map_or_else(String::new, |bytes| {
                String::from_utf8_lossy(bytes).into_owned()
            });
        if !name.is_empty() && name != "_" {
            names.push(name);
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_rust_pattern_names(child, names, source);
    }
}

fn rust_is_lexical_scope_node(kind: &str) -> bool {
    matches!(
        kind,
        "block" | "match_block" | "unsafe_block" | "closure_expression"
    )
}

fn target_kinds_for_relation(relation: CandidateRelation) -> Vec<String> {
    match relation {
        CandidateRelation::Calls
        | CandidateRelation::IndirectCalls
        | CandidateRelation::Overrides
        | CandidateRelation::Tests => {
            vec!["function".to_owned(), "method".to_owned()]
        }
        CandidateRelation::Constructs => {
            vec![
                "class".to_owned(),
                "struct".to_owned(),
                "type_alias".to_owned(),
            ]
        }
        CandidateRelation::Decorates => vec!["function".to_owned(), "class".to_owned()],
        CandidateRelation::Annotates
        | CandidateRelation::Extends
        | CandidateRelation::Implements
        | CandidateRelation::UsesTrait
        | CandidateRelation::Embeds => vec![
            "class".to_owned(),
            "struct".to_owned(),
            "enum".to_owned(),
            "interface".to_owned(),
            "trait".to_owned(),
            "type_alias".to_owned(),
        ],
        CandidateRelation::References | CandidateRelation::TypeOf | CandidateRelation::Returns => {
            vec![
                "class".to_owned(),
                "struct".to_owned(),
                "enum".to_owned(),
                "interface".to_owned(),
                "trait".to_owned(),
                "type_alias".to_owned(),
                "parameter".to_owned(),
            ]
        }
        CandidateRelation::AccessesMember => vec!["field".to_owned(), "method".to_owned()],
        CandidateRelation::InvokesMacro => vec!["macro".to_owned()],
        CandidateRelation::Contains | CandidateRelation::Owns => Vec::new(),
        CandidateRelation::Imports | CandidateRelation::Reexports => {
            vec!["file".to_owned(), "module".to_owned(), "package".to_owned()]
        }
    }
}

fn evidence_declaration_body(node: Node<'_>) -> Option<Node<'_>> {
    node.child_by_field_name("body").or_else(|| {
        let mut cursor = node.walk();
        node.children(&mut cursor).find(|child| {
            matches!(
                child.kind(),
                "body" | "block" | "compound_statement" | "class_body" | "declaration_list"
            )
        })
    })
}

fn evidence_readable_signature(
    node: Node<'_>,
    body: Option<Node<'_>>,
    source: &[u8],
) -> Option<String> {
    let end = body.map_or(node.end_byte(), |body| body.start_byte());
    let raw = source.get(node.start_byte()..end)?;
    let compact = String::from_utf8_lossy(raw)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let compact = compact
        .trim()
        .trim_end_matches(['{', ':', ';'])
        .trim()
        .to_owned();
    if compact.is_empty() {
        return None;
    }
    let mut chars = compact.chars();
    let signature = chars.by_ref().take(500).collect::<String>();
    Some(if chars.next().is_some() {
        format!("{signature}…")
    } else {
        signature
    })
}

fn evidence_ast_hash(node: Node<'_>, source: &[u8], excluded: Option<usize>) -> String {
    let mut digest = Sha256::new();
    hash_evidence_ast_node(node, source, excluded, &mut digest);
    hex_sha256_digest(&digest.finalize())
}

fn hash_evidence_ast_node(
    node: Node<'_>,
    source: &[u8],
    excluded: Option<usize>,
    digest: &mut Sha256,
) {
    if excluded == Some(node.id()) || node.kind().contains("comment") {
        return;
    }
    digest.update(b"(");
    digest.update(node.kind().as_bytes());
    if node.child_count() == 0 {
        digest.update(b":");
        if let Some(bytes) = source.get(node.start_byte()..node.end_byte()) {
            digest.update(bytes);
        }
    } else {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            hash_evidence_ast_node(child, source, excluded, digest);
        }
    }
    digest.update(b")");
}

fn evidence_normalized_source_hash(source: &[u8]) -> String {
    let mut normalized = Vec::with_capacity(source.len());
    let mut index = 0;
    while index < source.len() {
        if source[index] == b'\r' && source.get(index + 1) == Some(&b'\n') {
            normalized.push(b'\n');
            index += 2;
        } else {
            normalized.push(source[index]);
            index += 1;
        }
    }
    hex_sha256_digest(&Sha256::digest(normalized))
}

fn hex_sha256_digest(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(value, "{byte:02x}");
    }
    value
}

fn ensure_capacity(name: &str, current: usize, limit: usize) -> Result<(), EvidenceError> {
    if current >= limit {
        return Err(EvidenceError::new(
            EvidenceErrorCode::ResourceLimit,
            format!("{name} count would exceed limit {limit}"),
        ));
    }
    Ok(())
}

const fn binding_kind_name(kind: BindingKind) -> &'static str {
    match kind {
        BindingKind::Import => "import",
        BindingKind::ImportAlias => "import_alias",
        BindingKind::Reexport => "reexport",
        BindingKind::LocalAlias => "local_alias",
        BindingKind::CallResult => "call_result",
        BindingKind::Package => "package",
        BindingKind::Member => "member",
    }
}

const fn symbol_namespace_name(namespace: Option<SymbolNamespace>) -> &'static str {
    match namespace {
        None => "",
        Some(SymbolNamespace::Value) => "value",
        Some(SymbolNamespace::Type) => "type",
        Some(SymbolNamespace::Namespace) => "namespace",
        Some(SymbolNamespace::ValueAndType) => "value_and_type",
    }
}

const fn semantic_role_name(role: SemanticRole) -> &'static str {
    match role {
        SemanticRole::Import => "import",
        SemanticRole::Reexport => "reexport",
        SemanticRole::Alias => "alias",
        SemanticRole::Call => "call",
        SemanticRole::CallableReference => "callable_reference",
        SemanticRole::Construction => "construction",
        SemanticRole::Decorator => "decorator",
        SemanticRole::Annotation => "annotation",
        SemanticRole::BaseType => "base_type",
        SemanticRole::Override => "override",
        SemanticRole::TypeReference => "type_reference",
        SemanticRole::MemberAccess => "member_access",
        SemanticRole::Ownership => "ownership",
        SemanticRole::Receiver => "receiver",
        SemanticRole::Embedding => "embedding",
        SemanticRole::TraitBound => "trait_bound",
        SemanticRole::MacroInvocation => "macro_invocation",
    }
}

const fn candidate_relation_name(relation: CandidateRelation) -> &'static str {
    match relation {
        CandidateRelation::Calls => "calls",
        CandidateRelation::IndirectCalls => "indirect_calls",
        CandidateRelation::Constructs => "constructs",
        CandidateRelation::Decorates => "decorates",
        CandidateRelation::Annotates => "annotates",
        CandidateRelation::Extends => "extends",
        CandidateRelation::Implements => "implements",
        CandidateRelation::UsesTrait => "uses_trait",
        CandidateRelation::Overrides => "overrides",
        CandidateRelation::References => "references",
        CandidateRelation::TypeOf => "type_of",
        CandidateRelation::Returns => "returns",
        CandidateRelation::AccessesMember => "accesses_member",
        CandidateRelation::Contains => "contains",
        CandidateRelation::Owns => "owns",
        CandidateRelation::Embeds => "embeds",
        CandidateRelation::Imports => "imports",
        CandidateRelation::Reexports => "reexports",
        CandidateRelation::InvokesMacro => "invokes_macro",
        CandidateRelation::Tests => "tests",
    }
}

fn rust_platform_cfg(node: Node<'_>, source: &[u8]) -> Option<RustPlatformCfg> {
    let parse = |attribute: Node<'_>| {
        let compact =
            String::from_utf8_lossy(&source[attribute.start_byte()..attribute.end_byte()])
                .chars()
                .filter(|character| !character.is_whitespace())
                .collect::<String>();
        match compact.as_str() {
            "#[cfg(not(any(unix,windows)))]" | "#[cfg(not(any(windows,unix)))]" => {
                Some(RustPlatformCfg::Fallback)
            }
            "#[cfg(unix)]" => Some(RustPlatformCfg::Unix),
            "#[cfg(windows)]" => Some(RustPlatformCfg::Windows),
            _ => None,
        }
    };
    let mut platforms = BTreeSet::new();
    let mut sibling = node.prev_named_sibling();
    while let Some(attribute) = sibling {
        if attribute.kind() != "attribute_item" {
            break;
        }
        if let Some(platform) = parse(attribute) {
            platforms.insert(platform);
        }
        sibling = attribute.prev_named_sibling();
    }
    let mut cursor = node.walk();
    for attribute in node
        .children(&mut cursor)
        .filter(|child| child.kind() == "attribute_item")
    {
        if let Some(platform) = parse(attribute) {
            platforms.insert(platform);
        }
    }
    let platforms = platforms.into_iter().collect::<Vec<_>>();
    let [platform] = platforms.as_slice() else {
        return None;
    };
    Some(*platform)
}

fn rust_has_test_attribute(node: Node<'_>, source: &[u8]) -> bool {
    let is_test = |attribute: Node<'_>| {
        let text = String::from_utf8_lossy(&source[attribute.start_byte()..attribute.end_byte()]);
        let compact = text
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>();
        let path = compact
            .strip_prefix("#[")
            .and_then(|value| value.strip_suffix(']'))
            .unwrap_or(&compact)
            .split('(')
            .next()
            .unwrap_or_default();
        path.rsplit("::").next() == Some("test")
    };
    let mut sibling = node.prev_named_sibling();
    while let Some(attribute) = sibling {
        if attribute.kind() != "attribute_item" {
            break;
        }
        if is_test(attribute) {
            return true;
        }
        sibling = attribute.prev_named_sibling();
    }
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .any(|child| child.kind() == "attribute_item" && is_test(child))
}
