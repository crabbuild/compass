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
                    id: profile.id.to_owned(),
                    language: profile.language.to_owned(),
                    version: profile.version,
                    evidence_schema: profile.evidence_schema.to_owned(),
                    profile: profile.profile,
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
        mut constraints: ResolutionConstraint,
    ) -> Result<String, EvidenceError> {
        ensure_capacity(
            "candidates",
            self.batch.candidates.len(),
            self.limits.candidates,
        )?;
        constraints.allowed_target_kinds.sort_unstable();
        constraints.allowed_target_kinds.dedup();
        let mut identity = vec![
            candidate_relation_name(relation),
            source_declaration_id,
            occurrence_id.unwrap_or_default(),
            binding_id.unwrap_or_default(),
            target_spelling,
            constraints.exact_language.as_deref().unwrap_or_default(),
            constraints.module_or_package.as_deref().unwrap_or_default(),
            constraints.scope_id.as_deref().unwrap_or_default(),
            constraints.qualified_name.as_deref().unwrap_or_default(),
        ];
        identity.extend(constraints.allowed_target_kinds.iter().map(String::as_str));
        identity.push(if constraints.allow_external {
            "allow_external"
        } else {
            "internal_only"
        });
        let id = self.stable_id("candidate", &identity);
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

/// Extract hard-cut universal evidence directly from the parser tree.
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
    state.add_file(root)?;
    if root.end_byte() == root.start_byte() {
        return state.builder.finish();
    }
    state.capture_parser_errors(root);
    match profile.language {
        "python" => state.extract_python(root)?,
        "go" => state.extract_go(root)?,
        "rust" => state.extract_rust(root)?,
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

#[derive(Clone)]
struct RustImplContext {
    scope_id: String,
    type_qualified_name: String,
    trait_qualified_name: Option<String>,
    owner_declaration_id: Option<String>,
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
    local_shadows: HashMap<String, HashSet<String>>,
    scope_parents: HashMap<String, String>,
    ambiguous_bindings: HashSet<(String, String)>,
    python_module_bound_names: HashSet<String>,
    python_type_bases: HashMap<String, PythonTypeBases>,
    rust_containers: HashMap<usize, DeclarationContext>,
    rust_impls: HashMap<usize, RustImplContext>,
    rust_types_by_qualified_name: HashMap<String, DeclarationContext>,
    rust_types_by_name: HashMap<String, Vec<DeclarationContext>>,
    rust_import_nodes: HashSet<usize>,
    rust_test_declarations: HashSet<String>,
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
        } else if profile.language == "go" {
            go_package_identity(path, source_file)
        } else if profile.language == "rust" {
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
            language: profile.language,
            module_or_package,
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
            python_type_bases: HashMap::new(),
            rust_containers: HashMap::new(),
            rust_impls: HashMap::new(),
            rust_types_by_qualified_name: HashMap::new(),
            rust_types_by_name: HashMap::new(),
            rust_import_nodes: HashSet::new(),
            rust_test_declarations: HashSet::new(),
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
        let graph_node_id = make_id(&[self.source_file]);
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
        self.collect_python_declarations(root, &file, None)?;
        self.collect_python_imports(root, &file)?;
        let module_bound = crate::engine::python_bound_names(root, self.source, true);
        self.python_module_bound_names.clone_from(&module_bound);
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
            self.scope_parents
                .insert(scope_id.clone(), owner.scope_id.clone());
            let context = DeclarationContext {
                fact_id,
                scope_id,
                graph_node_id,
                name,
                qualified_name,
                kind: kind.to_owned(),
                enclosing_type_qualified_name: (!is_class)
                    .then(|| class_owner.map(|owner| owner.qualified_name.clone()))
                    .flatten(),
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
        let mut bases = PythonTypeBases {
            complete: true,
            qualified_names: Vec::new(),
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
                bases.complete = false;
                continue;
            };
            let raw = self.text(target);
            let (qualifier, spelling) = split_qualified(&raw);
            if let Some(qualified_name) =
                self.python_base_qualified_name(owner, qualifier, spelling, target.start_byte())
            {
                bases.qualified_names.push(qualified_name);
            } else {
                bases.complete = false;
            }
            self.add_relationship_occurrence(
                SemanticRole::BaseType,
                CandidateRelation::Extends,
                owner,
                spelling,
                qualifier,
                target,
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
        self.walk_rust_evidence(root, &file, true)
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
            self.add_ownership(owner, &context)?;
            self.local_targets
                .entry(owner.scope_id.clone())
                .or_default()
                .insert(name.clone(), qualified_name.clone());
            self.rust_containers.insert(node.id(), context.clone());
            self.declarations.insert(node.id(), context.clone());
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
                return Ok(());
            }
            "type_item" => {
                self.add_rust_named_declaration(node, owner, "type_alias")?;
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
        let scope_id = if matches!(kind, "function" | "method") {
            let scope_id = self.builder.open_scope(
                "callable",
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
        Ok(Some(context))
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
            .insert(name, qualified_name);
        if rust_has_test_attribute(node, self.source) {
            self.rust_test_declarations.insert(context.fact_id.clone());
        }
        self.declarations.insert(node.id(), context);
        Ok(())
    }

    fn rust_declaration_context(&self, fact_id: &str) -> Option<&DeclarationContext> {
        self.declarations
            .values()
            .find(|context| context.fact_id == fact_id)
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
        if let (Some(type_context), Some(trait_node), Some(trait_qualified_name)) = (
            type_context.as_ref(),
            trait_node,
            trait_qualified_name.as_ref(),
        ) {
            self.add_rust_occurrence_candidate(
                SemanticRole::TraitBound,
                CandidateRelation::Implements,
                type_context,
                trait_node,
                Some(trait_qualified_name),
                vec!["trait".to_owned()],
                !rust_identity_is_internal(&self.module_or_package, trait_qualified_name),
            )?;
        }
        self.rust_impls.insert(node.id(), implementation.clone());
        if let Some(body) = node.child_by_field_name("body") {
            let mut cursor = body.walk();
            for child in body.children(&mut cursor).filter(|child| child.is_named()) {
                self.collect_rust_declarations(child, owner, Some(&implementation))?;
            }
        }
        Ok(())
    }

    fn rust_type_context(
        &self,
        owner: &DeclarationContext,
        raw: &str,
    ) -> Option<&DeclarationContext> {
        let raw = raw.trim().trim_start_matches(['&', '*']);
        let qualified = rust_qualify_local_path(&owner.qualified_name, raw);
        self.rust_types_by_qualified_name
            .get(&qualified)
            .or_else(|| {
                let leaf = rust_path_leaf(raw);
                self.rust_types_by_name
                    .get(leaf)
                    .filter(|contexts| contexts.len() == 1)
                    .and_then(|contexts| contexts.first())
            })
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
                self.add_rust_field(field, owner, None)?;
            } else if rust_is_type_node(field.kind()) {
                self.add_rust_field(field, owner, Some(tuple_index))?;
                tuple_index = tuple_index.saturating_add(1);
            }
        }
        Ok(())
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
            let target =
                rust_canonical_import_target(&self.rust_enclosing_module(owner), &raw_target);
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
                    exact_language: Some(self.language.to_owned()),
                    module_or_package: rust_qualified_parent(&target).map(str::to_owned),
                    scope_id: Some(owner.scope_id.clone()),
                    qualified_name: Some(target.clone()),
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
        let mut targets = Vec::new();
        collect_rust_type_nodes(node, body_start, name_id, &mut targets);
        for target in targets {
            let raw = self.text(target);
            if raw.is_empty() || rust_primitive_type(&raw) || raw == owner.name {
                continue;
            }
            let relation = if node.kind() == "trait_item"
                && rust_node_has_ancestor_before(target, node, "trait_bounds")
            {
                CandidateRelation::Extends
            } else {
                CandidateRelation::References
            };
            let role = if relation == CandidateRelation::Extends {
                SemanticRole::TraitBound
            } else {
                SemanticRole::TypeReference
            };
            self.add_rust_path_candidate(role, relation, owner, target, None)?;
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
        let (qualifier, spelling) = split_qualified(&raw);
        if spelling.is_empty() {
            return Ok(());
        }
        let binding_name = qualifier.map(qualified_binding_head).unwrap_or(spelling);
        if self.import_binding_is_ambiguous(owner, binding_name) {
            return Ok(());
        }
        let direct_binding = self
            .binding_for_occurrence(owner, binding_name, function.start_byte(), true)
            .cloned();
        let wildcard_binding = direct_binding
            .is_none()
            .then(|| {
                let wildcard_eligible = qualifier.is_some_and(|value| {
                    qualified_binding_head(value)
                        .chars()
                        .next()
                        .is_some_and(char::is_uppercase)
                }) || (qualifier.is_none()
                    && spelling.chars().next().is_some_and(char::is_uppercase));
                wildcard_eligible
                    .then(|| self.rust_wildcard_binding(owner, function.start_byte()))
                    .flatten()
            })
            .flatten()
            .cloned();
        let qualified_name = self
            .rust_call_qualified_name(owner, qualifier, spelling)
            .or_else(|| {
                wildcard_binding.is_some().then(|| {
                    qualifier.map_or_else(
                        || spelling.to_owned(),
                        |qualifier| rust_join_qualified(qualifier, spelling),
                    )
                })
            });
        let wildcard_bound = wildcard_binding.is_some();
        let binding = direct_binding.or(wildcard_binding);
        let occurrence_id = self.builder.occur(
            SemanticRole::Call,
            &owner.fact_id,
            spelling,
            qualifier,
            Some(&owner.scope_id),
            range_for_node(self.source_file, function),
        )?;
        let constraints = ResolutionConstraint {
            exact_language: Some(self.language.to_owned()),
            module_or_package: qualified_name
                .as_deref()
                .and_then(|qualified| rust_qualified_parent(qualified).map(str::to_owned))
                .or_else(|| Some(self.module_or_package.clone())),
            scope_id: Some(owner.scope_id.clone()),
            qualified_name: qualified_name.clone(),
            allowed_target_kinds: vec![
                "enum_member".to_owned(),
                "function".to_owned(),
                "method".to_owned(),
                "struct".to_owned(),
            ],
            allow_external: qualified_name.as_deref().is_some_and(|qualified| {
                (wildcard_bound || !qualifier.is_some_and(rust_deferred_owner))
                    && !rust_identity_is_internal(&self.module_or_package, qualified)
            }),
        };
        self.builder.relate(
            CandidateRelation::Calls,
            &owner.fact_id,
            Some(&occurrence_id),
            binding.as_deref(),
            spelling,
            constraints.clone(),
        )?;
        if self.rust_test_declarations.contains(&owner.fact_id) {
            self.builder.relate(
                CandidateRelation::Tests,
                &owner.fact_id,
                Some(&occurrence_id),
                binding.as_deref(),
                spelling,
                constraints,
            )?;
        }
        Ok(())
    }

    fn rust_call_qualified_name(
        &self,
        owner: &DeclarationContext,
        qualifier: Option<&str>,
        spelling: &str,
    ) -> Option<String> {
        let Some(qualifier) = qualifier else {
            return self
                .imported_target_for_occurrence(owner, spelling, 0, true)
                .cloned();
        };
        if (qualifier == "Self" || rust_receiver_is_self(qualifier))
            && let Some(enclosing) = rust_callable_owner(owner)
        {
            return Some(rust_join_qualified(enclosing, spelling));
        }
        if let Some(target) = self.local_target_for(owner, qualifier) {
            return Some(rust_join_qualified(target, spelling));
        }
        if let Some(target) = self.imported_qualified_target_for(owner, qualifier, 0, true) {
            return Some(rust_join_qualified(&target, spelling));
        }
        let first = qualified_binding_head(qualifier);
        if matches!(first, "crate" | "self" | "super") {
            return Some(rust_join_qualified(
                &rust_canonical_import_target(&self.rust_enclosing_module(owner), qualifier),
                spelling,
            ));
        }
        Some(rust_join_qualified(qualifier, spelling))
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
                exact_language: Some(self.language.to_owned()),
                module_or_package: qualified_name
                    .as_deref()
                    .and_then(|qualified| rust_qualified_parent(qualified).map(str::to_owned))
                    .or_else(|| Some(self.module_or_package.clone())),
                scope_id: Some(owner.scope_id.clone()),
                qualified_name: qualified_name.clone(),
                allowed_target_kinds: vec!["macro".to_owned()],
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
                    .and_then(|qualified| rust_qualified_parent(qualified).map(str::to_owned))
                    .or_else(|| Some(self.module_or_package.clone())),
                scope_id: Some(owner.scope_id.clone()),
                qualified_name: qualified_name.map(str::to_owned),
                allowed_target_kinds,
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
        self.collect_go_declarations(root, &file)?;
        self.walk_go_evidence(root, &file, true)
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
        let mut active = self
            .declarations
            .get(&node.id())
            .cloned()
            .unwrap_or_else(|| owner.clone());
        if node.kind() == "func_literal" && self.go_has_named_parameters(node) {
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
        if matches!(
            node.kind(),
            "function_declaration" | "method_declaration" | "func_literal"
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
                if parameter.kind() == "variadic_parameter_declaration" {
                    continue;
                }
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

    fn go_has_named_parameters(&self, declaration: Node<'_>) -> bool {
        let Some(parameters) = declaration.child_by_field_name("parameters") else {
            return false;
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
        parameter_declarations.into_iter().any(|parameter| {
            let mut cursor = parameter.walk();
            parameter
                .children_by_field_name("name", &mut cursor)
                .any(|name_node| {
                    let name = self.text(name_node);
                    !name.is_empty() && name != "_"
                })
        })
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
            "function_declaration" | "method_declaration" | "func_literal"
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
        let python_super_receiver = (self.language == "python")
            .then(|| python_super_receiver(function, self.source))
            .flatten();
        let exact_super_target = if let Some(receiver) = python_super_receiver {
            if !python_super_call_is_builtin(receiver, call, owner, self) {
                return Ok(());
            }
            let Some(target) = self.direct_python_super_target(owner, spelling) else {
                return Ok(());
            };
            Some(target)
        } else {
            None
        };
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
        let qualified_name = exact_super_target.or_else(|| {
            qualifier
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
                })
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
                allow_external: qualified_name.is_some() && python_super_receiver.is_none(),
            },
        )?;
        let _ = call_kind;
        Ok(())
    }

    fn direct_python_super_target(
        &self,
        owner: &DeclarationContext,
        spelling: &str,
    ) -> Option<String> {
        let enclosing_type = owner.enclosing_type_qualified_name.as_deref()?;
        let bases = self.python_type_bases.get(enclosing_type)?;
        let [base] = bases.qualified_names.as_slice() else {
            return None;
        };
        bases.complete.then(|| format!("{base}::{spelling}"))
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
        self.local_value_for(&self.local_bindings, owner, name)
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

fn rust_qualified_parent(path: &str) -> Option<&str> {
    path.rsplit_once("::").map(|(parent, _)| parent)
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
    let crate_name = source_root
        .and_then(|index| index.checked_sub(1))
        .and_then(|index| portable_components.get(index).copied())
        .filter(|component| !matches!(*component, "crates" | "src"))
        .unwrap_or("crate");
    let relative = if let Some(index) = source_root {
        portable_components[index.saturating_add(1)..].to_vec()
    } else {
        path.file_name()
            .and_then(|value| value.to_str())
            .map_or_else(Vec::new, |value| vec![value])
    };
    let mut components = relative.into_iter().map(str::to_owned).collect::<Vec<_>>();
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

fn rust_qualify_local_path(module: &str, raw: &str) -> String {
    let raw = raw.trim().trim_start_matches(['&', '*']);
    if raw.is_empty() {
        return module.to_owned();
    }
    if raw == "Self" {
        return module.to_owned();
    }
    rust_canonical_import_target(module, raw)
}

fn rust_qualify_imported_path(
    state: &DirectAdapterState<'_>,
    owner: &DeclarationContext,
    raw: &str,
    use_start: usize,
) -> String {
    let (qualifier, spelling) = split_qualified(raw);
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
    rust_canonical_import_target(&state.rust_enclosing_module(owner), raw)
}

fn rust_qualify_evidence_path(
    state: &DirectAdapterState<'_>,
    owner: &DeclarationContext,
    raw: &str,
    use_start: usize,
) -> Option<String> {
    let raw = raw.trim().trim_start_matches(['&', '*']);
    if raw.is_empty() || rust_primitive_type(raw) {
        return None;
    }
    let (qualifier, spelling) = split_qualified(raw);
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
    if matches!(binding_name, "crate" | "self" | "super") {
        return Some(rust_canonical_import_target(
            &state.rust_enclosing_module(owner),
            raw,
        ));
    }
    if qualifier.is_some_and(|value| {
        value.contains("::") || value.chars().next().is_some_and(char::is_lowercase)
    }) {
        return Some(raw.to_owned());
    }
    Some(rust_join_qualified(
        &state.rust_enclosing_module(owner),
        raw,
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

fn range_for_byte_span(
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
    matches!(
        raw,
        "Box"
            | "Clone"
            | "Copy"
            | "Default"
            | "DoubleEndedIterator"
            | "Drop"
            | "Eq"
            | "Err"
            | "ExactSizeIterator"
            | "Extend"
            | "Fn"
            | "FnMut"
            | "FnOnce"
            | "From"
            | "Into"
            | "Iterator"
            | "None"
            | "Ok"
            | "Option"
            | "Ord"
            | "PartialEq"
            | "PartialOrd"
            | "Result"
            | "Send"
            | "Sized"
            | "Some"
            | "String"
            | "Sync"
            | "ToOwned"
            | "ToString"
            | "Vec"
    )
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

fn python_super_call_is_builtin(
    receiver: Node<'_>,
    call: Node<'_>,
    owner: &DeclarationContext,
    state: &DirectAdapterState<'_>,
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

fn go_package_identity(path: &Path, source_file: &str) -> String {
    let source = source_file.replace('\\', "/");
    let directory = source
        .rsplit_once('/')
        .map(|(directory, _)| directory.trim_matches('/'))
        .filter(|directory| !directory.is_empty() && *directory != ".");
    directory.map_or_else(
        || {
            path.parent()
                .and_then(Path::file_name)
                .and_then(|value| value.to_str())
                .map_or_else(|| file_stem(path), str::to_owned)
        },
        str::to_owned,
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
    split_qualified_head(qualifier).0
}

fn target_kinds_for_relation(relation: CandidateRelation) -> Vec<String> {
    match relation {
        CandidateRelation::Calls | CandidateRelation::IndirectCalls | CandidateRelation::Tests => {
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
            "enum".to_owned(),
            "interface".to_owned(),
            "trait".to_owned(),
            "type_alias".to_owned(),
        ],
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
        BindingKind::Package => "package",
        BindingKind::Member => "member",
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
        CandidateRelation::References => "references",
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
