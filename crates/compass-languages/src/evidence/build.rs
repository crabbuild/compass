use std::collections::{HashMap, HashSet};
use std::path::Path;

use sha2::{Digest, Sha256};
use tree_sitter::Node;

use crate::{AdapterProfile, EXTRACTION_SEMANTICS_VERSION, file_stem, make_id};

use super::model::{
    AdapterIdentity, BindingFact, BindingKind, CandidateRelation, DeclarationFact,
    EvidenceDiagnostic, EvidenceRange, OccurrenceFact, RelationshipCandidate, ResolutionConstraint,
    ScopeFact, SemanticEvidenceBatch, SemanticRole,
};
use super::validate::{EvidenceError, EvidenceErrorCode, EvidenceLimits, validate_evidence};

/// Bounded direct-construction API shared by hard-cut language adapters.
pub struct EvidenceBuilder {
    batch: SemanticEvidenceBatch,
    source_file: String,
    limits: EvidenceLimits,
}

#[derive(Default)]
struct DeclarationMetadata {
    signature: Option<String>,
    signature_hash: Option<String>,
    implementation_hash: Option<String>,
    source_hash: Option<String>,
}

impl EvidenceBuilder {
    #[must_use]
    pub fn new(
        profile: &'static AdapterProfile,
        producer: impl Into<String>,
        source_file: impl Into<String>,
        limits: EvidenceLimits,
    ) -> Self {
        Self {
            batch: SemanticEvidenceBatch {
                adapter: AdapterIdentity {
                    language: profile.language.to_owned(),
                    producer: producer.into(),
                    capabilities: profile.capabilities.to_vec(),
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
        ensure_capacity(
            "declarations",
            self.batch.declarations.len(),
            self.limits.declarations,
        )?;
        let id = self.stable_id(
            "declaration",
            &[
                kind,
                graph_node_id,
                name,
                qualified_name,
                module_or_package.unwrap_or_default(),
                scope_id.unwrap_or_default(),
                &range.start_byte.to_string(),
                &range.end_byte.to_string(),
            ],
        );
        self.batch.declarations.push(DeclarationFact {
            id: id.clone(),
            language: self.batch.adapter.language.clone(),
            graph_node_id: graph_node_id.to_owned(),
            kind: kind.to_owned(),
            name: name.to_owned(),
            qualified_name: qualified_name.to_owned(),
            module_or_package: module_or_package.map(str::to_owned),
            scope_id: scope_id.map(str::to_owned),
            signature: metadata.signature,
            signature_hash: metadata.signature_hash,
            implementation_hash: metadata.implementation_hash,
            source_hash: metadata.source_hash,
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
            language: self.batch.adapter.language.clone(),
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
        ensure_capacity("bindings", self.batch.bindings.len(), self.limits.bindings)?;
        let id = self.stable_id(
            "binding",
            &[
                binding_kind_name(kind),
                spelling,
                qualified_target,
                target_declaration_id.unwrap_or_default(),
                scope_id.unwrap_or_default(),
                &range.start_byte.to_string(),
                &range.end_byte.to_string(),
            ],
        );
        self.batch.bindings.push(BindingFact {
            id: id.clone(),
            language: self.batch.adapter.language.clone(),
            kind,
            spelling: spelling.to_owned(),
            qualified_target: qualified_target.to_owned(),
            target_declaration_id: target_declaration_id.map(str::to_owned),
            scope_id: scope_id.map(str::to_owned),
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
            language: self.batch.adapter.language.clone(),
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
        constraints: ResolutionConstraint,
    ) -> Result<String, EvidenceError> {
        ensure_capacity(
            "candidates",
            self.batch.candidates.len(),
            self.limits.candidates,
        )?;
        let id = self.stable_id(
            "candidate",
            &[
                candidate_relation_name(relation),
                source_declaration_id,
                occurrence_id.unwrap_or_default(),
                binding_id.unwrap_or_default(),
                target_spelling,
                constraints.exact_language.as_deref().unwrap_or_default(),
                constraints.module_or_package.as_deref().unwrap_or_default(),
                constraints.scope_id.as_deref().unwrap_or_default(),
                constraints.qualified_name.as_deref().unwrap_or_default(),
            ],
        );
        self.batch.candidates.push(RelationshipCandidate {
            id: id.clone(),
            language: self.batch.adapter.language.clone(),
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
            language: self.batch.adapter.language.clone(),
            fact_id: fact_id.map(str::to_owned),
            range,
            message: message.to_owned(),
        });
        Ok(())
    }

    pub fn finish(mut self) -> Result<SemanticEvidenceBatch, EvidenceError> {
        self.batch.adapter.capabilities.sort_unstable();
        self.batch.adapter.capabilities.dedup();
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
            self.batch.adapter.language.as_str(),
            self.batch.adapter.producer.as_str(),
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

/// Extract Python or Go universal evidence directly from the parser tree.
///
/// This path never reads `RawNodeRecord`, `RawEdgeRecord`, or `RawCall`.
pub(crate) fn extract_tree_evidence(
    path: &Path,
    source_file: &str,
    source: &[u8],
    root: Node<'_>,
    profile: &'static AdapterProfile,
) -> Result<SemanticEvidenceBatch, EvidenceError> {
    let mut state = DirectAdapterState::new(path, source_file, source, profile);
    if root.end_byte() == root.start_byte() {
        return state.builder.finish();
    }
    state.capture_parser_errors(root);
    state.add_file(root)?;
    match profile.language {
        "python" => state.extract_python(root)?,
        "go" => state.extract_go(root)?,
        _ => {
            return Err(EvidenceError::new(
                EvidenceErrorCode::InvalidAdapter,
                format!(
                    "language {:?} has no direct universal extractor",
                    profile.language
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
    state.builder.finish()
}

#[derive(Clone)]
struct DeclarationContext {
    fact_id: String,
    scope_id: String,
    graph_node_id: String,
    name: String,
    qualified_name: String,
    kind: String,
}

struct ImportBindingVersion {
    binding_id: String,
    target: String,
    active_from: usize,
}

struct DirectAdapterState<'source> {
    path: &'source Path,
    source_file: &'source str,
    source: &'source [u8],
    language: &'static str,
    module_or_package: String,
    stem: String,
    file: Option<DeclarationContext>,
    declarations: HashMap<usize, DeclarationContext>,
    import_bindings: HashMap<String, HashMap<String, Vec<ImportBindingVersion>>>,
    local_bindings: HashMap<String, HashMap<String, String>>,
    local_targets: HashMap<String, HashMap<String, String>>,
    ambiguous_bindings: HashSet<(String, String)>,
    go_lexical_bindings: HashMap<usize, Vec<GoLexicalBinding>>,
    graph_ids: HashSet<String>,
    parser_error_ranges: Vec<(usize, usize)>,
    builder: EvidenceBuilder,
}

impl<'source> DirectAdapterState<'source> {
    fn new(
        path: &'source Path,
        source_file: &'source str,
        source: &'source [u8],
        profile: &'static AdapterProfile,
    ) -> Self {
        let stem = file_stem(path);
        let module_or_package = if profile.language == "python" {
            python_module_identity(path, source_file)
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
            language: profile.language,
            module_or_package,
            stem,
            file: None,
            declarations: HashMap::new(),
            import_bindings: HashMap::new(),
            local_bindings: HashMap::new(),
            local_targets: HashMap::new(),
            ambiguous_bindings: HashSet::new(),
            go_lexical_bindings: HashMap::new(),
            graph_ids: HashSet::new(),
            parser_error_ranges: Vec::new(),
            builder: EvidenceBuilder::new(
                profile,
                format!("compass.languages.{}.universal", profile.language),
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

    fn declaration_metadata(&self, node: Node<'_>) -> DeclarationMetadata {
        let body = evidence_declaration_body(node);
        DeclarationMetadata {
            signature: evidence_readable_signature(node, body, self.source),
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

    fn add_file(&mut self, root: Node<'_>) -> Result<(), EvidenceError> {
        let label = self
            .path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or(self.source_file);
        let graph_node_id = make_id(&[&self.path.to_string_lossy()]);
        self.graph_ids.insert(graph_node_id.clone());
        let range = range_for_node(self.source_file, root);
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
        self.collect_python_declarations(root, &file, None)?;
        self.collect_python_imports(root, &file)?;
        let module_bound = crate::engine::python_bound_names(root, self.source, true);
        self.walk_python_indirect(root, &file, true, &module_bound)?;
        self.walk_python_evidence(root, &file, true)
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
        class_owner: Option<&DeclarationContext>,
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
            let qualified_name = if let Some(class_owner) = class_owner {
                format!("{}::{name}", class_owner.qualified_name)
            } else {
                format!("{}.{}", self.module_or_package, name)
            };
            let graph_node_id = if let Some(class_owner) = class_owner {
                make_id(&[&class_owner.graph_node_id, &name])
            } else {
                make_id(&[
                    &self.stem,
                    qualified_name.rsplit('.').next().unwrap_or(&name),
                ])
            };
            let graph_node_id = self.unique_graph_id(graph_node_id, node);
            let kind = if is_class {
                "class"
            } else if class_owner.is_some() {
                "method"
            } else {
                "function"
            };
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
            let scope_id = self.builder.open_scope(
                kind,
                Some(&fact_id),
                Some(&owner.scope_id),
                range_for_node(self.source_file, node),
            )?;
            let context = DeclarationContext {
                fact_id,
                scope_id,
                graph_node_id,
                name,
                qualified_name,
                kind: kind.to_owned(),
            };
            self.add_ownership(owner, &context)?;
            self.declarations.insert(node.id(), context.clone());
            if is_class {
                let body = node.child_by_field_name("body").unwrap_or(node);
                let mut cursor = body.walk();
                for child in body.children(&mut cursor).filter(|child| child.is_named()) {
                    self.collect_python_declarations(child, &context, Some(&context))?;
                }
            }
            return Ok(());
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(|child| child.is_named()) {
            self.collect_python_declarations(child, owner, class_owner)?;
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
            }
            self.add_python_annotations(node, &active)?;
            if node.kind() == "function_definition" {
                let body = node.child_by_field_name("body").unwrap_or(node);
                let bound = crate::engine::python_bound_names(node, self.source, false);
                self.walk_python_indirect(body, &active, true, &bound)?;
            }
        }
        match node.kind() {
            "import_statement" | "import_from_statement" => return Ok(()),
            "call" => self.add_call(node, &active, "call")?,
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

    fn walk_python_indirect(
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
                    self.add_python_callable_reference(owner, candidate, "argument", bound)?;
                }
            }
        }
        if matches!(node.kind(), "dictionary" | "list" | "set" | "tuple") {
            let mut identifiers = Vec::new();
            crate::engine::collect_python_collection_values(node, &mut identifiers);
            for identifier in identifiers {
                self.add_python_callable_reference(owner, Some(identifier), "collection", bound)?;
            }
        } else if node.kind() == "assignment"
            && let Some(value) = node.child_by_field_name("right")
        {
            let mut identifiers = Vec::new();
            crate::engine::collect_python_reference_values(value, &mut identifiers);
            for identifier in identifiers {
                self.add_python_callable_reference(owner, Some(identifier), "assignment", bound)?;
            }
        } else if node.kind() == "return_statement" {
            let mut cursor = node.walk();
            if let Some(value) = node.children(&mut cursor).find(|child| child.is_named()) {
                let mut identifiers = Vec::new();
                crate::engine::collect_python_reference_values(value, &mut identifiers);
                for identifier in identifiers {
                    self.add_python_callable_reference(owner, Some(identifier), "return", bound)?;
                }
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk_python_indirect(child, owner, false, bound)?;
        }
        Ok(())
    }

    fn add_python_callable_reference(
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
            CandidateRelation::IndirectCalls,
            &owner.fact_id,
            Some(&occurrence_id),
            binding.as_deref(),
            &spelling,
            ResolutionConstraint {
                exact_language: Some(self.language.to_owned()),
                module_or_package: qualified_name
                    .as_deref()
                    .and_then(|qualified| qualified.rsplit_once('.').map(|(module, _)| module))
                    .map(str::to_owned)
                    .or_else(|| Some(self.module_or_package.clone())),
                scope_id: Some(owner.scope_id.clone()),
                qualified_name: qualified_name.clone(),
                allowed_target_kinds: vec!["function".to_owned(), "method".to_owned()],
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
            || python_import_contains_wildcard(&statement)
        {
            return Ok(());
        }
        let occurrence_range = range_for_node(self.source_file, node);
        let module = node.child_by_field_name("module_name").map(|module| {
            resolve_python_module(
                &self.module_or_package,
                &self.text(module),
                self.path.file_name().and_then(|name| name.to_str()) == Some("__init__.py"),
            )
        });
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
            let (target_name, alias) = if imported.kind() == "aliased_import" {
                let Some(target) = imported.child_by_field_name("name") else {
                    continue;
                };
                (
                    self.text(target),
                    imported
                        .child_by_field_name("alias")
                        .map(|alias| self.text(alias)),
                )
            } else {
                (self.text(imported), None)
            };
            if !valid_python_import_target(&target_name)
                || alias
                    .as_deref()
                    .is_some_and(|alias| !valid_python_identifier(alias))
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
                occurrence_range.clone(),
            )?;
        }
        Ok(())
    }

    fn add_python_import_binding(
        &mut self,
        imported: Node<'_>,
        owner: &DeclarationContext,
        local: String,
        binding_target: String,
        import_target: String,
        occurrence_range: EvidenceRange,
    ) -> Result<(), EvidenceError> {
        let is_reexport = owner.kind == "file"
            && self.path.file_name().and_then(|name| name.to_str()) == Some("__init__.py");
        let kind = if is_reexport {
            BindingKind::Reexport
        } else if local == binding_target.rsplit('.').next().unwrap_or_default() {
            BindingKind::Import
        } else {
            BindingKind::ImportAlias
        };
        let range = range_for_node(self.source_file, imported);
        let binding_id = self.builder.bind(
            kind,
            &local,
            &binding_target,
            None,
            Some(&owner.scope_id),
            range.clone(),
        )?;
        self.record_import_binding(
            owner,
            &local,
            &binding_target,
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
                exact_language: Some(self.language.to_owned()),
                module_or_package: import_target
                    .rsplit_once('.')
                    .map(|(module, _)| module.to_owned()),
                scope_id: Some(owner.scope_id.clone()),
                qualified_name: Some(import_target.clone()),
                allowed_target_kinds: vec![
                    "file".to_owned(),
                    "module".to_owned(),
                    "class".to_owned(),
                    "function".to_owned(),
                ],
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
        let mut cursor = arguments.walk();
        for argument in arguments
            .children(&mut cursor)
            .filter(|child| child.is_named())
        {
            let target = match argument.kind() {
                "identifier" | "attribute" => Some(argument),
                "subscript" => argument
                    .child_by_field_name("value")
                    .filter(|value| matches!(value.kind(), "identifier" | "attribute")),
                _ => None,
            };
            let Some(target) = target else {
                continue;
            };
            let raw = self.text(target);
            let (qualifier, spelling) = split_qualified(&raw);
            self.add_relationship_occurrence(
                SemanticRole::BaseType,
                CandidateRelation::Extends,
                owner,
                spelling,
                qualifier,
                target,
            )?;
        }
        Ok(())
    }

    fn add_python_annotations(
        &mut self,
        declaration: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
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

    fn extract_go(&mut self, root: Node<'_>) -> Result<(), EvidenceError> {
        let file = self.file.clone().ok_or_else(|| {
            EvidenceError::new(
                EvidenceErrorCode::InvalidFact,
                "non-empty Go source has no file evidence",
            )
        })?;
        self.collect_go_declarations(root, &file)?;
        self.walk_go_evidence(root, &file, true)
    }

    fn collect_go_declarations(
        &mut self,
        node: Node<'_>,
        file: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        if node.kind() == "type_spec" {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = self.text(name_node);
                let qualified_name = format!("{}.{}", self.module_or_package, name);
                let graph_node_id =
                    self.unique_graph_id(make_id(&[&self.module_or_package, &name]), node);
                let kind = if has_descendant(node, "struct_type") {
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
                let context = DeclarationContext {
                    fact_id,
                    scope_id,
                    graph_node_id,
                    name,
                    qualified_name,
                    kind: kind.to_owned(),
                };
                self.add_ownership(file, &context)?;
                self.declarations.insert(node.id(), context);
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
            let context = DeclarationContext {
                fact_id,
                scope_id,
                graph_node_id,
                name,
                qualified_name,
                kind: kind.to_owned(),
            };
            self.add_ownership(file, &context)?;
            self.declarations.insert(node.id(), context);
            return Ok(());
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(|child| child.is_named()) {
            self.collect_go_declarations(child, file)?;
        }
        Ok(())
    }

    fn walk_go_evidence(
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
        if matches!(node.kind(), "function_declaration" | "method_declaration") {
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
        if matches!(node.kind(), "function_declaration" | "method_declaration")
            || (node.kind() == "type_spec"
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
                    "function_declaration" | "method_declaration" | "type_spec"
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
        for field in ["receiver", "parameters"] {
            let Some(parameters) = declaration.child_by_field_name(field) else {
                continue;
            };
            let mut parameter_declarations = Vec::new();
            collect_nodes(
                parameters,
                "parameter_declaration",
                &mut parameter_declarations,
            );
            for parameter in parameter_declarations {
                let Some(type_node) = parameter.child_by_field_name("type") else {
                    continue;
                };
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
                let mut cursor = parameter.walk();
                for name_node in parameter.children_by_field_name("name", &mut cursor) {
                    let name = self.text(name_node);
                    if name.is_empty() || name == "_" {
                        continue;
                    }
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
                    exact_language: Some(self.language.to_owned()),
                    module_or_package: Some(target.clone()),
                    scope_id: Some(owner.scope_id.clone()),
                    qualified_name: Some(target),
                    allowed_target_kinds: vec!["file".to_owned(), "package".to_owned()],
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
            "function_declaration" | "method_declaration"
        ) {
            roots.extend(
                ["receiver", "parameters", "result"]
                    .into_iter()
                    .filter_map(|field| declaration.child_by_field_name(field)),
            );
        } else if let Some(type_node) = declaration.child_by_field_name("type") {
            roots.push(type_node);
        }
        let mut targets = Vec::new();
        for root in roots {
            collect_named_targets(root, &["type_identifier", "qualified_type"], &mut targets);
        }
        for target in targets {
            let raw = self.text(target);
            if raw == owner.name || is_go_predeclared_type(&raw) {
                continue;
            }
            let (qualifier, spelling) = split_qualified(&raw);
            self.add_relationship_occurrence(
                SemanticRole::TypeReference,
                CandidateRelation::References,
                owner,
                spelling,
                qualifier,
                target,
            )?;
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
        let construction =
            self.language == "python" && spelling.chars().next().is_some_and(char::is_uppercase);
        let (role, relation) = if construction {
            (SemanticRole::Construction, CandidateRelation::Constructs)
        } else {
            (SemanticRole::Call, CandidateRelation::Calls)
        };
        let binding = self
            .binding_for_occurrence(
                owner,
                qualifier.map(qualified_binding_head).unwrap_or(spelling),
                function.start_byte(),
                allow_later_file_binding,
            )
            .cloned();
        let qualified_name = qualifier
            .and_then(|qualifier| {
                self.local_target_for(owner, qualifier)
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
                self.imported_target_for_occurrence(
                    owner,
                    spelling,
                    function.start_byte(),
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
            range_for_node(self.source_file, function),
        )?;
        self.builder.relate(
            relation,
            &owner.fact_id,
            Some(&occurrence_id),
            binding.as_deref(),
            spelling,
            ResolutionConstraint {
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
                allowed_target_kinds: if construction {
                    vec![
                        "class".to_owned(),
                        "struct".to_owned(),
                        "type_alias".to_owned(),
                    ]
                } else {
                    vec!["function".to_owned(), "method".to_owned()]
                },
                allow_external: qualified_name.is_some(),
            },
        )?;
        let _ = call_kind;
        Ok(())
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
            .cloned();
        let qualified_name = qualifier
            .and_then(|qualifier| {
                self.local_target_for(owner, qualifier)
                    .map(|target| format!("{target}::{spelling}"))
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
                allowed_target_kinds: target_kinds_for_relation(relation),
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
        if self.language != "python" && !versions.is_empty() {
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
        if self
            .import_bindings
            .get(&owner.scope_id)
            .is_some_and(|bindings| bindings.contains_key(name))
        {
            return Some(owner.scope_id.as_str());
        }
        let file_scope = self.file.as_ref()?.scope_id.as_str();
        (file_scope != owner.scope_id
            && self
                .import_bindings
                .get(file_scope)
                .is_some_and(|bindings| bindings.contains_key(name)))
        .then_some(file_scope)
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
        self.local_bindings
            .get(&owner.scope_id)
            .and_then(|bindings| bindings.get(name))
            .or_else(|| {
                self.import_binding_version_at(owner, name, use_start, allow_later_file_binding)
                    .map(|binding| &binding.binding_id)
            })
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
        let (head, suffix) = qualifier
            .split_once('.')
            .map_or((qualifier, None), |(head, suffix)| (head, Some(suffix)));
        self.imported_target_for_occurrence(owner, head, use_start, allow_later_file_binding)
            .map(|target| {
                suffix.map_or_else(|| target.clone(), |suffix| format!("{target}.{suffix}"))
            })
    }

    fn import_binding_is_ambiguous(&self, owner: &DeclarationContext, name: &str) -> bool {
        if self
            .local_bindings
            .get(&owner.scope_id)
            .is_some_and(|bindings| bindings.contains_key(name))
        {
            return false;
        }
        if self
            .import_bindings
            .get(&owner.scope_id)
            .is_some_and(|bindings| bindings.contains_key(name))
        {
            return self
                .ambiguous_bindings
                .contains(&(owner.scope_id.clone(), name.to_owned()));
        }
        self.file.as_ref().is_some_and(|file| {
            self.ambiguous_bindings
                .contains(&(file.scope_id.clone(), name.to_owned()))
        })
    }

    fn local_target_for(&self, owner: &DeclarationContext, name: &str) -> Option<&String> {
        self.local_targets
            .get(&owner.scope_id)
            .and_then(|targets| targets.get(name))
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
                exact_language: Some(self.language.to_owned()),
                module_or_package: Some(self.module_or_package.clone()),
                scope_id: Some(owner.scope_id.clone()),
                qualified_name: Some(child.qualified_name.clone()),
                allowed_target_kinds: vec![child.kind.clone()],
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

fn split_qualified(raw: &str) -> (Option<&str>, &str) {
    let raw = raw.trim().trim_start_matches(['*', '&']);
    raw.rsplit_once('.')
        .map_or((None, raw), |(qualifier, spelling)| {
            (Some(qualifier), spelling)
        })
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
            .trim_end_matches(".py")
            .trim_end_matches("/__init__")
            .replace('/', ".");
    }
    let stem = source_file.trim_end_matches(".py");
    let parent = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if source_file == "__init__.py" {
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
    for field in ["receiver", "parameters"] {
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
    qualifier.split('.').next().unwrap_or(qualifier)
}

fn target_kinds_for_relation(relation: CandidateRelation) -> Vec<String> {
    match relation {
        CandidateRelation::Calls | CandidateRelation::IndirectCalls => {
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
        | CandidateRelation::References
        | CandidateRelation::Embeds => vec![
            "class".to_owned(),
            "struct".to_owned(),
            "interface".to_owned(),
            "type_alias".to_owned(),
        ],
        CandidateRelation::AccessesMember => vec!["field".to_owned(), "method".to_owned()],
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
        BindingKind::Package => "package",
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
        SemanticRole::TypeReference => "type_reference",
        SemanticRole::MemberAccess => "member_access",
        SemanticRole::Ownership => "ownership",
        SemanticRole::Receiver => "receiver",
        SemanticRole::Embedding => "embedding",
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
        CandidateRelation::References => "references",
        CandidateRelation::AccessesMember => "accesses_member",
        CandidateRelation::Contains => "contains",
        CandidateRelation::Owns => "owns",
        CandidateRelation::Embeds => "embeds",
        CandidateRelation::Imports => "imports",
        CandidateRelation::Reexports => "reexports",
    }
}
