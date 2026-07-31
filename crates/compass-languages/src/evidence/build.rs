use std::collections::{HashMap, HashSet};
use std::path::Path;

use sha2::{Digest, Sha256};
use tree_sitter::Node;

use crate::{AdapterProfile, EXTRACTION_SEMANTICS_VERSION, file_stem, make_id};

use super::model::{
    AdapterIdentity, BindingFact, BindingKind, CandidateRelation, DeclarationFact,
    EvidenceDiagnostic, EvidenceRange, HierarchyConstraint, OccurrenceFact,
    ReceiverDispatchStrategy, RelationshipCandidate, ResolutionConstraint, ScopeFact,
    SemanticEvidenceBatch, SemanticRole,
};
use super::validate::{EvidenceError, EvidenceErrorCode, EvidenceLimits, validate_evidence};

/// Bounded direct-construction API shared by hard-cut language adapters.
pub struct EvidenceBuilder {
    batch: SemanticEvidenceBatch,
    source_file: String,
    limits: EvidenceLimits,
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
        let hierarchy_identity = hierarchy_constraint_identity(constraints.hierarchy.as_ref());
        let id = self.stable_id(
            "candidate",
            &[
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
                &hierarchy_identity,
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
    enclosing_type_qualified_name: Option<String>,
    runtime_nested: bool,
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
    bindings: HashMap<String, String>,
    imported_targets: HashMap<String, String>,
    local_bindings: HashMap<String, HashMap<String, String>>,
    local_targets: HashMap<String, HashMap<String, String>>,
    local_import_targets: HashMap<String, HashMap<String, String>>,
    ambiguous_bindings: HashSet<String>,
    ambiguous_local_bindings: HashMap<String, HashSet<String>>,
    graph_ids: HashSet<String>,
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
            bindings: HashMap::new(),
            imported_targets: HashMap::new(),
            local_bindings: HashMap::new(),
            local_targets: HashMap::new(),
            local_import_targets: HashMap::new(),
            ambiguous_bindings: HashSet::new(),
            ambiguous_local_bindings: HashMap::new(),
            graph_ids: HashSet::new(),
            builder: EvidenceBuilder::new(
                profile,
                format!("compass.languages.{}.universal", profile.language),
                source_file,
                EvidenceLimits::default(),
            ),
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
            enclosing_type_qualified_name: None,
            runtime_nested: false,
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
        self.walk_python_evidence(root, &file, true)
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
                    &self.stem,
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
            let fact_id = self.builder.declare(
                kind,
                &graph_node_id,
                &name,
                &qualified_name,
                Some(&self.module_or_package),
                Some(&owner.scope_id),
                range_for_node(self.source_file, name_node),
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
                enclosing_type_qualified_name: if owner.kind == "class" {
                    Some(owner.qualified_name.clone())
                } else {
                    owner.enclosing_type_qualified_name.clone()
                },
                runtime_nested: owner.runtime_nested
                    || matches!(owner.kind.as_str(), "function" | "method"),
            };
            self.add_ownership(owner, &context)?;
            self.declarations.insert(node.id(), context.clone());
            let body = node.child_by_field_name("body").unwrap_or(node);
            let mut cursor = body.walk();
            for child in body.children(&mut cursor).filter(|child| child.is_named()) {
                self.collect_python_declarations(child, &context)?;
            }
            return Ok(());
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(|child| child.is_named()) {
            self.collect_python_declarations(child, owner)?;
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
        }
        match node.kind() {
            "import_statement" | "import_from_statement" => {
                self.add_python_imports(node, &active)?;
                return Ok(());
            }
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

    fn add_python_imports(
        &mut self,
        node: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        let module = node.child_by_field_name("module_name").map(|module| {
            resolve_python_module(
                &self.module_or_package,
                &self.text(module),
                self.path.file_name().and_then(|name| name.to_str()) == Some("__init__.py"),
            )
        });
        let mut cursor = node.walk();
        for imported in node.children_by_field_name("name", &mut cursor) {
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
            if target_name.is_empty() || target_name == "*" {
                continue;
            }
            let (local, target) = if let Some(module) = module.as_deref() {
                let local = alias.unwrap_or_else(|| target_name.clone());
                let target = if module.is_empty() {
                    target_name
                } else {
                    format!("{module}.{target_name}")
                };
                (local, target)
            } else {
                let local = alias.unwrap_or_else(|| {
                    target_name.split('.').next().unwrap_or_default().to_owned()
                });
                (local, target_name)
            };
            if local.is_empty() || target.rsplit('.').next().is_none_or(str::is_empty) {
                self.builder.diagnose(
                    "unsupported_import_target",
                    Some(&owner.fact_id),
                    Some(range_for_node(self.source_file, imported)),
                    "import target could not be represented as a source-grounded binding",
                )?;
                continue;
            }
            self.add_python_import_binding(imported, owner, local, target)?;
        }
        Ok(())
    }

    fn add_python_import_binding(
        &mut self,
        imported: Node<'_>,
        owner: &DeclarationContext,
        local: String,
        target: String,
    ) -> Result<(), EvidenceError> {
        let is_reexport = owner.kind == "file"
            && self.path.file_name().and_then(|name| name.to_str()) == Some("__init__.py");
        let kind = if is_reexport {
            BindingKind::Reexport
        } else if local == target.rsplit('.').next().unwrap_or_default() {
            BindingKind::Import
        } else {
            BindingKind::ImportAlias
        };
        let range = range_for_node(self.source_file, imported);
        let binding_id = self.builder.bind(
            kind,
            &local,
            &target,
            None,
            Some(&owner.scope_id),
            range.clone(),
        )?;
        if owner.kind == "file" {
            if self
                .bindings
                .insert(local.clone(), binding_id.clone())
                .is_some()
            {
                self.ambiguous_bindings.insert(local.clone());
            }
            self.imported_targets.insert(local.clone(), target.clone());
        } else {
            if self
                .local_bindings
                .entry(owner.scope_id.clone())
                .or_default()
                .insert(local.clone(), binding_id.clone())
                .is_some()
            {
                self.ambiguous_local_bindings
                    .entry(owner.scope_id.clone())
                    .or_default()
                    .insert(local.clone());
            }
            self.local_import_targets
                .entry(owner.scope_id.clone())
                .or_default()
                .insert(local.clone(), target.clone());
        }
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
            range,
        )?;
        let target_spelling = target.rsplit('.').next().unwrap_or(&target);
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
                module_or_package: target.rsplit_once('.').map(|(module, _)| module.to_owned()),
                scope_id: Some(owner.scope_id.clone()),
                qualified_name: Some(target.clone()),
                allowed_target_kinds: vec![
                    "file".to_owned(),
                    "module".to_owned(),
                    "class".to_owned(),
                    "function".to_owned(),
                ],
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
        let mut bases = Vec::new();
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
            let target = target.unwrap_or(argument);
            let raw = self.text(target);
            let (qualifier, spelling) = split_qualified(&raw);
            if spelling.is_empty() {
                continue;
            }
            let qualified_name = self.python_base_qualified_name(owner, qualifier, spelling);
            bases.push((
                target,
                qualifier.map(str::to_owned),
                spelling.to_owned(),
                qualified_name,
            ));
        }
        let base_set_complete =
            !bases.is_empty() && bases.iter().all(|(_, _, _, qualified)| qualified.is_some());
        for (target, qualifier, spelling, qualified_name) in bases {
            self.add_relationship_occurrence_with_hierarchy(
                SemanticRole::BaseType,
                CandidateRelation::Extends,
                owner,
                &spelling,
                qualifier.as_deref(),
                target,
                qualified_name,
                Some(HierarchyConstraint::DirectBase { base_set_complete }),
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
                let fact_id = self.builder.declare(
                    kind,
                    &graph_node_id,
                    &name,
                    &qualified_name,
                    Some(&self.module_or_package),
                    Some(&file.scope_id),
                    range_for_node(self.source_file, name_node),
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
                    enclosing_type_qualified_name: None,
                    runtime_nested: false,
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
            let fact_id = self.builder.declare(
                kind,
                &graph_node_id,
                &name,
                &qualified_name,
                Some(&self.module_or_package),
                Some(&file.scope_id),
                range_for_node(self.source_file, name_node),
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
                enclosing_type_qualified_name: None,
                runtime_nested: false,
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
                    .and_then(|qualifier| self.imported_targets.get(qualifier))
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
            if self
                .bindings
                .insert(local.clone(), binding_id.clone())
                .is_some()
            {
                self.ambiguous_bindings.insert(local.clone());
            }
            self.imported_targets.insert(local.clone(), target.clone());
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
        let lookup_name = qualifier.unwrap_or(spelling);
        if self.binding_is_ambiguous(owner, lookup_name) {
            return Ok(());
        }
        if self.language == "go"
            && qualifier.is_none()
            && go_name_is_locally_bound(call, spelling, self.source)
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
            .binding_for(owner, qualifier.unwrap_or(spelling))
            .cloned();
        let hierarchy = if self.language == "python" && qualifier == Some("super()") {
            owner
                .enclosing_type_qualified_name
                .as_ref()
                .map(
                    |receiver_qualified_name| HierarchyConstraint::ReceiverDispatch {
                        receiver_qualified_name: receiver_qualified_name.clone(),
                        strategy: ReceiverDispatchStrategy::C3AfterReceiver,
                    },
                )
        } else {
            None
        };
        let qualified_name = hierarchy
            .is_none()
            .then(|| {
                qualifier
                    .and_then(|qualifier| {
                        self.local_target_for(owner, qualifier)
                            .map(|target| format!("{target}::{spelling}"))
                    })
                    .or_else(|| {
                        qualifier.and_then(|qualifier| {
                            self.imported_target_for(owner, qualifier)
                                .map(|target| format!("{target}.{spelling}"))
                        })
                    })
                    .or_else(|| self.imported_target_for(owner, spelling).cloned())
            })
            .flatten();
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
                allowed_target_kinds: if construction {
                    vec![
                        "class".to_owned(),
                        "struct".to_owned(),
                        "type_alias".to_owned(),
                    ]
                } else {
                    vec!["function".to_owned(), "method".to_owned()]
                },
                hierarchy,
                allow_external: qualified_name.is_some() && qualifier != Some("super()"),
            },
        )?;
        let _ = call_kind;
        Ok(())
    }

    fn python_base_qualified_name(
        &self,
        owner: &DeclarationContext,
        qualifier: Option<&str>,
        spelling: &str,
    ) -> Option<String> {
        if spelling.is_empty() || is_python_builtin_type(spelling) {
            return None;
        }
        match qualifier {
            None => self
                .imported_target_for(owner, spelling)
                .cloned()
                .or_else(|| {
                    owner
                        .qualified_name
                        .rsplit_once("::")
                        .map(|(parent, _)| format!("{parent}::{spelling}"))
                        .filter(|qualified| {
                            self.declarations.values().any(|declaration| {
                                declaration.kind == "class"
                                    && declaration.qualified_name == *qualified
                            })
                        })
                        .or_else(|| Some(format!("{}.{}", self.module_or_package, spelling)))
                }),
            Some(qualifier) => {
                let (root, suffix) = qualifier
                    .split_once('.')
                    .map_or((qualifier, ""), |(root, suffix)| (root, suffix));
                self.imported_target_for(owner, root).map(|target| {
                    if suffix.is_empty() {
                        format!("{target}.{spelling}")
                    } else {
                        format!("{target}.{suffix}.{spelling}")
                    }
                })
            }
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
        let lookup_name = qualifier.unwrap_or(spelling);
        if self.binding_is_ambiguous(owner, lookup_name) {
            return Ok(());
        }
        let binding = self
            .binding_for(owner, qualifier.unwrap_or(spelling))
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
                    .and_then(|qualifier| self.imported_target_for(owner, qualifier))
                    .map(|target| format!("{target}.{spelling}"))
            })
            .or_else(|| self.imported_target_for(owner, spelling).cloned());
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
                allowed_target_kinds: target_kinds_for_relation(relation),
                hierarchy,
                allow_external: qualified_name.is_some(),
            },
        )?;
        Ok(())
    }

    fn binding_for(&self, owner: &DeclarationContext, name: &str) -> Option<&String> {
        self.local_bindings
            .get(&owner.scope_id)
            .and_then(|bindings| bindings.get(name))
            .or_else(|| self.bindings.get(name))
    }

    fn local_target_for(&self, owner: &DeclarationContext, name: &str) -> Option<&String> {
        self.local_targets
            .get(&owner.scope_id)
            .and_then(|targets| targets.get(name))
    }

    fn imported_target_for(&self, owner: &DeclarationContext, name: &str) -> Option<&String> {
        self.local_import_targets
            .get(&owner.scope_id)
            .and_then(|targets| targets.get(name))
            .or_else(|| self.imported_targets.get(name))
    }

    fn binding_is_ambiguous(&self, owner: &DeclarationContext, name: &str) -> bool {
        if self
            .local_bindings
            .get(&owner.scope_id)
            .is_some_and(|bindings| bindings.contains_key(name))
        {
            return self
                .ambiguous_local_bindings
                .get(&owner.scope_id)
                .is_some_and(|bindings| bindings.contains(name));
        }
        self.ambiguous_bindings.contains(name)
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
                exact_target_declaration_id: (self.language == "python" && child.runtime_nested)
                    .then(|| child.fact_id.clone()),
                exact_language: Some(self.language.to_owned()),
                module_or_package: Some(self.module_or_package.clone()),
                scope_id: Some(owner.scope_id.clone()),
                qualified_name: Some(child.qualified_name.clone()),
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

fn go_name_is_locally_bound(call: Node<'_>, spelling: &str, source: &[u8]) -> bool {
    let mut ancestor = call.parent();
    while let Some(node) = ancestor {
        if matches!(
            node.kind(),
            "function_declaration" | "method_declaration" | "func_literal"
        ) {
            let mut binding_nodes = Vec::new();
            for field in ["receiver", "parameters"] {
                if let Some(parameters) = node.child_by_field_name(field) {
                    binding_nodes.push(parameters);
                }
            }
            collect_go_prior_binding_nodes(node, call.start_byte(), &mut binding_nodes);
            return binding_nodes.into_iter().any(|binding| {
                let mut identifiers = Vec::new();
                collect_named_targets(binding, &["identifier"], &mut identifiers);
                identifiers
                    .into_iter()
                    .any(|identifier| identifier.utf8_text(source).ok() == Some(spelling))
            });
        }
        ancestor = node.parent();
    }
    false
}

fn collect_go_prior_binding_nodes<'tree>(
    node: Node<'tree>,
    before: usize,
    output: &mut Vec<Node<'tree>>,
) {
    if node.start_byte() >= before {
        return;
    }
    match node.kind() {
        "short_var_declaration" => {
            if node.end_byte() <= before
                && let Some(left) = node.child_by_field_name("left")
            {
                output.push(left);
            }
            return;
        }
        "var_spec" => {
            if node.end_byte() <= before {
                let type_node = node.child_by_field_name("type");
                let value_node = node.child_by_field_name("value");
                let mut cursor = node.walk();
                output.extend(node.children(&mut cursor).filter(|child| {
                    child.is_named() && Some(*child) != type_node && Some(*child) != value_node
                }));
            }
            return;
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_go_prior_binding_nodes(child, before, output);
    }
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

fn target_kinds_for_relation(relation: CandidateRelation) -> Vec<String> {
    match relation {
        CandidateRelation::Calls => vec!["function".to_owned(), "method".to_owned()],
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

fn hierarchy_constraint_identity(constraint: Option<&HierarchyConstraint>) -> String {
    match constraint {
        None => String::new(),
        Some(HierarchyConstraint::DirectBase { base_set_complete }) => {
            format!("direct_base:{base_set_complete}")
        }
        Some(HierarchyConstraint::ReceiverDispatch {
            receiver_qualified_name,
            strategy: ReceiverDispatchStrategy::C3AfterReceiver,
        }) => format!("receiver_dispatch:c3_after_receiver:{receiver_qualified_name}"),
    }
}
