//! Shared AST-first traversal for the language-specific universal producers.
//!
//! The four grammars have different surface syntax, but their project-neutral
//! evidence boundary is the same: declarations and lexical scopes first,
//! followed by exact import/call/type occurrences.  The producer deliberately
//! leaves target selection to `compass-resolve`; a terminal spelling is never
//! treated as an identity and every candidate is constrained to its source
//! language.

use std::collections::{BTreeMap, BTreeSet};
use std::marker::PhantomData;
use std::path::Path;

use tree_sitter::Node;

use super::build::{EvidenceBuilder, range_for_byte_span, range_for_file, range_for_node};
use super::model::{
    BindingKind, CandidateRelation, EvidenceRange, LanguageCapability, ResolutionConstraint,
    SemanticEvidenceBatch, SemanticRole, SymbolNamespace,
};
use super::validate::{EvidenceError, EvidenceErrorCode, EvidenceLimits};
use crate::{UniversalEvidenceRegistry, file_stem, make_id};

const MAX_TRAVERSAL_DEPTH: usize = 256;
const MAX_TEXT_BYTES: usize = 4_096;

/// Language-specific policy for the shared AST traversal.
///
/// The traversal owns the evidence contract and bounded bookkeeping; each
/// language module supplies only its syntax profile and any language-specific
/// source supplement. This keeps the production entry points separate without
/// duplicating validation and relationship machinery.
pub(super) trait LanguageProfile: Sized {
    const LANGUAGE: &'static str;

    fn package_name(_source: &[u8]) -> Option<String> {
        None
    }

    fn declaration_kind(kind: &str) -> Option<&'static str> {
        shared_declaration_kind(kind)
    }

    fn declaration_lookup_name(name: &str) -> String {
        name.to_owned()
    }

    fn emits_module_declarations() -> bool {
        false
    }

    fn has_source_supplement(_declaration_count: usize) -> bool {
        false
    }

    fn collect_source_supplement<'source>(
        _state: &mut State<'source, Self>,
    ) -> Result<(), EvidenceError> {
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub(super) struct Decl {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) qualified: String,
    pub(super) kind: String,
    pub(super) body_scope_id: String,
    pub(super) start: usize,
    pub(super) end: usize,
}

#[derive(Clone, Debug)]
struct Import {
    spelling: String,
    target: String,
}

pub(super) struct State<'source, P: LanguageProfile> {
    pub(super) source: &'source [u8],
    pub(super) source_file: &'source str,
    builder: EvidenceBuilder,
    file_id: String,
    pub(super) file_scope_id: String,
    namespace: String,
    pub(super) declarations: Vec<Decl>,
    by_node: BTreeMap<usize, usize>,
    by_terminal: BTreeMap<String, Vec<usize>>,
    by_qualified: BTreeMap<String, Vec<usize>>,
    name_ranges: BTreeSet<(usize, usize)>,
    imports: Vec<Import>,
    module_targets: BTreeSet<String>,
    emitted: BTreeSet<(SemanticRole, usize, usize, String)>,
    occurrence_ids: BTreeMap<(SemanticRole, usize, usize, String), String>,
    _profile: PhantomData<P>,
}

pub(super) fn emit_tree_evidence<P: LanguageProfile>(
    path: &Path,
    source_file: &str,
    source: &[u8],
    root: Node<'_>,
) -> Result<SemanticEvidenceBatch, EvidenceError> {
    let pipeline = UniversalEvidenceRegistry::pipeline(P::LANGUAGE).ok_or_else(|| {
        EvidenceError::new(
            EvidenceErrorCode::InvalidPipeline,
            format!(
                "{} universal evidence pipeline is not registered",
                P::LANGUAGE
            ),
        )
    })?;
    let file_range = range_for_file(source_file, source);
    let mut builder = EvidenceBuilder::new(
        pipeline,
        format!("compass.languages.{}.universal", P::LANGUAGE),
        source_file,
        EvidenceLimits::default(),
    );
    let file_graph_id = make_id(&[source_file]);
    let file_id = builder.declare(
        "file",
        &file_graph_id,
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(source_file),
        source_file,
        Some(&file_stem(Path::new(source_file))),
        None,
        file_range.clone(),
    )?;
    // A module scope is the schema's zero-width-safe file scope. It can
    // represent an empty/trivia-only source while retaining the file's exact
    // inventory range.
    let file_scope_id = builder.open_scope("module", Some(&file_id), None, file_range)?;
    if root.end_byte() == root.start_byte() {
        return builder.finish();
    }

    let namespace = P::package_name(source).unwrap_or_default();
    let mut state = State {
        source,
        source_file,
        builder,
        file_id,
        file_scope_id: file_scope_id.clone(),
        namespace,
        declarations: Vec::new(),
        by_node: BTreeMap::new(),
        by_terminal: BTreeMap::new(),
        by_qualified: BTreeMap::new(),
        name_ranges: BTreeSet::new(),
        imports: Vec::new(),
        module_targets: BTreeSet::new(),
        emitted: BTreeSet::new(),
        occurrence_ids: BTreeMap::new(),
        _profile: PhantomData,
    };
    if std::str::from_utf8(source).is_err() {
        state.builder.diagnose(
            "invalid_utf8",
            None,
            Some(range_for_file(source_file, source)),
            "source is not valid UTF-8; text-derived evidence is omitted where decoding is unsafe",
        )?;
    }
    let root_scope = state.add_namespace(root)?;
    state.collect_declarations(root, None, &root_scope, 0)?;
    if P::has_source_supplement(state.declarations.len()) {
        P::collect_source_supplement(&mut state)?;
    }
    state.collect_imports(root, 0)?;
    state.collect_semantics(root, 0)?;
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

impl<'source, P: LanguageProfile> State<'source, P> {
    fn add_namespace(&mut self, root: Node<'_>) -> Result<String, EvidenceError> {
        if self.namespace.is_empty() || !self.supports(LanguageCapability::Namespaces) {
            return Ok(self.file_scope_id.clone());
        }
        let graph_id = make_id(&[P::LANGUAGE, "namespace", &self.namespace]);
        let declaration_id = self.builder.declare_with_namespace(
            "namespace",
            &graph_id,
            &self.namespace,
            &self.namespace,
            Some(&self.namespace),
            Some(&self.file_scope_id),
            Some(SymbolNamespace::Namespace),
            range_for_node(self.source_file, root),
        )?;
        let scope_id = self.builder.open_scope(
            "namespace",
            Some(&declaration_id),
            Some(&self.file_scope_id),
            range_for_node(self.source_file, root),
        )?;
        self.builder.relate(
            CandidateRelation::Owns,
            &self.file_id,
            None,
            None,
            &self.namespace,
            ResolutionConstraint {
                exact_target_declaration_id: Some(declaration_id),
                exact_language: Some(P::LANGUAGE.to_owned()),
                ..ResolutionConstraint::default()
            },
        )?;
        Ok(scope_id)
    }

    fn collect_declarations(
        &mut self,
        node: Node<'_>,
        parent_decl: Option<usize>,
        parent_scope: &str,
        depth: usize,
    ) -> Result<(), EvidenceError> {
        if depth > MAX_TRAVERSAL_DEPTH {
            self.depth_diagnostic(node)?;
            return Ok(());
        }
        let mut owner = parent_decl;
        let mut scope = parent_scope.to_owned();
        let prefix = parent_decl
            .and_then(|index| self.declarations.get(index))
            .map(|decl| decl.qualified.clone())
            .unwrap_or_else(|| self.namespace.clone());

        if let Some(kind) = P::declaration_kind(node.kind())
            && let Some(name_node) = declaration_name(node)
        {
            let name = self.text(name_node);
            let lookup_name = P::declaration_lookup_name(&name);
            // The Dart grammar exposes a method signature as the declaration
            // name child (for example ``clearLibraryContext()``).  Calls and
            // Analyzer source evidence carry the base name only; retaining
            // the punctuation makes an otherwise exact lexical target look
            // unresolved.  Preserve the parser spelling for declaration
            // identity and index a separate base-name alias for lookup; this
            // keeps overloads and stable declaration IDs distinct.
            if valid_name(&lookup_name)
                && !self
                    .name_ranges
                    .contains(&(name_node.start_byte(), name_node.end_byte()))
            {
                let qualified = join_name(&prefix, &name);
                let key = (node.start_byte(), node.end_byte(), name.clone());
                if !self
                    .declarations
                    .iter()
                    .any(|decl| (decl.start, decl.end) == (key.0, key.1) && decl.name == key.2)
                {
                    let graph_id = make_id(&[
                        self.source_file,
                        P::LANGUAGE,
                        kind,
                        &qualified,
                        &node.start_byte().to_string(),
                        &node.end_byte().to_string(),
                    ]);
                    let declaration_id = self.builder.declare(
                        kind,
                        &graph_id,
                        &name,
                        &qualified,
                        if self.namespace.is_empty() {
                            None
                        } else {
                            Some(&self.namespace)
                        },
                        Some(parent_scope),
                        range_for_node(self.source_file, node),
                    )?;
                    let opens_scope = opens_scope(kind);
                    let body_scope_id = if opens_scope {
                        self.builder.open_scope(
                            scope_kind(kind),
                            Some(&declaration_id),
                            Some(parent_scope),
                            range_for_node(self.source_file, node),
                        )?
                    } else {
                        parent_scope.to_owned()
                    };
                    let source_id = parent_decl
                        .and_then(|index| self.declarations.get(index))
                        .map_or(self.file_id.as_str(), |decl| decl.id.as_str())
                        .to_owned();
                    self.builder.relate(
                        CandidateRelation::Owns,
                        &source_id,
                        None,
                        None,
                        &name,
                        ResolutionConstraint {
                            exact_target_declaration_id: Some(declaration_id.clone()),
                            exact_language: Some(P::LANGUAGE.to_owned()),
                            ..ResolutionConstraint::default()
                        },
                    )?;
                    let index = self.declarations.len();
                    let decl = Decl {
                        id: declaration_id,
                        name: name.clone(),
                        qualified: qualified.clone(),
                        kind: kind.to_owned(),
                        body_scope_id: body_scope_id.clone(),
                        start: node.start_byte(),
                        end: node.end_byte(),
                    };
                    self.name_ranges
                        .insert((name_node.start_byte(), name_node.end_byte()));
                    self.by_node.insert(node.id(), index);
                    self.by_terminal.entry(name).or_default().push(index);
                    if lookup_name != decl.name {
                        self.by_terminal.entry(lookup_name).or_default().push(index);
                    }
                    self.by_qualified.entry(qualified).or_default().push(index);
                    self.declarations.push(decl);
                    if opens_scope {
                        owner = Some(index);
                        scope = body_scope_id;
                    }
                }
            }
        }

        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.collect_declarations(child, owner, &scope, depth.saturating_add(1))?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn add_source_declaration(
        &mut self,
        kind: &str,
        name: &str,
        start: usize,
        end: usize,
        name_start: usize,
        name_end: usize,
        parent: Option<usize>,
        parent_scope: &str,
    ) -> Result<Option<usize>, EvidenceError> {
        if !valid_name(name)
            || self
                .declarations
                .iter()
                .any(|decl| decl.start == start && decl.end == end && decl.name == name)
        {
            return Ok(None);
        }
        let prefix = parent
            .and_then(|index| self.declarations.get(index))
            .map(|decl| decl.qualified.as_str())
            .unwrap_or(self.namespace.as_str());
        let qualified = join_name(prefix, name);
        let graph_id = make_id(&[
            self.source_file,
            P::LANGUAGE,
            kind,
            &qualified,
            &start.to_string(),
            &end.to_string(),
        ]);
        let declaration_id = self.builder.declare(
            kind,
            &graph_id,
            name,
            &qualified,
            (!self.namespace.is_empty()).then_some(self.namespace.as_str()),
            Some(parent_scope),
            range_for_byte_span(self.source_file, self.source, start, end),
        )?;
        let body_scope_id = self.builder.open_scope(
            scope_kind(kind),
            Some(&declaration_id),
            Some(parent_scope),
            range_for_byte_span(self.source_file, self.source, start, end),
        )?;
        let owner_id = parent
            .and_then(|index| self.declarations.get(index))
            .map_or(self.file_id.as_str(), |decl| decl.id.as_str())
            .to_owned();
        self.builder.relate(
            CandidateRelation::Owns,
            &owner_id,
            None,
            None,
            name,
            ResolutionConstraint {
                exact_target_declaration_id: Some(declaration_id.clone()),
                exact_language: Some(P::LANGUAGE.to_owned()),
                ..ResolutionConstraint::default()
            },
        )?;
        let index = self.declarations.len();
        self.declarations.push(Decl {
            id: declaration_id,
            name: name.to_owned(),
            qualified: qualified.clone(),
            kind: kind.to_owned(),
            body_scope_id,
            start,
            end,
        });
        self.name_ranges.insert((name_start, name_end));
        self.by_terminal
            .entry(name.to_owned())
            .or_default()
            .push(index);
        self.by_qualified.entry(qualified).or_default().push(index);
        Ok(Some(index))
    }

    pub(super) fn emit_source_calls(
        &mut self,
        line_start: usize,
        line_end: usize,
        owner: usize,
    ) -> Result<(), EvidenceError> {
        let bytes = self.source.get(line_start..line_end).unwrap_or_default();
        let mut index = 0_usize;
        while index < bytes.len() {
            if !is_identifier_start(bytes[index]) {
                index = index.saturating_add(1);
                continue;
            }
            let start = index;
            index = index.saturating_add(1);
            while index < bytes.len() && is_identifier_continue(bytes[index]) {
                index = index.saturating_add(1);
            }
            let end = index;
            while index < bytes.len() && bytes[index].is_ascii_whitespace() {
                index = index.saturating_add(1);
            }
            if bytes.get(index) != Some(&b'(') {
                continue;
            }
            let spelling = String::from_utf8_lossy(&bytes[start..end]).into_owned();
            if matches!(
                spelling.as_str(),
                "if" | "for" | "while" | "switch" | "catch" | "return" | "new" | "super" | "this"
            ) || self
                .name_ranges
                .contains(&(line_start + start, line_start + end))
            {
                continue;
            }
            let qualifier = (start > 0 && bytes[start - 1] == b'.').then(|| {
                let mut qualifier_start = start.saturating_sub(1);
                while qualifier_start > 0 && is_identifier_continue(bytes[qualifier_start - 1]) {
                    qualifier_start = qualifier_start.saturating_sub(1);
                }
                String::from_utf8_lossy(&bytes[qualifier_start..start - 1]).into_owned()
            });
            self.emit_call_site(
                Some(owner),
                &spelling,
                qualifier.as_deref(),
                range_for_byte_span(
                    self.source_file,
                    self.source,
                    line_start + start,
                    line_start + end,
                ),
                false,
            )?;
        }
        Ok(())
    }

    fn collect_imports(&mut self, node: Node<'_>, depth: usize) -> Result<(), EvidenceError> {
        if depth > MAX_TRAVERSAL_DEPTH {
            self.depth_diagnostic(node)?;
            return Ok(());
        }
        let statement = self.text(node);
        if is_import_node(node.kind())
            && let Some((target, alias, reexport)) = parse_import(&statement)
            && !target.is_empty()
        {
            let has_alias = alias.is_some();
            let spelling = alias.unwrap_or_else(|| terminal(&target).to_owned());
            if !spelling.is_empty() {
                let owner = self.owner_for(node.start_byte());
                let owner_scope = self
                    .declaration_for(owner)
                    .map_or(self.file_scope_id.as_str(), |decl| {
                        decl.body_scope_id.as_str()
                    })
                    .to_owned();
                // Swift's pre-universal extractor published imported modules
                // as source-anchored module nodes. Keep that established
                // Vapor/framework identity on the evidence route while the
                // binding itself remains an exact, language-constrained
                // import candidate.
                if P::emits_module_declarations() && self.module_targets.insert(target.clone()) {
                    let module_id = make_id(&[self.source_file, P::LANGUAGE, "module", &target]);
                    let module_declaration = self.builder.declare(
                        "module",
                        &module_id,
                        &spelling,
                        &target,
                        None,
                        Some(&self.file_scope_id),
                        range_for_node(self.source_file, node),
                    )?;
                    self.builder.relate(
                        CandidateRelation::Owns,
                        &self.file_id,
                        None,
                        None,
                        &spelling,
                        ResolutionConstraint {
                            exact_target_declaration_id: Some(module_declaration),
                            exact_language: Some(P::LANGUAGE.to_owned()),
                            ..ResolutionConstraint::default()
                        },
                    )?;
                }
                let binding_id = self.builder.bind_with_identity(
                    if reexport {
                        BindingKind::Reexport
                    } else if has_alias {
                        BindingKind::ImportAlias
                    } else {
                        BindingKind::Import
                    },
                    &spelling,
                    &target,
                    None,
                    Some(&owner_scope),
                    None,
                    false,
                    range_for_node(self.source_file, node),
                )?;
                let owner_id = self.owner_id(owner);
                let role = if reexport {
                    SemanticRole::Reexport
                } else {
                    SemanticRole::Import
                };
                let occurrence_id = self.emit_occurrence(
                    role,
                    &owner_id,
                    &spelling,
                    qualifier_for(&target),
                    Some(&owner_scope),
                    range_for_node(self.source_file, node),
                )?;
                let relation = if reexport {
                    CandidateRelation::Reexports
                } else {
                    CandidateRelation::Imports
                };
                self.builder.relate(
                    relation,
                    &owner_id,
                    Some(&occurrence_id),
                    Some(&binding_id),
                    &spelling,
                    ResolutionConstraint {
                        exact_language: Some(P::LANGUAGE.to_owned()),
                        qualified_name: Some(target.clone()),
                        allow_external: true,
                        ..ResolutionConstraint::default()
                    },
                )?;
                self.imports.push(Import { spelling, target });
            }
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.collect_imports(child, depth.saturating_add(1))?;
        }
        Ok(())
    }

    fn collect_semantics(&mut self, node: Node<'_>, depth: usize) -> Result<(), EvidenceError> {
        if depth > MAX_TRAVERSAL_DEPTH {
            self.depth_diagnostic(node)?;
            return Ok(());
        }
        if is_call_node(node.kind()) || self.is_identifier_call(node) {
            self.emit_call(node)?;
        }
        if is_type_leaf(node.kind()) {
            self.emit_type_reference(node)?;
        }
        if self.supports(LanguageCapability::Members) && is_member_node(node.kind()) {
            self.emit_member_access(node)?;
        }
        if self.supports(LanguageCapability::Decorators) && is_decorator_node(node.kind()) {
            self.emit_decorator(node)?;
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.collect_semantics(child, depth.saturating_add(1))?;
        }
        Ok(())
    }

    fn emit_call(&mut self, node: Node<'_>) -> Result<(), EvidenceError> {
        let callee = if is_call_node(node.kind()) {
            call_callee(node)
        } else {
            Some(node)
        };
        let Some(callee) = callee else {
            return Ok(());
        };
        let raw = self.text(callee);
        let (qualifier, spelling) = split_qualified(&raw);
        if !valid_name(&spelling) {
            return Ok(());
        }
        self.emit_call_site(
            self.owner_for(node.start_byte()),
            &spelling,
            qualifier.as_deref(),
            range_for_node(self.source_file, callee),
            node.kind().contains("constructor"),
        )
    }

    fn emit_call_site(
        &mut self,
        owner: Option<usize>,
        spelling: &str,
        qualifier: Option<&str>,
        range: EvidenceRange,
        constructor_node: bool,
    ) -> Result<(), EvidenceError> {
        let owner_id = self.owner_id(owner);
        let owner_scope = self.owner_scope(owner);
        let constructor =
            spelling.chars().next().is_some_and(char::is_uppercase) || constructor_node;
        let role = if constructor {
            SemanticRole::Construction
        } else {
            SemanticRole::Call
        };
        let relation = if constructor {
            CandidateRelation::Constructs
        } else {
            CandidateRelation::Calls
        };
        let occurrence_id = self.emit_occurrence(
            role,
            &owner_id,
            spelling,
            qualifier,
            Some(&owner_scope),
            range,
        )?;
        let exact = self.resolve_local(spelling, qualifier);
        let imported_target = (qualifier.is_none())
            .then(|| {
                self.imports
                    .iter()
                    .find(|import| import.spelling == spelling)
                    .map(|import| import.target.clone())
            })
            .flatten();
        let qualified_name = exact
            .and_then(|index| {
                self.declarations
                    .get(index)
                    .map(|decl| decl.qualified.clone())
            })
            .or(imported_target);
        let mut allowed = if constructor {
            vec![
                "class".to_owned(),
                "struct".to_owned(),
                "enum".to_owned(),
                "constructor".to_owned(),
            ]
        } else {
            vec![
                "function".to_owned(),
                "method".to_owned(),
                "constructor".to_owned(),
            ]
        };
        allowed.sort_unstable();
        self.builder.relate(
            relation,
            &owner_id,
            Some(&occurrence_id),
            None,
            spelling,
            ResolutionConstraint {
                exact_target_declaration_id: exact.map(|index| self.declarations[index].id.clone()),
                exact_language: Some(P::LANGUAGE.to_owned()),
                qualified_name,
                allowed_target_kinds: allowed,
                allow_external: exact.is_none(),
                ..ResolutionConstraint::default()
            },
        )?;
        Ok(())
    }

    fn emit_type_reference(&mut self, node: Node<'_>) -> Result<(), EvidenceError> {
        if self
            .name_ranges
            .contains(&(node.start_byte(), node.end_byte()))
        {
            return Ok(());
        }
        let raw = self.text(node);
        let (qualifier, spelling) = split_qualified(&raw);
        if !valid_name(&spelling) || spelling.len() > 256 {
            return Ok(());
        }
        let owner = self.owner_for(node.start_byte());
        let owner_id = self.owner_id(owner);
        let owner_scope = self.owner_scope(owner);
        let base = is_base_context(node);
        let role = if base {
            SemanticRole::BaseType
        } else {
            SemanticRole::TypeReference
        };
        let relation = if base {
            if self.declaration_for(owner).is_some_and(|decl| {
                decl.kind == "interface" || decl.kind == "trait" || decl.kind == "protocol"
            }) {
                CandidateRelation::Implements
            } else {
                CandidateRelation::Extends
            }
        } else {
            CandidateRelation::References
        };
        let occurrence_id = self.emit_occurrence(
            role,
            &owner_id,
            &spelling,
            qualifier.as_deref(),
            Some(&owner_scope),
            range_for_node(self.source_file, node),
        )?;
        let exact = self.resolve_local(&spelling, qualifier.as_deref());
        self.builder.relate(
            relation,
            &owner_id,
            Some(&occurrence_id),
            None,
            &spelling,
            ResolutionConstraint {
                exact_target_declaration_id: exact.map(|index| self.declarations[index].id.clone()),
                exact_language: Some(P::LANGUAGE.to_owned()),
                qualified_name: qualifier
                    .as_ref()
                    .map(|prefix| format!("{prefix}.{spelling}")),
                allowed_target_kinds: vec![
                    "class".to_owned(),
                    "enum".to_owned(),
                    "interface".to_owned(),
                    "protocol".to_owned(),
                    "struct".to_owned(),
                    "trait".to_owned(),
                    "type_alias".to_owned(),
                ],
                allow_external: exact.is_none(),
                ..ResolutionConstraint::default()
            },
        )?;
        Ok(())
    }

    fn emit_member_access(&mut self, node: Node<'_>) -> Result<(), EvidenceError> {
        let raw = self.text(node);
        let Some((qualifier, spelling)) = raw.rsplit_once('.') else {
            return Ok(());
        };
        let spelling = spelling
            .trim_matches(|character: char| !character.is_ascii_alphanumeric() && character != '_');
        let qualifier = qualifier.trim();
        if !valid_name(spelling) || qualifier.is_empty() {
            return Ok(());
        }
        let owner = self.owner_for(node.start_byte());
        let owner_id = self.owner_id(owner);
        let owner_scope = self.owner_scope(owner);
        let occurrence_id = self.emit_occurrence(
            SemanticRole::MemberAccess,
            &owner_id,
            spelling,
            Some(qualifier),
            Some(&owner_scope),
            range_for_node(self.source_file, node),
        )?;
        self.builder.relate(
            CandidateRelation::AccessesMember,
            &owner_id,
            Some(&occurrence_id),
            None,
            spelling,
            ResolutionConstraint {
                exact_language: Some(P::LANGUAGE.to_owned()),
                qualified_name: Some(format!("{qualifier}.{spelling}")),
                allowed_target_kinds: vec![
                    "field".to_owned(),
                    "property".to_owned(),
                    "method".to_owned(),
                ],
                allow_external: true,
                ..ResolutionConstraint::default()
            },
        )?;
        Ok(())
    }

    fn emit_decorator(&mut self, node: Node<'_>) -> Result<(), EvidenceError> {
        let raw = self.text(node);
        let spelling = raw
            .trim_start_matches(['@', '#', '['])
            .split(['(', '[', ' ', '\n', '\r', ']'])
            .next()
            .unwrap_or_default()
            .trim();
        if !valid_name(spelling) {
            return Ok(());
        }
        let owner = self.owner_for(node.start_byte());
        let owner_id = self.owner_id(owner);
        let owner_scope = self.owner_scope(owner);
        let occurrence_id = self.emit_occurrence(
            SemanticRole::Decorator,
            &owner_id,
            spelling,
            None,
            Some(&owner_scope),
            range_for_node(self.source_file, node),
        )?;
        self.builder.relate(
            CandidateRelation::Decorates,
            &owner_id,
            Some(&occurrence_id),
            None,
            spelling,
            ResolutionConstraint {
                exact_language: Some(P::LANGUAGE.to_owned()),
                allow_external: true,
                ..ResolutionConstraint::default()
            },
        )?;
        Ok(())
    }

    fn emit_occurrence(
        &mut self,
        role: SemanticRole,
        owner_id: &str,
        spelling: &str,
        qualifier: Option<&str>,
        scope_id: Option<&str>,
        range: EvidenceRange,
    ) -> Result<String, EvidenceError> {
        let key = (
            role,
            range.start_byte as usize,
            range.end_byte as usize,
            spelling.to_owned(),
        );
        if !self.emitted.insert(key.clone()) {
            return Ok(self.occurrence_ids.get(&key).cloned().unwrap_or_default());
        }
        let id = self
            .builder
            .occur(role, owner_id, spelling, qualifier, scope_id, range)?;
        self.occurrence_ids.insert(key, id.clone());
        Ok(id)
    }

    fn owner_for(&self, byte: usize) -> Option<usize> {
        self.declarations
            .iter()
            .enumerate()
            .filter(|(_, decl)| decl.start <= byte && byte < decl.end)
            .max_by_key(|(_, decl)| decl.start)
            .map(|(index, _)| index)
    }

    fn owner_id(&self, owner: Option<usize>) -> String {
        owner
            .and_then(|index| self.declarations.get(index))
            .map_or_else(|| self.file_id.clone(), |decl| decl.id.clone())
    }

    fn owner_scope(&self, owner: Option<usize>) -> String {
        owner
            .and_then(|index| self.declarations.get(index))
            .map_or_else(
                || self.file_scope_id.clone(),
                |decl| decl.body_scope_id.clone(),
            )
    }

    fn declaration_for(&self, owner: Option<usize>) -> Option<&Decl> {
        owner.and_then(|index| self.declarations.get(index))
    }

    fn resolve_local(&self, spelling: &str, qualifier: Option<&str>) -> Option<usize> {
        let values = if let Some(qualifier) = qualifier {
            self.by_qualified.get(&format!("{qualifier}.{spelling}"))
        } else {
            self.by_terminal.get(spelling)
        }?;
        (values.len() == 1).then_some(values[0])
    }

    fn supports(&self, capability: LanguageCapability) -> bool {
        UniversalEvidenceRegistry::pipeline(P::LANGUAGE)
            .is_some_and(|pipeline| pipeline.producer.capabilities.contains(&capability))
    }

    fn is_identifier_call(&self, node: Node<'_>) -> bool {
        if !matches!(
            node.kind(),
            "identifier" | "simple_identifier" | "field_identifier"
        ) || self
            .name_ranges
            .contains(&(node.start_byte(), node.end_byte()))
        {
            return false;
        }
        let mut end = node.end_byte();
        while self.source.get(end).is_some_and(u8::is_ascii_whitespace) {
            end = end.saturating_add(1);
        }
        self.source.get(end) == Some(&b'(')
    }

    fn text(&self, node: Node<'_>) -> String {
        self.source
            .get(node.start_byte()..node.end_byte())
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .map(|text| text.chars().take(MAX_TEXT_BYTES).collect())
            .unwrap_or_default()
    }

    fn depth_diagnostic(&mut self, node: Node<'_>) -> Result<(), EvidenceError> {
        self.builder.diagnose(
            "traversal_depth_limit",
            None,
            Some(range_for_node(self.source_file, node)),
            "parser tree exceeded the bounded universal evidence traversal depth",
        )
    }
}

pub(super) fn shared_declaration_kind(kind: &str) -> Option<&'static str> {
    if matches!(
        kind,
        "source_file"
            | "program"
            | "compilation_unit"
            | "translation_unit"
            | "package_declaration"
            | "package_clause"
            | "namespace_declaration"
            | "library_directive"
            | "import_declaration"
            | "import_statement"
            | "import_directive"
            | "import_or_export"
            | "export_directive"
    ) {
        return None;
    }
    let lower = kind.to_ascii_lowercase();
    if lower.contains("protocol") {
        return Some("protocol");
    }
    if lower.contains("interface") {
        return Some("interface");
    }
    if lower.contains("trait") {
        return Some("trait");
    }
    if lower.contains("enum") {
        return Some("enum");
    }
    if lower.contains("struct") {
        return Some("struct");
    }
    if lower.contains("record") {
        return Some("record");
    }
    if lower.contains("class") {
        return Some("class");
    }
    if matches!(
        lower.as_str(),
        "object_definition" | "module_definition" | "extension_declaration"
    ) || lower.ends_with("_object_definition")
    {
        return Some("module");
    }
    if lower.contains("type_alias") || lower.contains("type_definition") {
        return Some("type_alias");
    }
    if lower.contains("initializer") || lower.contains("constructor") || lower == "init_declaration"
    {
        return Some("constructor");
    }
    if lower.contains("deinitializer") || lower.contains("deinit") {
        return Some("method");
    }
    if lower.contains("subscript") {
        return Some("method");
    }
    if lower.contains("method") {
        return Some("method");
    }
    if lower.contains("function")
        || lower == "function_declaration"
        || lower == "function_definition"
    {
        return Some("function");
    }
    if lower.contains("property") || lower.contains("field") {
        return Some("field");
    }
    None
}

fn declaration_name(node: Node<'_>) -> Option<Node<'_>> {
    for field in ["name", "identifier", "declarator"] {
        if let Some(child) = node.child_by_field_name(field) {
            return Some(child);
        }
    }
    let mut cursor = node.walk();
    node.named_children(&mut cursor).find(|child| {
        matches!(
            child.kind(),
            "identifier"
                | "type_identifier"
                | "simple_identifier"
                | "field_identifier"
                | "constant_identifier"
                | "name"
        )
    })
}

pub(super) fn valid_name(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && value.len() <= 512
        && value
            .chars()
            .all(|character| character.is_alphanumeric() || matches!(character, '_' | '$' | '`'))
}

fn opens_scope(kind: &str) -> bool {
    matches!(
        kind,
        "class"
            | "struct"
            | "record"
            | "enum"
            | "protocol"
            | "interface"
            | "trait"
            | "module"
            | "function"
            | "method"
            | "constructor"
    )
}

fn scope_kind(kind: &str) -> &'static str {
    if matches!(kind, "function" | "method" | "constructor") {
        "function"
    } else {
        "type"
    }
}

fn join_name(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_owned()
    } else {
        format!("{prefix}.{name}")
    }
}

pub(super) fn package_name_from_source(source: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(source).ok()?;
    for line in text.lines().take(128) {
        let line = line.trim();
        let Some(value) = line
            .strip_prefix("package ")
            .or_else(|| line.strip_prefix("namespace "))
        else {
            continue;
        };
        let value = value
            .trim()
            .trim_end_matches([';', '{'])
            .split_whitespace()
            .next()
            .unwrap_or_default();
        if !value.is_empty()
            && value
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "._$`".contains(c))
        {
            return Some(value.to_owned());
        }
    }
    None
}

fn is_import_node(kind: &str) -> bool {
    let lower = kind.to_ascii_lowercase();
    lower.contains("import") || lower == "export_directive"
}

fn parse_import(statement: &str) -> Option<(String, Option<String>, bool)> {
    let trimmed = statement.trim();
    let reexport = trimmed.starts_with("export ") || trimmed.starts_with("export\n");
    let keyword = if reexport { "export" } else { "import" };
    let rest = trimmed.strip_prefix(keyword)?.trim();
    let mut target = rest
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .trim_end_matches(';')
        .trim_matches(['\'', '"', '`'])
        .to_owned();
    if target.is_empty() {
        return None;
    }
    if let Some(index) = target.find(',') {
        target.truncate(index);
    }
    let alias = rest
        .split_whitespace()
        .collect::<Vec<_>>()
        .windows(2)
        .find(|pair| pair[0] == "as")
        .map(|pair| pair[1].trim_matches([',', ';', '`', '\'', '"']).to_owned())
        .filter(|value| valid_name(value));
    Some((target, alias, reexport))
}

fn terminal(value: &str) -> &str {
    value.rsplit(['.', ':', '/']).next().unwrap_or(value)
}

fn qualifier_for(value: &str) -> Option<&str> {
    value.rsplit_once(['.', ':']).map(|(prefix, _)| prefix)
}

fn split_qualified(raw: &str) -> (Option<String>, String) {
    let cleaned = raw
        .trim()
        .trim_matches(['`', '\'', '"'])
        .trim_end_matches(['?', '!']);
    if let Some((prefix, name)) = cleaned.rsplit_once('.') {
        (Some(prefix.trim().to_owned()), terminal(name).to_owned())
    } else if let Some((prefix, name)) = cleaned.rsplit_once("::") {
        (Some(prefix.trim().to_owned()), terminal(name).to_owned())
    } else {
        (None, terminal(cleaned).to_owned())
    }
}

fn is_call_node(kind: &str) -> bool {
    let lower = kind.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "call_expression"
            | "call"
            | "function_call"
            | "method_call"
            | "method_invocation"
            | "invocation_expression"
            | "apply_expression"
            | "call_expression_with_trailing_closure"
    ) || (lower.contains("invocation") && !lower.contains("declaration"))
}

fn call_callee(node: Node<'_>) -> Option<Node<'_>> {
    for field in ["function", "callee", "name", "method", "receiver", "object"] {
        if let Some(child) = node.child_by_field_name(field) {
            return Some(child);
        }
    }
    let mut cursor = node.walk();
    node.named_children(&mut cursor).next()
}

fn is_type_leaf(kind: &str) -> bool {
    matches!(
        kind,
        "type_identifier"
            | "simple_type"
            | "user_type"
            | "named_type"
            | "type_reference"
            | "class_type"
    )
}

fn is_base_context(node: Node<'_>) -> bool {
    let mut current = node.parent();
    for _ in 0..=3 {
        let Some(parent) = current else { break };
        let lower = parent.kind().to_ascii_lowercase();
        if lower.contains("extends")
            || lower.contains("implements")
            || lower.contains("supertype")
            || lower.contains("inheritance")
            || lower.contains("base")
        {
            return true;
        }
        current = parent.parent();
    }
    false
}

fn is_member_node(kind: &str) -> bool {
    let lower = kind.to_ascii_lowercase();
    lower.contains("member_access")
        || lower.contains("field_expression")
        || lower == "selector"
        || lower.contains("navigation")
        || lower.contains("property_access")
}

fn is_decorator_node(kind: &str) -> bool {
    let lower = kind.to_ascii_lowercase();
    lower.contains("annotation") || lower.contains("decorator") || lower == "attribute_list"
}

fn is_identifier_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn is_identifier_continue(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}
