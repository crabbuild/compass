use std::collections::{HashMap, HashSet};
use std::fs;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

use crate::{RawEdgeRecord as EdgeRecord, RawNodeRecord as NodeRecord};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use tree_sitter::{Node, Parser, Tree};

use crate::builtins::is_language_builtin_global;
use crate::config::{GenericConfig, generic_config};
use crate::{
    CombinedExtraction, EXTRACTION_QUALITY_EXTENSION, EXTRACTION_QUALITY_PARTIAL,
    EXTRACTION_QUALITY_REASON_EXTENSION, ExtractError, Extraction, ExtractorKind,
    FRAMEWORK_PROJECT_EVIDENCE_EXTENSION, LanguageSpec, ProjectEvidenceIndex, RawCall, Registry,
    file_stem, make_id,
};

const JSON_MAX_BYTES: u64 = 1_048_576;

#[derive(Default)]
pub struct Engine {
    parsers: HashMap<&'static str, Parser>,
    project_evidence: Option<Arc<ProjectEvidenceIndex>>,
}

impl Engine {
    #[must_use]
    pub fn with_project_evidence(project_evidence: Arc<ProjectEvidenceIndex>) -> Self {
        Self {
            parsers: HashMap::new(),
            project_evidence: Some(project_evidence),
        }
    }

    pub fn extract(&mut self, path: &Path) -> Result<Extraction, ExtractError> {
        let spec =
            Registry::resolve(path).ok_or_else(|| ExtractError::Unsupported(path.to_path_buf()))?;
        let mut extraction = match spec.kind {
            ExtractorKind::Generic => self.extract_generic(path, spec),
            ExtractorKind::Markdown => {
                let source = fs::read(path).map_err(|source| compass_files::FileError::Io {
                    path: path.to_path_buf(),
                    source,
                })?;
                let source_file = path.to_string_lossy().into_owned();
                crate::markdown::extract_source(path, &source_file, &source)
            }
            ExtractorKind::Html => {
                let source = fs::read(path).map_err(|source| compass_files::FileError::Io {
                    path: path.to_path_buf(),
                    source,
                })?;
                let source_file = path.to_string_lossy().into_owned();
                crate::html::extract_source(path, &source_file, &source)
            }
            ExtractorKind::JsonConfig => self.extract_json(path, spec),
            ExtractorKind::McpConfig => crate::mcp::extract(path),
            ExtractorKind::PackageManifest => crate::package_manifest::extract(path),
            ExtractorKind::Terraform => self.extract_terraform(path, spec),
            ExtractorKind::PascalForm => crate::pascal_forms::extract_form(path),
            ExtractorKind::LazarusPackage => crate::pascal_forms::extract_package(path),
            ExtractorKind::DreamMaker => self.extract_dreammaker(path),
            ExtractorKind::Solution => crate::dotnet_project::extract_solution(path),
            ExtractorKind::ProjectXml => crate::dotnet_project::extract_project(path),
            ExtractorKind::Xaml => crate::xaml::extract(self, path),
            ExtractorKind::Template => {
                let mut extraction = crate::templates::extract(self, path, spec.name)?;
                if let Ok(source) = fs::read(path) {
                    crate::frameworks::detect_template_file_route(
                        path,
                        &source,
                        self.project_evidence(path),
                        &mut extraction,
                    );
                }
                Ok(extraction)
            }
            ExtractorKind::FrameworkConfig => {
                let source = fs::read(path).map_err(|source| compass_files::FileError::Io {
                    path: path.to_path_buf(),
                    source,
                })?;
                Ok(crate::frameworks::detect_config_file(
                    path,
                    &source,
                    self.project_evidence(path),
                ))
            }
        }?;
        self.stamp_project_evidence(path, &mut extraction);
        stamp_producer_metadata(&mut extraction, spec.name);
        Ok(extraction)
    }

    /// Extract from bytes already read by the caller. Source-driven extractors
    /// use the supplied buffer directly; format-specific fallbacks retain their
    /// existing file-based implementations.
    pub fn extract_source(
        &mut self,
        path: &Path,
        source: &[u8],
    ) -> Result<Extraction, ExtractError> {
        let spec =
            Registry::resolve(path).ok_or_else(|| ExtractError::Unsupported(path.to_path_buf()))?;
        let mut extraction = match spec.kind {
            ExtractorKind::Generic => self.extract_generic_source(path, spec, source),
            ExtractorKind::Markdown => {
                let source_file = path.to_string_lossy().into_owned();
                crate::markdown::extract_source(path, &source_file, source)
            }
            ExtractorKind::Html => {
                let source_file = path.to_string_lossy().into_owned();
                crate::html::extract_source(path, &source_file, source)
            }
            ExtractorKind::JsonConfig => self.extract_json_source(path, spec, source),
            ExtractorKind::Terraform => self.extract_terraform_source(path, spec, source),
            ExtractorKind::FrameworkConfig => Ok(crate::frameworks::detect_config_file(
                path,
                source,
                self.project_evidence(path),
            )),
            _ => self.extract(path),
        }?;
        self.stamp_project_evidence(path, &mut extraction);
        stamp_producer_metadata(&mut extraction, spec.name);
        Ok(extraction)
    }

    /// Extract qualification-only Ruby/TypeScript/JavaScript universal evidence
    /// directly from source. The same source-backed emitters are used by the
    /// production registry after the language cutover so qualification cannot
    /// drift from published output.
    ///
    /// This hidden API remains useful for qualification fixtures, but it now
    /// calls the same registered candidate emitter used by normal Compass
    /// extraction. Keeping both paths on one implementation prevents a
    /// qualification-only graph from diverging from production output.
    #[doc(hidden)]
    pub fn extract_source_universal_candidate_evidence(
        &mut self,
        path: &Path,
        source_file: &str,
        source: &[u8],
    ) -> Result<crate::SemanticEvidenceBatch, ExtractError> {
        let spec =
            Registry::resolve(path).ok_or_else(|| ExtractError::Unsupported(path.to_path_buf()))?;
        if !matches!(spec.name, "typescript" | "tsx" | "javascript" | "kotlin" | "ruby") {
            return Err(ExtractError::Unsupported(path.to_path_buf()));
        }
        let tree = self.parse(path, spec, source)?;
        let evidence = if matches!(spec.name, "kotlin" | "ruby") {
            let profile = Registry::universal_profile_for_spec(spec)
                .ok_or_else(|| ExtractError::Unsupported(path.to_path_buf()))?;
            crate::evidence::extract_tree_evidence(
                path,
                source_file,
                source,
                tree.root_node(),
                profile,
            )
        } else {
            crate::evidence::extract_candidate_tree_evidence(
                path,
                source_file,
                source,
                tree.root_node(),
                spec.name,
            )
        };
        evidence.map_err(|error| ExtractError::InvalidProgramEvidence {
            path: path.to_path_buf(),
            detail: error.to_string(),
        })
    }

    pub fn extract_source_combined(
        &mut self,
        path: &Path,
        source_file: &str,
        source: &[u8],
    ) -> Result<CombinedExtraction, ExtractError> {
        self.extract_source_with_program(path, source_file, source, true)
    }

    /// Extract only structural graph evidence while retaining an explicit,
    /// repository-relative source identity.
    ///
    /// Structural-only callers use this path to avoid constructing the
    /// independent Program IR evidence batch from the same syntax tree.
    pub fn extract_source_graph_only(
        &mut self,
        path: &Path,
        source_file: &str,
        source: &[u8],
    ) -> Result<Extraction, ExtractError> {
        self.extract_source_with_program(path, source_file, source, false)
            .map(|combined| combined.graph)
    }

    fn extract_source_with_program(
        &mut self,
        path: &Path,
        source_file: &str,
        source: &[u8],
        include_program: bool,
    ) -> Result<CombinedExtraction, ExtractError> {
        let spec =
            Registry::resolve(path).ok_or_else(|| ExtractError::Unsupported(path.to_path_buf()))?;
        if spec.kind == ExtractorKind::Markdown {
            let mut graph = crate::markdown::extract_source(path, source_file, source)?;
            self.stamp_project_evidence(path, &mut graph);
            stamp_producer_metadata(&mut graph, spec.name);
            return Ok(CombinedExtraction {
                graph,
                program: None,
            });
        }
        if spec.kind == ExtractorKind::Html {
            let mut graph = crate::html::extract_source(path, source_file, source)?;
            self.stamp_project_evidence(path, &mut graph);
            stamp_producer_metadata(&mut graph, spec.name);
            return Ok(CombinedExtraction {
                graph,
                program: None,
            });
        }
        let universal_profile = Registry::universal_profile_for_spec(spec);
        if spec.kind == ExtractorKind::Generic
            && universal_profile.is_some()
            && !crate::program::supports_language(spec.name)
        {
            let tree = self.parse(path, spec, source)?;
            let mut graph = self.extract_generic_from_tree(
                path,
                spec,
                source_file,
                true,
                source,
                tree.root_node(),
            );
            self.stamp_project_evidence(path, &mut graph);
            stamp_producer_metadata(&mut graph, spec.name);
            return Ok(CombinedExtraction {
                graph,
                program: None,
            });
        }
        if spec.kind != ExtractorKind::Generic || !crate::program::supports_language(spec.name) {
            return self
                .extract_source(path, source)
                .map(|graph| CombinedExtraction {
                    graph,
                    program: None,
                });
        }
        let tree = self.parse(path, spec, source)?;
        let root = tree.root_node();
        let mut graph = self.extract_generic_from_tree(path, spec, source_file, true, source, root);
        self.stamp_project_evidence(path, &mut graph);
        stamp_producer_metadata(&mut graph, spec.name);
        let program = include_program
            .then(|| crate::program::extract_from_tree(source_file, spec.name, source, root))
            .transpose()
            .map_err(|error| ExtractError::InvalidProgramEvidence {
                path: path.to_path_buf(),
                detail: error.to_string(),
            })?;
        Ok(CombinedExtraction { graph, program })
    }

    pub(super) fn extract_embedded_script(
        &mut self,
        path: &Path,
        source: &[u8],
        language: &'static str,
        grammar: &'static str,
    ) -> Result<Extraction, ExtractError> {
        let spec = LanguageSpec {
            name: language,
            grammar: Some(grammar),
            kind: ExtractorKind::Generic,
        };
        let tree = self.parse(path, spec, source)?;
        let mut extraction = extract_tree(
            path,
            source,
            tree.root_node(),
            &generic_config(spec),
            language,
            true,
        );
        if language == "python" {
            add_python_rationale(path, source, tree.root_node(), &mut extraction);
        }
        attach_definition_metadata(
            &mut extraction,
            source,
            tree.root_node(),
            &generic_config(spec),
            language,
        );
        self.stamp_project_evidence(path, &mut extraction);
        stamp_producer_metadata(&mut extraction, language);
        Ok(extraction)
    }

    fn extract_generic(
        &mut self,
        path: &Path,
        spec: LanguageSpec,
    ) -> Result<Extraction, ExtractError> {
        let source = fs::read(path).map_err(|source| compass_files::FileError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        self.extract_generic_source(path, spec, &source)
    }

    fn extract_generic_source(
        &mut self,
        path: &Path,
        spec: LanguageSpec,
        source: &[u8],
    ) -> Result<Extraction, ExtractError> {
        if spec.name == "groovy" {
            let mut extraction = crate::groovy::extract(path, source);
            attach_basic_symbol_metadata(&mut extraction, source, spec.name);
            return Ok(extraction);
        }
        // These extractors are intentionally source-driven and do not consume a
        // tree-sitter root. Avoid initializing and touching their large static
        // grammar tables only to discard the tree; this materially lowers cold
        // multilingual startup RSS and latency while preserving identical facts.
        let source_driven = match spec.name {
            "zig" => Some(crate::zig::extract(path, source)),
            "verilog" => Some(crate::verilog::extract(path, source)),
            "sql" => Some(crate::sql::extract(path, source)),
            "r" => Some(crate::r::extract(path, source)),
            "pascal" => Some(crate::pascal::extract(path, source)),
            "apex" => Some(crate::apex::extract(path, source)),
            "dart" => Some(crate::dart::extract(path, source)),
            _ => None,
        };
        if let Some(mut extraction) = source_driven {
            attach_basic_symbol_metadata(&mut extraction, source, spec.name);
            return Ok(extraction);
        }
        let mut masked = Vec::new();
        let source = if spec.name == "objc" {
            masked.extend_from_slice(source);
            crate::objc::mask_annotation_macros(&mut masked);
            masked.as_slice()
        } else {
            source
        };
        let tree = self.parse(path, spec, source)?;
        let evidence_source_file = portable_evidence_source(path);
        Ok(self.extract_generic_from_tree(
            path,
            spec,
            &evidence_source_file,
            false,
            source,
            tree.root_node(),
        ))
    }

    fn extract_generic_from_tree(
        &self,
        path: &Path,
        spec: LanguageSpec,
        evidence_source_file: &str,
        evidence_source_is_explicit: bool,
        source: &[u8],
        root: Node<'_>,
    ) -> Extraction {
        let config = generic_config(spec);
        let universal_profile = Registry::universal_profile_for_spec(spec);
        let mut extraction = if universal_profile.is_some() {
            Extraction::default()
        } else {
            match spec.name {
                "go" => crate::go::extract(path, source, root),
                "bash" => crate::bash::extract(path, source, root),
                "cpp" => crate::cpp::extract(path, source, root),
                "php" => crate::php::extract(path, source, root),
                "swift" => crate::swift::extract(path, source, root),
                "objc" => crate::objc::extract(path, source, root),
                "powershell" => crate::powershell::extract(path, source, root),
                "elixir" => crate::elixir::extract(path, source, root),
                "julia" => crate::julia::extract(path, source, root),
                "fortran" => crate::fortran::extract(path, source, root),
                _ => extract_tree(path, source, root, &config, spec.name, true),
            }
        };
        if spec.name == "python" {
            add_python_rationale(path, source, root, &mut extraction);
        }
        if universal_profile.is_none() {
            attach_definition_metadata(&mut extraction, source, root, &config, spec.name);
            crate::semantic::enrich(path, source, root, spec.name, &mut extraction);
        }
        if let Some(profile) = universal_profile {
            match crate::evidence::extract_tree_evidence(
                path,
                evidence_source_file,
                source,
                root,
                profile,
            ) {
                Ok(evidence) => {
                    extraction.semantic_evidence = Some(evidence);
                    project_universal_declaration_sources(&mut extraction);
                }
                Err(error) => {
                    extraction.error = Some(format!(
                        "{} universal evidence extraction failed: {error}",
                        spec.name
                    ));
                }
            }
        }
        // Framework conventions need the complete repository-relative path
        // (for example `src/routes/**`, `app/routes/**`, and `src/app/**`) to
        // classify a file. Pipeline callers supply that authoritative identity;
        // direct extraction derives a bounded portable fallback from the path.
        let framework_source = universal_profile.map(|_| {
            if evidence_source_is_explicit
                && !evidence_source_file.is_empty()
                && Path::new(evidence_source_file).is_relative()
            {
                evidence_source_file.replace('\\', "/")
            } else {
                portable_framework_source(path)
            }
        });
        let framework_path = framework_source.as_deref().map(Path::new).unwrap_or(path);
        crate::frameworks::detect(
            framework_path,
            source,
            root,
            spec.name,
            self.project_evidence(path),
            &mut extraction,
        );
        if universal_profile.is_some() && extraction.semantic_evidence.is_some() {
            extraction.raw_calls = None;
        }
        if root.has_error() {
            extraction.extensions.insert(
                EXTRACTION_QUALITY_EXTENSION.to_owned(),
                Value::String(EXTRACTION_QUALITY_PARTIAL.to_owned()),
            );
            extraction.extensions.insert(
                EXTRACTION_QUALITY_REASON_EXTENSION.to_owned(),
                Value::String("syntax parser recovered from malformed input".to_owned()),
            );
        }
        extraction
    }

    fn project_evidence(&self, path: &Path) -> Option<&crate::ProjectEvidence> {
        self.project_evidence
            .as_deref()
            .map(|index| index.evidence_for(path))
    }

    fn stamp_project_evidence(&self, path: &Path, extraction: &mut Extraction) {
        let Some(evidence) = self.project_evidence(path) else {
            return;
        };
        extraction.extensions.insert(
            FRAMEWORK_PROJECT_EVIDENCE_EXTENSION.to_owned(),
            Value::String(evidence.fingerprint().to_owned()),
        );
    }

    fn extract_json(
        &mut self,
        path: &Path,
        spec: LanguageSpec,
    ) -> Result<Extraction, ExtractError> {
        let mut source = Vec::new();
        File::open(path)
            .map_err(|source| compass_files::FileError::Io {
                path: path.to_path_buf(),
                source,
            })?
            .take(JSON_MAX_BYTES + 1)
            .read_to_end(&mut source)
            .map_err(|source| compass_files::FileError::Io {
                path: path.to_path_buf(),
                source,
            })?;
        self.extract_json_source(path, spec, &source)
    }

    fn extract_json_source(
        &mut self,
        path: &Path,
        spec: LanguageSpec,
        source: &[u8],
    ) -> Result<Extraction, ExtractError> {
        if source.len() > JSON_MAX_BYTES as usize {
            return Ok(crate::json_config::error("json file too large to index"));
        }
        let tree = self.parse(path, spec, source)?;
        Ok(crate::json_config::extract(path, source, tree.root_node()))
    }

    fn extract_terraform(
        &mut self,
        path: &Path,
        spec: LanguageSpec,
    ) -> Result<Extraction, ExtractError> {
        let source = fs::read(path).map_err(|source| compass_files::FileError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        self.extract_terraform_source(path, spec, &source)
    }

    fn extract_terraform_source(
        &mut self,
        path: &Path,
        spec: LanguageSpec,
        source: &[u8],
    ) -> Result<Extraction, ExtractError> {
        let tree = self.parse(path, spec, source)?;
        Ok(crate::terraform::extract(path, source, tree.root_node()))
    }

    fn extract_dreammaker(&mut self, path: &Path) -> Result<Extraction, ExtractError> {
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !matches!(extension.as_str(), "dm" | "dme") {
            return crate::dm::extract_asset(path);
        }
        let source = fs::read(path).map_err(|source| compass_files::FileError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let parser = if let Some(parser) = self.parsers.get_mut("dm") {
            parser
        } else {
            let language = tree_sitter_dm::LANGUAGE.into();
            let mut parser = Parser::new();
            parser
                .set_language(&language)
                .map_err(|error| ExtractError::MissingGrammar {
                    language: "dm".to_owned(),
                    detail: error.to_string(),
                })?;
            self.parsers.entry("dm").or_insert(parser)
        };
        let tree = parser
            .parse(&source, None)
            .ok_or_else(|| ExtractError::ParseCancelled(path.to_path_buf()))?;
        Ok(crate::dm::extract_source(path, &source, tree.root_node()))
    }

    pub(crate) fn parse(
        &mut self,
        path: &Path,
        spec: LanguageSpec,
        source: &[u8],
    ) -> Result<Tree, ExtractError> {
        let grammar = spec
            .grammar
            .ok_or_else(|| ExtractError::Unsupported(path.to_path_buf()))?;
        let parser = if let Some(parser) = self.parsers.get_mut(grammar) {
            parser
        } else {
            let language = tree_sitter_language_pack::get_language(grammar).map_err(|error| {
                ExtractError::MissingGrammar {
                    language: grammar.to_owned(),
                    detail: error.to_string(),
                }
            })?;
            let mut parser = Parser::new();
            parser
                .set_language(&language)
                .map_err(|error| ExtractError::MissingGrammar {
                    language: grammar.to_owned(),
                    detail: error.to_string(),
                })?;
            self.parsers.entry(grammar).or_insert(parser)
        };
        // Keep known pinned-grammar gaps from quarantining otherwise exact
        // TypeScript files. The parser-only mask preserves every byte and line
        // offset; extractors still read names, modules, and hashes from the
        // original source buffer.
        let masked = matches!(spec.name, "typescript" | "tsx")
            .then(|| mask_typescript_parser_gaps(source))
            .flatten();
        let parser_source = masked.as_deref().unwrap_or(source);
        let mut tree = parser
            .parse(parser_source, None)
            .ok_or_else(|| ExtractError::ParseCancelled(path.to_path_buf()))?;
        if matches!(spec.name, "typescript" | "tsx")
            && tree.root_node().has_error()
            && let Some(variance_masked) =
                mask_typescript_variance_errors(parser_source, tree.root_node())
        {
            tree = parser
                .parse(&variance_masked, None)
                .ok_or_else(|| ExtractError::ParseCancelled(path.to_path_buf()))?;
        }
        Ok(tree)
    }
}

fn mask_typescript_parser_gaps(source: &[u8]) -> Option<Vec<u8>> {
    let mut masked = None::<Vec<u8>>;
    let mut line_start = 0;
    while line_start < source.len() {
        let line_end = source[line_start..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(source.len(), |offset| line_start.saturating_add(offset));
        let line = &source[line_start..line_end];
        let mut offset = line
            .iter()
            .take_while(|byte| byte.is_ascii_whitespace())
            .count();
        if line.get(offset..offset.saturating_add(6)) == Some(b"export")
            && line
                .get(offset.saturating_add(6))
                .is_some_and(u8::is_ascii_whitespace)
        {
            offset = offset.saturating_add(6);
            offset = offset.saturating_add(
                line[offset..]
                    .iter()
                    .take_while(|byte| byte.is_ascii_whitespace())
                    .count(),
            );
            let modifier_start = offset;
            if line.get(offset..offset.saturating_add(4)) == Some(b"type")
                && line
                    .get(offset.saturating_add(4))
                    .is_some_and(u8::is_ascii_whitespace)
            {
                offset = offset.saturating_add(4);
                offset = offset.saturating_add(
                    line[offset..]
                        .iter()
                        .take_while(|byte| byte.is_ascii_whitespace())
                        .count(),
                );
                if line.get(offset) == Some(&b'*') {
                    let output = masked.get_or_insert_with(|| source.to_vec());
                    output[line_start.saturating_add(modifier_start)
                        ..line_start.saturating_add(modifier_start).saturating_add(4)]
                        .fill(b' ');
                }
            }
        }
        let declaration = line
            .get(
                line.iter()
                    .take_while(|byte| byte.is_ascii_whitespace())
                    .count()..,
            )
            .unwrap_or_default();
        let is_type_alias =
            declaration.starts_with(b"type ") || declaration.starts_with(b"export type ");
        let unsupported_import_query = line
            .windows(b"typeof import(".len())
            .position(|window| window == b"typeof import(")
            .filter(|query| line[*query..].windows(2).any(|window| window == b")["));
        if is_type_alias
            && let Some(query) = unsupported_import_query
            && let Some(equals) = line[..query].iter().rposition(|byte| *byte == b'=')
            && let Some(semicolon) = line.iter().rposition(|byte| *byte == b';')
        {
            let value_start = equals.saturating_add(1).saturating_add(
                line[equals.saturating_add(1)..semicolon]
                    .iter()
                    .take_while(|byte| byte.is_ascii_whitespace())
                    .count(),
            );
            if value_start.saturating_add(3) <= semicolon {
                let output = masked.get_or_insert_with(|| source.to_vec());
                output[line_start.saturating_add(equals).saturating_add(1)
                    ..line_start.saturating_add(semicolon)]
                    .fill(b' ');
                output[line_start.saturating_add(value_start)
                    ..line_start.saturating_add(value_start).saturating_add(3)]
                    .copy_from_slice(b"any");
            }
        }
        line_start = line_end.saturating_add(1);
    }
    masked
}

fn mask_typescript_variance_errors(source: &[u8], root: Node<'_>) -> Option<Vec<u8>> {
    // The pinned upstream grammar reports valid TypeScript `in`/`out`
    // variance modifiers as either direct ERROR children of `type_parameters`
    // or as the immediately preceding token of an errored `type_parameter`.
    // Restrict the second parse to those parser-proven positions so mapped-
    // type `in`, identifiers named `out`, and unrelated malformed input retain
    // their original meaning and recovery status. The replacement is byte-
    // preserving, so the reparsed tree still addresses the original source.
    const MAX_VARIANCE_MODIFIERS: usize = 256;

    let mut ranges = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.is_error()
            && let Some(range) = typescript_variance_error_range(source, node)
        {
            if ranges.len() == MAX_VARIANCE_MODIFIERS {
                return None;
            }
            ranges.push(range);
        }
        let mut cursor = node.walk();
        stack.extend(node.children(&mut cursor));
    }
    if ranges.is_empty() {
        return None;
    }
    ranges.sort_unstable_by_key(|range| (range.start, range.end));
    ranges.dedup();
    let mut masked = source.to_vec();
    for range in ranges {
        masked[range].fill(b' ');
    }
    Some(masked)
}

fn typescript_variance_error_range(
    source: &[u8],
    node: Node<'_>,
) -> Option<std::ops::Range<usize>> {
    let parent = node.parent()?;
    if parent.kind() == "type_parameters"
        && source
            .get(node.start_byte()..node.end_byte())
            .is_some_and(|text| matches!(text, b"in" | b"out"))
    {
        return Some(node.start_byte()..node.end_byte());
    }
    if parent.kind() != "type_parameter"
        || !parent
            .parent()
            .is_some_and(|grandparent| grandparent.kind() == "type_parameters")
    {
        return None;
    }
    let prefix = source.get(parent.start_byte()..node.start_byte())?;
    let modifier_end = prefix
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())?
        .saturating_add(1);
    let modifier_start = prefix[..modifier_end]
        .iter()
        .rposition(|byte| byte.is_ascii_whitespace())
        .map_or(0, |index| index.saturating_add(1));
    matches!(&prefix[modifier_start..modifier_end], b"in" | b"out").then(|| {
        parent.start_byte().saturating_add(modifier_start)
            ..parent.start_byte().saturating_add(modifier_end)
    })
}

fn project_universal_declaration_sources(extraction: &mut Extraction) {
    let Some(evidence) = extraction.semantic_evidence.as_ref() else {
        return;
    };
    let locations = evidence
        .declarations
        .iter()
        .map(|declaration| (declaration.graph_node_id.as_str(), &declaration.range))
        .collect::<HashMap<_, _>>();
    for node in &mut extraction.nodes {
        let Some(range) = locations.get(node.id.as_str()) else {
            continue;
        };
        node.attributes.insert(
            "source_file".to_owned(),
            Value::String(range.source_file.clone()),
        );
    }
}

fn portable_evidence_source(path: &Path) -> String {
    if path.is_relative() {
        return path.to_string_lossy().replace('\\', "/");
    }
    let file = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("source");
    let parent = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if parent.is_empty() || parent.starts_with('.') || parent.starts_with("tmp") {
        file.to_owned()
    } else {
        format!("{parent}/{file}")
    }
}

fn portable_framework_source(path: &Path) -> String {
    if path.is_relative() {
        return path.to_string_lossy().replace('\\', "/");
    }
    let source = path.to_string_lossy().replace('\\', "/");
    const ROUTE_MARKERS: &[&str] = &[
        "src/routes/",
        "app/routes/",
        "src/app/",
        "app/",
        "src/pages/",
        "pages/",
        "server/api/",
        "middleware/",
    ];
    if let Some((index, _)) = ROUTE_MARKERS
        .iter()
        .filter_map(|marker| {
            source
                .match_indices(marker)
                .find(|(index, _)| *index == 0 || source.as_bytes().get(index - 1) == Some(&b'/'))
                .map(|(index, _)| (index, *marker))
        })
        .min_by_key(|(index, _)| *index)
    {
        return source[index..].to_owned();
    }
    portable_evidence_source(path)
}

struct FunctionBody<'tree> {
    id: String,
    node: Node<'tree>,
    top_level: bool,
}

#[derive(Clone)]
struct JsImportTarget {
    target: String,
    module: String,
    imported_name: String,
    type_only: bool,
}

struct ExtractState<'source, 'tree> {
    source: &'source [u8],
    source_file: String,
    stem: String,
    file_id: String,
    config: &'source GenericConfig,
    language: &'static str,
    extraction: Extraction,
    seen_nodes: HashSet<String>,
    functions: Vec<FunctionBody<'tree>>,
    callables: HashMap<String, Vec<String>>,
    types: HashMap<String, String>,
    seen_resolved_calls: HashSet<(String, String, usize, usize)>,
    seen_js_references: HashSet<(String, String, usize, usize)>,
    seen_dynamic_imports: HashSet<(String, String)>,
    js_import_targets: HashMap<String, JsImportTarget>,
    js_type_namespace_names: HashSet<String>,
    js_value_bindings: HashMap<String, Vec<String>>,
    python_import_aliases: HashMap<String, String>,
    python_import_targets: HashMap<String, String>,
}

fn extract_tree(
    path: &Path,
    source: &[u8],
    root: Node<'_>,
    config: &GenericConfig,
    language: &'static str,
    collect_calls: bool,
) -> Extraction {
    let source_file = path.to_string_lossy().into_owned();
    let stem = file_stem(path);
    let file_id = make_id(&[&source_file]);
    let (python_import_aliases, python_import_targets) = if language == "python" && collect_calls {
        python_import_maps(root, source)
    } else {
        (HashMap::new(), HashMap::new())
    };
    let js_type_namespace_names = if matches!(language, "javascript" | "typescript" | "tsx") {
        js_top_level_type_names(root, config, source)
    } else {
        HashSet::new()
    };
    let mut state = ExtractState {
        source,
        source_file,
        stem,
        file_id,
        config,
        language,
        extraction: Extraction::default(),
        seen_nodes: HashSet::new(),
        functions: Vec::new(),
        callables: HashMap::new(),
        types: HashMap::new(),
        seen_resolved_calls: HashSet::new(),
        seen_js_references: HashSet::new(),
        seen_dynamic_imports: HashSet::new(),
        js_import_targets: HashMap::new(),
        js_type_namespace_names,
        js_value_bindings: HashMap::new(),
        python_import_aliases,
        python_import_targets,
    };
    let file_label = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    state.add_node(&state.file_id.clone(), file_label, 1, false, None);
    state.walk_declarations(root, None);
    if matches!(language, "javascript" | "typescript" | "tsx") {
        state.walk_jsx_references(root);
    }
    if language == "python" && collect_calls {
        let module_bound = python_bound_names(root, source, true);
        state.walk_python_indirect(root, &state.file_id.clone(), true, &module_bound);
    } else if matches!(language, "javascript" | "typescript" | "tsx") {
        state.walk_js_module_indirect(root, true);
    }
    if collect_calls {
        state.walk_function_calls();
    }
    state.extraction
}

fn attach_definition_metadata(
    extraction: &mut Extraction,
    source: &[u8],
    root: Node<'_>,
    config: &GenericConfig,
    language: &str,
) {
    attach_basic_symbol_metadata(extraction, source, language);
    let mut candidates = HashMap::<usize, Vec<usize>>::new();
    for (index, node) in extraction.nodes.iter().enumerate() {
        if node.string("file_type") != "code" || node.string("source_file").is_empty() {
            continue;
        }
        let Some(line) = node
            .attributes
            .get("source_location")
            .and_then(Value::as_str)
            .and_then(source_line)
        else {
            continue;
        };
        candidates.entry(line).or_default().push(index);
    }
    let mut definitions = Vec::new();
    collect_definitions(root, config, &mut definitions);
    for definition in definitions {
        let at_line = definition.start_position().row + 1;
        let Some(indices) = candidates.get_mut(&at_line) else {
            continue;
        };
        let exact = indices.iter().position(|index| {
            let record = &extraction.nodes[*index];
            record.attributes.get("start_byte").and_then(Value::as_u64)
                == Some(u64::try_from(definition.start_byte()).unwrap_or(u64::MAX))
                && record.attributes.get("end_byte").and_then(Value::as_u64)
                    == Some(u64::try_from(definition.end_byte()).unwrap_or(u64::MAX))
        });
        let name = definition_name(definition, source);
        let matched = name.as_deref().and_then(|name| {
            indices.iter().position(|index| {
                normalize_symbol_label(extraction.nodes[*index].label())
                    == clean_name(name.to_owned())
            })
        });
        let fallback = indices.iter().position(|index| {
            extraction.nodes[*index]
                .attributes
                .get("_callable")
                .and_then(Value::as_bool)
                == Some(true)
        });
        let unanchored_single = (indices.len() == 1
            && extraction.nodes[indices[0]]
                .attributes
                .get("start_byte")
                .is_none())
        .then_some(0);
        let Some(position) = exact.or(matched).or(fallback).or(unanchored_single) else {
            continue;
        };
        let index = indices.remove(position);
        let body = definition_body(definition);
        let signature_hash = ast_hash(definition, source, body.map(|body| body.id()));
        let source_hash = source
            .get(definition.start_byte()..definition.end_byte())
            .map(normalized_source_hash);
        let is_nested = extraction.nodes[index].label().starts_with('.');
        let symbol_kind = symbol_kind(
            definition.kind(),
            config.class_types.contains(&definition.kind()),
            is_nested,
        );
        let attributes = &mut extraction.nodes[index].attributes;
        attributes.insert(
            "symbol_kind".to_owned(),
            Value::String(symbol_kind.to_owned()),
        );
        attributes.insert("language".to_owned(), Value::String(language.to_owned()));
        attributes.insert(
            "line_start".to_owned(),
            Value::from(definition.start_position().row + 1),
        );
        attributes.insert(
            "line_end".to_owned(),
            Value::from(definition.end_position().row + 1),
        );
        if let Some(signature) = readable_signature(definition, body, source) {
            attributes.insert("signature".to_owned(), Value::String(signature));
        }
        attributes.insert("signature_hash".to_owned(), Value::String(signature_hash));
        if let Some(body) = body {
            attributes.insert(
                "implementation_hash".to_owned(),
                Value::String(ast_hash(body, source, None)),
            );
        }
        if let Some(source_hash) = source_hash {
            attributes.insert("source_hash".to_owned(), Value::String(source_hash));
        }
    }
    attach_overload_discriminators(extraction);
}

fn attach_overload_discriminators(extraction: &mut Extraction) {
    let mut groups = HashMap::<(String, String, String), Vec<usize>>::new();
    for (index, node) in extraction.nodes.iter().enumerate() {
        let source_file = node.string("source_file");
        let symbol_kind = node.string("symbol_kind");
        let mut qualified_name = node.string("qualified_name");
        if qualified_name.is_empty() {
            qualified_name = node.label().to_owned();
        }
        if source_file.is_empty() || symbol_kind.is_empty() || qualified_name.is_empty() {
            continue;
        }
        groups
            .entry((source_file, symbol_kind, qualified_name))
            .or_default()
            .push(index);
    }
    for indices in groups.values_mut().filter(|indices| indices.len() > 1) {
        indices.sort_by_key(|index| {
            extraction.nodes[*index]
                .attributes
                .get("line_start")
                .and_then(Value::as_u64)
                .unwrap_or(u64::MAX)
        });
        for (position, index) in indices.iter().enumerate() {
            extraction.nodes[*index].attributes.insert(
                "overload_discriminator".to_owned(),
                Value::String(format!("overload:{position}")),
            );
        }
    }
}

fn attach_basic_symbol_metadata(extraction: &mut Extraction, source: &[u8], language: &str) {
    let source_lines = source.iter().filter(|byte| **byte == b'\n').count() + 1;
    for node in &mut extraction.nodes {
        let source_file = node.string("source_file");
        if source_file.is_empty() {
            continue;
        }
        let line_start = node
            .attributes
            .get("source_location")
            .and_then(Value::as_str)
            .and_then(source_line)
            .unwrap_or(1);
        let file_name = Path::new(&source_file)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let is_file = node.label() == file_name;
        let fallback_kind = if is_file {
            "file".to_owned()
        } else if node.label().ends_with("()") {
            if node.label().starts_with('.') {
                "method".to_owned()
            } else {
                "function".to_owned()
            }
        } else {
            node.attributes
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("symbol")
                .to_owned()
        };
        node.attributes
            .entry("symbol_kind".to_owned())
            .or_insert_with(|| Value::String(fallback_kind));
        node.attributes
            .entry("language".to_owned())
            .or_insert_with(|| Value::String(language.to_owned()));
        node.attributes
            .entry("line_start".to_owned())
            .or_insert_with(|| Value::from(line_start));
        node.attributes
            .entry("line_end".to_owned())
            .or_insert_with(|| Value::from(if is_file { source_lines } else { line_start }));
    }
}

pub(crate) fn stamp_producer_metadata(extraction: &mut Extraction, language: &str) {
    let extractor = format!("compass.languages.{language}");
    for attributes in extraction
        .nodes
        .iter_mut()
        .map(|node| &mut node.attributes)
        .chain(extraction.edges.iter_mut().map(|edge| &mut edge.attributes))
    {
        attributes
            .entry("language".to_owned())
            .or_insert_with(|| Value::String(language.to_owned()));
        attributes
            .entry("extractor".to_owned())
            .or_insert_with(|| Value::String(extractor.clone()));
    }
    if let Some(calls) = extraction.raw_calls.as_mut() {
        for call in calls {
            call.lang.get_or_insert_with(|| language.to_owned());
            call.extensions
                .entry("language".to_owned())
                .or_insert_with(|| Value::String(language.to_owned()));
            call.extensions
                .entry("extractor".to_owned())
                .or_insert_with(|| Value::String(extractor.clone()));
        }
    }
}

fn definition_body(node: Node<'_>) -> Option<Node<'_>> {
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

fn definition_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    if let Some(name) = node.child_by_field_name("name") {
        return Some(source_node_text(name, source));
    }
    let mut declarator = node.child_by_field_name("declarator");
    while let Some(candidate) = declarator {
        if matches!(
            candidate.kind(),
            "identifier" | "field_identifier" | "type_identifier" | "operator_name"
        ) {
            return Some(source_node_text(candidate, source));
        }
        declarator = candidate
            .child_by_field_name("declarator")
            .or_else(|| candidate.child_by_field_name("name"));
    }
    None
}

fn normalize_symbol_label(label: &str) -> String {
    clean_name(
        label
            .trim_start_matches('.')
            .trim_end_matches("()")
            .trim()
            .to_owned(),
    )
}

fn symbol_kind(kind: &str, is_class: bool, is_nested: bool) -> &'static str {
    if is_class {
        if kind.contains("interface") {
            "interface"
        } else if kind.contains("trait") {
            "trait"
        } else if kind.contains("enum") {
            "enum"
        } else if kind.contains("struct") || kind.contains("record") {
            "struct"
        } else if kind.contains("protocol") {
            "protocol"
        } else if kind.contains("module") || kind.contains("object") {
            "module"
        } else if kind.contains("type_alias") || kind == "type_item" {
            "type_alias"
        } else {
            "class"
        }
    } else if kind.contains("deinit") {
        "method"
    } else if kind.contains("constructor") || kind.contains("init_declaration") {
        "constructor"
    } else if kind.contains("method") || is_nested {
        "method"
    } else {
        "function"
    }
}

fn readable_signature(node: Node<'_>, body: Option<Node<'_>>, source: &[u8]) -> Option<String> {
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

fn source_line(location: &str) -> Option<usize> {
    let location = location.strip_prefix('L')?;
    let digits = location.split_once('-').map_or(location, |value| value.0);
    digits.parse().ok()
}

fn collect_definitions<'tree>(
    node: Node<'tree>,
    config: &GenericConfig,
    output: &mut Vec<Node<'tree>>,
) {
    if config.class_types.contains(&node.kind()) || config.function_types.contains(&node.kind()) {
        output.push(node);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_definitions(child, config, output);
    }
}

fn ast_hash(node: Node<'_>, source: &[u8], excluded: Option<usize>) -> String {
    let mut digest = Sha256::new();
    hash_ast_node(node, source, excluded, &mut digest);
    hex_digest(&digest.finalize())
}

fn hash_ast_node(node: Node<'_>, source: &[u8], excluded: Option<usize>, digest: &mut Sha256) {
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
            hash_ast_node(child, source, excluded, digest);
        }
    }
    digest.update(b")");
}

fn normalized_source_hash(source: &[u8]) -> String {
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
    hex_digest(&Sha256::digest(normalized))
}

fn hex_digest(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(value, "{byte:02x}");
    }
    value
}

fn add_python_rationale(path: &Path, source: &[u8], root: Node<'_>, extraction: &mut Extraction) {
    let stem = file_stem(path);
    let file_id = make_id(&[&path.to_string_lossy()]);
    let mut seen = extraction
        .nodes
        .iter()
        .map(|node| node.id.clone())
        .collect::<HashSet<_>>();
    let autogenerated = source.get(..source.len().min(2_048)).is_some_and(|head| {
        let head = String::from_utf8_lossy(head);
        [
            "DO NOT EDIT",
            "@generated",
            "Generated by the protocol buffer",
        ]
        .iter()
        .any(|marker| head.contains(marker))
            || (head.contains("def upgrade(")
                && head.contains("down_revision")
                && (head.contains("revision =") || head.contains("revision:")))
            || (head.contains("class Migration(migrations.Migration)")
                && head.contains("operations"))
    });
    if !autogenerated && let Some((text, line)) = python_docstring(root, source) {
        push_rationale(path, &stem, &file_id, &text, line, extraction, &mut seen);
    }
    walk_python_docstrings(path, &stem, &file_id, root, source, extraction, &mut seen);
    let text = String::from_utf8_lossy(source);
    for (index, line) in text.lines().enumerate() {
        let stripped = line.trim();
        if [
            "# NOTE:",
            "# IMPORTANT:",
            "# HACK:",
            "# WHY:",
            "# RATIONALE:",
            "# TODO:",
            "# FIXME:",
        ]
        .iter()
        .any(|prefix| stripped.starts_with(prefix))
        {
            push_rationale(
                path,
                &stem,
                &file_id,
                stripped,
                index + 1,
                extraction,
                &mut seen,
            );
        }
    }
}

fn walk_python_docstrings(
    path: &Path,
    stem: &str,
    parent_id: &str,
    node: Node<'_>,
    source: &[u8],
    extraction: &mut Extraction,
    seen: &mut HashSet<String>,
) {
    match node.kind() {
        "class_definition" => {
            let Some(name_node) = node.child_by_field_name("name") else {
                return;
            };
            let Some(body) = node.child_by_field_name("body") else {
                return;
            };
            let name = source_node_text(name_node, source);
            let class_id = make_id(&[stem, &name]);
            if let Some((text, line)) = python_docstring(body, source) {
                push_rationale(path, stem, &class_id, &text, line, extraction, seen);
            }
            let mut cursor = body.walk();
            for child in body.children(&mut cursor) {
                walk_python_docstrings(path, stem, &class_id, child, source, extraction, seen);
            }
            return;
        }
        "function_definition" => {
            let Some(name_node) = node.child_by_field_name("name") else {
                return;
            };
            let Some(body) = node.child_by_field_name("body") else {
                return;
            };
            let name = source_node_text(name_node, source);
            let function_id = if parent_id == make_id(&[&path.to_string_lossy()]) {
                make_id(&[stem, &name])
            } else {
                make_id(&[parent_id, &name])
            };
            if let Some((text, line)) = python_docstring(body, source) {
                push_rationale(path, stem, &function_id, &text, line, extraction, seen);
            }
            return;
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_python_docstrings(path, stem, parent_id, child, source, extraction, seen);
    }
}

fn python_docstring(node: Node<'_>, source: &[u8]) -> Option<(String, usize)> {
    let mut cursor = node.walk();
    let first = node.named_children(&mut cursor).next()?;
    let string = if matches!(first.kind(), "string" | "concatenated_string") {
        first
    } else if first.kind() == "expression_statement" {
        let mut inner_cursor = first.walk();
        first
            .named_children(&mut inner_cursor)
            .find(|child| matches!(child.kind(), "string" | "concatenated_string"))?
    } else {
        return None;
    };
    let text = source_node_text(string, source)
        .trim_matches(['\"', '\''])
        .trim()
        .to_owned();
    (text.chars().count() > 20).then(|| (text, line(first)))
}

fn push_rationale(
    path: &Path,
    stem: &str,
    parent_id: &str,
    text: &str,
    line_number: usize,
    extraction: &mut Extraction,
    seen: &mut HashSet<String>,
) {
    let id = make_id(&[stem, "rationale", &line_number.to_string()]);
    let label = text
        .chars()
        .take(80)
        .collect::<String>()
        .replace("\r\n", " ")
        .replace(['\r', '\n'], " ")
        .trim()
        .to_owned();
    let source_file = path.to_string_lossy().into_owned();
    let source_location = format!("L{line_number}");
    if seen.insert(id.clone()) {
        extraction.nodes.push(NodeRecord {
            id: id.clone(),
            attributes: Map::from_iter([
                ("label".to_owned(), Value::String(label)),
                (
                    "file_type".to_owned(),
                    Value::String("rationale".to_owned()),
                ),
                ("source_file".to_owned(), Value::String(source_file.clone())),
                (
                    "source_location".to_owned(),
                    Value::String(source_location.clone()),
                ),
            ]),
        });
    }
    extraction.edges.push(EdgeRecord {
        source: id,
        target: parent_id.to_owned(),
        attributes: Map::from_iter([
            (
                "relation".to_owned(),
                Value::String("rationale_for".to_owned()),
            ),
            (
                "confidence".to_owned(),
                Value::String("EXTRACTED".to_owned()),
            ),
            ("source_file".to_owned(), Value::String(source_file)),
            ("source_location".to_owned(), Value::String(source_location)),
            ("weight".to_owned(), Value::from(1.0)),
        ]),
    });
}

fn source_node_text(node: Node<'_>, source: &[u8]) -> String {
    source
        .get(node.start_byte()..node.end_byte())
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
        .unwrap_or_default()
}

fn collect_js_binding_names(node: Node<'_>, source: &[u8], output: &mut Vec<String>) {
    match node.kind() {
        "identifier" | "shorthand_property_identifier_pattern" => {
            let name = clean_name(source_node_text(node, source));
            if !name.is_empty() {
                output.push(name);
            }
        }
        "pair_pattern" => {
            if let Some(value) = node.child_by_field_name("value") {
                collect_js_binding_names(value, source, output);
            }
        }
        "assignment_pattern" => {
            if let Some(left) = node.child_by_field_name("left") {
                collect_js_binding_names(left, source, output);
            }
        }
        "rest_pattern" => {
            if let Some(argument) = node.child_by_field_name("argument") {
                collect_js_binding_names(argument, source, output);
            }
        }
        "object_pattern" | "array_pattern" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor).filter(|child| child.is_named()) {
                collect_js_binding_names(child, source, output);
            }
        }
        _ => {}
    }
}

fn js_type_only_statement(text: &str, reexport: bool) -> bool {
    let trimmed = text.trim_start();
    if reexport {
        trimmed
            .strip_prefix("export")
            .is_some_and(|rest| rest.trim_start().starts_with("type "))
    } else {
        trimmed
            .strip_prefix("import")
            .is_some_and(|rest| rest.trim_start().starts_with("type "))
    }
}

fn js_type_only_specifier(node: &Node<'_>, source: &[u8]) -> bool {
    source_node_text(*node, source)
        .trim_start()
        .strip_prefix("type")
        .is_some_and(|rest| rest.chars().next().is_some_and(char::is_whitespace))
}

fn js_commonjs_export_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    let spelling = source_node_text(node, source)
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    let name = if spelling == "module.exports" {
        "default"
    } else if let Some(name) = spelling.strip_prefix("module.exports.") {
        name
    } else {
        spelling.strip_prefix("exports.")?
    };
    (!name.is_empty() && name.len() <= 4_096 && !name.contains(['.', '\\', '\0']))
        .then(|| name.to_owned())
}

fn js_top_level_type_names(
    root: Node<'_>,
    config: &GenericConfig,
    source: &[u8],
) -> HashSet<String> {
    let mut names = HashSet::new();
    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        if config.class_types.contains(&child.kind()) {
            if let Some(name) = child.child_by_field_name("name") {
                names.insert(clean_name(source_node_text(name, source)));
            }
            continue;
        }
        if child.kind() != "export_statement" {
            continue;
        }
        let mut export_cursor = child.walk();
        for declaration in child.named_children(&mut export_cursor) {
            if config.class_types.contains(&declaration.kind())
                && let Some(name) = declaration.child_by_field_name("name")
            {
                names.insert(clean_name(source_node_text(name, source)));
            }
        }
    }
    names.remove("");
    names
}

impl<'source, 'tree> ExtractState<'source, 'tree> {
    fn walk_declarations(
        &mut self,
        node: Node<'tree>,
        parent_declaration: Option<(&str, &str, bool, bool)>,
    ) {
        let kind = node.kind();
        if self.config.import_types.contains(&kind) && self.language != "lua" {
            self.add_import(node);
        }

        if self.config.class_types.contains(&kind)
            && let Some(name) = self.declaration_name(node)
        {
            let semantic_scope = parent_declaration.map_or_else(
                || name.clone(),
                |(_, parent_scope, _, _)| format!("{parent_scope}::{name}"),
            );
            let base_id = parent_declaration.map_or_else(
                || make_id(&[&self.stem, &name]),
                |(parent_id, _, _, _)| make_id(&[parent_id, &name]),
            );
            let id = if self.seen_nodes.contains(&base_id) {
                make_id(&[&base_id, "overload", &line(node).to_string()])
            } else {
                base_id
            };
            self.add_node(&id, &name, line(node), true, None);
            let runtime_nested = parent_declaration.is_some_and(|(_, _, _, nested)| nested);
            if self.language == "python"
                && runtime_nested
                && let Some((_, semantic_owner, _, _)) = parent_declaration
                && let Some(declaration) = self
                    .extraction
                    .nodes
                    .iter_mut()
                    .find(|declaration| declaration.id == id)
            {
                declaration.attributes.insert(
                    "lexical_owner".to_owned(),
                    Value::String(semantic_owner.to_owned()),
                );
                declaration.attributes.insert(
                    "qualified_name".to_owned(),
                    Value::String(format!("{semantic_owner}::{name}")),
                );
            }
            self.types.insert(name.clone(), id.clone());
            self.callables.entry(name).or_default().push(id.clone());
            let source = parent_declaration
                .map(|(parent_id, _, _, _)| parent_id)
                .unwrap_or(&self.file_id)
                .to_owned();
            self.add_edge(&source, &id, "contains", line(node), None);
            if self.language == "python" {
                self.add_python_parent_edges(node, &id);
                self.add_python_decorators(node, &id);
            } else if self.language == "scala" {
                self.add_scala_class_references(node, &id);
            }
            if matches!(self.language, "javascript" | "typescript" | "tsx") {
                self.add_js_parent_edges(node, &id);
            }
            if matches!(self.language, "typescript" | "tsx") {
                self.add_ts_class_decorators(node, &id);
            }
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                self.walk_declarations(child, Some((&id, &semantic_scope, true, runtime_nested)));
            }
            return;
        }

        if self.config.function_types.contains(&kind)
            && let Some(name) = self.function_name(node)
        {
            let parent_id = parent_declaration.map(|(parent_id, _, _, _)| parent_id);
            let parent_is_class = parent_declaration.is_some_and(|(_, _, is_class, _)| is_class);
            let semantic_scope = parent_declaration.map_or_else(
                || name.clone(),
                |(_, parent_scope, _, _)| format!("{parent_scope}::{name}"),
            );
            let base_id = parent_declaration.map_or_else(
                || make_id(&[&self.stem, &name]),
                |(parent_id, _, _, _)| make_id(&[parent_id, &name]),
            );
            let id = if self.seen_nodes.contains(&base_id) {
                make_id(&[&base_id, "overload", &line(node).to_string()])
            } else {
                base_id
            };
            let label = if parent_is_class {
                format!(".{name}()")
            } else {
                format!("{name}()")
            };
            self.add_node(&id, &label, line(node), true, None);
            if let Some((_, semantic_owner, _, _)) = parent_declaration
                && let Some(declaration) = self
                    .extraction
                    .nodes
                    .iter_mut()
                    .find(|declaration| declaration.id == id)
            {
                declaration.attributes.insert(
                    "lexical_owner".to_owned(),
                    Value::String(semantic_owner.to_owned()),
                );
                declaration.attributes.insert(
                    "qualified_name".to_owned(),
                    Value::String(format!("{semantic_owner}::{name}")),
                );
            }
            let source = parent_id.unwrap_or(&self.file_id).to_owned();
            self.add_edge(
                &source,
                &id,
                if parent_is_class {
                    "method"
                } else {
                    "contains"
                },
                line(node),
                None,
            );
            if self.language == "python" {
                self.add_python_function_references(node, &id);
                self.add_python_decorators(node, &id);
            } else if self.language == "c" {
                self.add_c_function_references(node, &id);
            } else if self.language == "scala" {
                self.add_scala_function_references(node, &id);
            }
            self.callables.entry(name).or_default().push(id.clone());
            self.functions.push(FunctionBody {
                id: id.clone(),
                node,
                top_level: parent_declaration.is_none()
                    && (self.language != "python"
                        || node
                            .parent()
                            .is_some_and(|parent| parent.kind() == "module")),
            });
            if self.language == "python" {
                let body = node.child_by_field_name("body").unwrap_or(node);
                let mut cursor = body.walk();
                for child in body.children(&mut cursor) {
                    self.walk_declarations(child, Some((&id, &semantic_scope, false, true)));
                }
            }
            return;
        }

        if matches!(self.language, "javascript" | "typescript" | "tsx")
            && parent_declaration.is_none()
            && kind == "assignment_expression"
            && self.add_js_prototype_method(node)
        {
            return;
        }

        if matches!(self.language, "javascript" | "typescript" | "tsx")
            && parent_declaration.is_none()
            && kind == "assignment_expression"
        {
            self.add_js_commonjs_export(node);
        }

        if self.language == "scala"
            && matches!(kind, "val_definition" | "var_definition")
            && let Some((class_id, _, _, _)) = parent_declaration
        {
            self.add_scala_field_reference(node, class_id);
        }

        if matches!(self.language, "javascript" | "typescript" | "tsx")
            && kind == "lexical_declaration"
            && parent_declaration.is_none()
            && node.parent().is_some_and(|parent| {
                parent.kind() == "program"
                    || (parent.kind() == "export_statement"
                        && parent
                            .parent()
                            .is_some_and(|grandparent| grandparent.kind() == "program"))
            })
        {
            self.add_js_module_bindings(node);
            return;
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk_declarations(child, parent_declaration);
        }
    }

    fn walk_function_calls(&mut self) {
        let functions = std::mem::take(&mut self.functions);
        for function in functions {
            if self.language == "python" {
                if function.top_level {
                    self.walk_python_import_calls(function.node, &function.id);
                }
                let bound = python_bound_names(function.node, self.source, false);
                let body = function
                    .node
                    .child_by_field_name("body")
                    .unwrap_or(function.node);
                self.walk_python_indirect(body, &function.id, true, &bound);
                self.walk_calls(body, &function.id, true);
            } else {
                let body = function
                    .node
                    .child_by_field_name("body")
                    .unwrap_or(function.node);
                self.walk_calls(body, &function.id, true);
            }
        }
    }

    fn walk_python_import_calls(&mut self, node: Node<'tree>, caller: &str) {
        if node.kind() == "call"
            && let Some(function) = node.child_by_field_name("function")
            && let Some(name) = if function.kind() == "identifier" {
                self.node_text(function)
                    .map(clean_name)
                    .and_then(|local_name| self.python_import_aliases.get(&local_name).cloned())
            } else {
                None
            }
        {
            let mut extensions = Map::new();
            extensions.insert("symbol_import_use".to_owned(), Value::Bool(true));
            crate::facts::stamp_node_range(&mut extensions, node);
            self.extraction.raw_calls_mut().push(RawCall {
                caller_nid: caller.to_owned(),
                callee: name,
                is_member_call: Some(false),
                source_file: self.source_file.clone(),
                source_location: format!("L{}", line(node)),
                receiver: None,
                receiver_type: None,
                lang: None,
                extensions,
            });
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk_python_import_calls(child, caller);
        }
    }

    fn walk_python_indirect(
        &mut self,
        node: Node<'tree>,
        caller: &str,
        is_root: bool,
        bound: &HashSet<String>,
    ) {
        if !is_root && matches!(node.kind(), "function_definition" | "class_definition") {
            return;
        }
        if caller != self.file_id
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
                    self.add_python_indirect(caller, candidate, "argument", bound);
                }
            }
        }
        if matches!(node.kind(), "dictionary" | "list" | "set" | "tuple") {
            let mut identifiers = Vec::new();
            collect_python_collection_values(node, &mut identifiers);
            for identifier in identifiers {
                self.add_python_indirect(caller, Some(identifier), "collection", bound);
            }
        } else if node.kind() == "assignment"
            && let Some(value) = node.child_by_field_name("right")
        {
            let mut identifiers = Vec::new();
            collect_python_reference_values(value, &mut identifiers);
            for identifier in identifiers {
                self.add_python_indirect(caller, Some(identifier), "assignment", bound);
            }
        } else if node.kind() == "return_statement" {
            let mut cursor = node.walk();
            if let Some(value) = node.children(&mut cursor).find(|child| child.is_named()) {
                let mut identifiers = Vec::new();
                collect_python_reference_values(value, &mut identifiers);
                for identifier in identifiers {
                    self.add_python_indirect(caller, Some(identifier), "return", bound);
                }
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk_python_indirect(child, caller, false, bound);
        }
    }

    fn add_python_indirect(
        &mut self,
        caller: &str,
        node: Option<Node<'tree>>,
        context: &str,
        bound: &HashSet<String>,
    ) {
        let Some(node) = node else {
            return;
        };
        let Some(name) = self.node_text(node).map(clean_name) else {
            return;
        };
        if name.is_empty() || bound.contains(&name) {
            return;
        }
        let mut extensions = Map::new();
        extensions.insert("indirect".to_owned(), Value::Bool(true));
        extensions.insert("context".to_owned(), Value::String(context.to_owned()));
        crate::facts::stamp_node_range(&mut extensions, node);
        self.extraction.raw_calls_mut().push(RawCall {
            caller_nid: caller.to_owned(),
            callee: name,
            is_member_call: Some(false),
            source_file: self.source_file.clone(),
            source_location: format!("L{}", line(node)),
            receiver: None,
            receiver_type: None,
            lang: None,
            extensions,
        });
    }

    fn walk_js_module_indirect(&mut self, node: Node<'tree>, is_root: bool) {
        if !is_root
            && matches!(
                node.kind(),
                "function_declaration"
                    | "function_expression"
                    | "arrow_function"
                    | "generator_function_declaration"
                    | "generator_function"
                    | "class_declaration"
                    | "class"
            )
        {
            return;
        }
        if matches!(node.kind(), "object" | "array") {
            let mut identifiers = Vec::new();
            collect_js_collection_values(node, &mut identifiers);
            for identifier in identifiers {
                if !self.add_js_import_reference(identifier, "collection") {
                    self.add_js_reference(identifier, "collection");
                }
            }
        } else if matches!(node.kind(), "call_expression" | "new_expression")
            && let Some(arguments) = node.child_by_field_name("arguments")
        {
            let mut cursor = arguments.walk();
            for argument in arguments.children(&mut cursor) {
                if argument.kind() == "identifier"
                    && !self.add_js_import_reference(argument, "argument")
                {
                    self.add_js_reference(argument, "argument");
                }
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk_js_module_indirect(child, false);
        }
    }

    fn walk_jsx_references(&mut self, node: Node<'tree>) {
        if matches!(
            node.kind(),
            "jsx_opening_element" | "jsx_self_closing_element"
        ) {
            self.add_jsx_reference(node);
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk_jsx_references(child);
        }
    }

    fn add_jsx_reference(&mut self, node: Node<'tree>) {
        let Some(name_node) = node
            .child_by_field_name("name")
            .or_else(|| first_descendant(node, "jsx_identifier"))
        else {
            return;
        };
        let Some(name) = self.node_text(name_node).map(clean_name) else {
            return;
        };
        if name.is_empty()
            || name
                .chars()
                .next()
                .is_some_and(|character| character.is_ascii_lowercase())
        {
            // Lower-case JSX tags are intrinsic DOM/custom elements, not
            // JavaScript symbol references. Preserve component identity only
            // for capitalized, member, or namespace-qualified tags.
            return;
        }
        let reference = if matches!(
            name_node.kind(),
            "jsx_member_expression" | "member_expression"
        ) {
            name_node
                .child_by_field_name("object")
                .or_else(|| first_identifier(name_node))
                .unwrap_or(name_node)
        } else {
            name_node
        };
        if !self.add_js_import_reference(reference, "jsx") {
            self.add_js_reference(reference, "jsx");
        }
    }

    fn add_js_import_reference(&mut self, node: Node<'tree>, context: &str) -> bool {
        let Some(name) = self.node_text(node).map(clean_name) else {
            return false;
        };
        let Some(binding) = self.js_import_targets.get(&name).cloned() else {
            return false;
        };
        if !self.seen_js_references.insert((
            self.file_id.clone(),
            binding.target.clone(),
            node.start_byte(),
            node.end_byte(),
        )) {
            return true;
        }
        self.add_edge_at(
            &self.file_id.clone(),
            &binding.target,
            "references",
            node,
            Some(context),
        );
        if let Some(edge) = self.extraction.edges.last_mut() {
            edge.attributes
                .insert("binding_name".to_owned(), Value::String(name));
            edge.attributes
                .insert("module".to_owned(), Value::String(binding.module));
            edge.attributes.insert(
                "imported_name".to_owned(),
                Value::String(binding.imported_name),
            );
            if binding.type_only {
                edge.attributes
                    .insert("type_only".to_owned(), Value::Bool(true));
            }
        }
        true
    }

    fn add_js_reference(&mut self, node: Node<'tree>, context: &str) {
        let Some(name) = self.node_text(node).map(clean_name) else {
            return;
        };
        if name.is_empty() {
            return;
        }
        let value_candidates = self.js_value_bindings.get(&name);
        let callable_candidates = self.callables.get(&name);
        let target = match (value_candidates, callable_candidates) {
            (Some(values), None) if values.len() == 1 => values.first().cloned(),
            (None, Some(callables)) if callables.len() == 1 => callables.first().cloned(),
            (Some(values), Some(callables)) if values.len() == 1 && callables.len() == 1 => {
                (values.first() == callables.first()).then(|| values[0].clone())
            }
            _ => None,
        };
        let Some(target) = target else {
            return;
        };
        if target == self.file_id
            || !self.seen_js_references.insert((
                self.file_id.clone(),
                target.clone(),
                node.start_byte(),
                node.end_byte(),
            ))
        {
            return;
        }
        self.add_edge_at(
            &self.file_id.clone(),
            &target,
            "references",
            node,
            Some(context),
        );
    }

    fn walk_calls(&mut self, node: Node<'tree>, caller: &str, is_root: bool) {
        let kind = node.kind();
        if !is_root && self.config.function_boundaries.contains(&kind) {
            return;
        }
        if matches!(self.language, "javascript" | "typescript" | "tsx")
            && kind == "call_expression"
            && self.add_js_dynamic_import(node, caller)
        {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                self.walk_calls(child, caller, false);
            }
            return;
        }
        if self.config.call_types.contains(&kind)
            && let Some(call) = self.call_name(node)
        {
            let candidates = self.callables.get(&call.name).cloned().unwrap_or_default();
            let value_candidates = (!call.member)
                .then(|| self.js_value_bindings.get(&call.name))
                .flatten();
            let value_target = value_candidates
                .filter(|candidates| candidates.len() == 1)
                .and_then(|candidates| candidates.first())
                .cloned();
            let value_is_ambiguous =
                value_candidates.is_some_and(|candidates| candidates.len() > 1);
            let defer_member = call.member
                && call
                    .receiver
                    .as_deref()
                    .is_some_and(|receiver| receiver.starts_with(char::is_uppercase));
            let target = value_target.or_else(|| {
                (!value_is_ambiguous && !defer_member)
                    .then(|| candidates.last().cloned())
                    .flatten()
                    .or_else(|| {
                        (!value_is_ambiguous && (!call.member || self.language == "python"))
                            .then(|| self.types.get(&call.name).cloned())
                            .flatten()
                    })
            });
            if let Some(target) = target.as_ref().filter(|target| {
                target.as_str() != caller
                    && self.seen_resolved_calls.insert((
                        caller.to_owned(),
                        (*target).clone(),
                        node.start_byte(),
                        node.end_byte(),
                    ))
            }) {
                self.add_edge_at(caller, target, "calls", node, Some("call"));
            } else if target.is_none()
                && !is_language_builtin_global(self.language, &call.name)
                && !(self.language == "lua" && (call.member || call.name.contains('.')))
            {
                let mut extensions = crate::facts::node_range(node);
                if self.language == "python"
                    && call.member
                    && let Some(receiver) = call.receiver.as_deref()
                    && let Some(imported) = self
                        .python_import_targets
                        .get(receiver)
                        .filter(|target| !target.is_empty())
                {
                    extensions.insert(
                        "python_qualified_target".to_owned(),
                        Value::String(format!("{imported}.{}", call.name)),
                    );
                }
                self.extraction.raw_calls_mut().push(RawCall {
                    caller_nid: caller.to_owned(),
                    callee: call.name,
                    is_member_call: Some(call.member),
                    source_file: self.source_file.clone(),
                    source_location: format!("L{}", line(node)),
                    receiver: Some(call.receiver),
                    receiver_type: None,
                    lang: None,
                    extensions,
                });
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk_calls(child, caller, false);
        }
    }

    fn add_js_dynamic_import(&mut self, node: Node<'tree>, caller: &str) -> bool {
        let function = node.child_by_field_name("function").or_else(|| {
            let mut cursor = node.walk();
            node.children(&mut cursor).next()
        });
        if function
            .and_then(|function| self.node_text(function))
            .as_deref()
            != Some("import")
        {
            return false;
        }
        let Some(arguments) = node.child_by_field_name("arguments") else {
            return true;
        };
        let mut cursor = arguments.walk();
        for argument in arguments.children(&mut cursor) {
            let raw = if argument.kind() == "template_string" {
                let mut nested = argument.walk();
                if argument
                    .children(&mut nested)
                    .any(|child| child.kind() == "template_substitution")
                {
                    break;
                }
                self.node_text(argument)
                    .map(|value| value.trim_matches('`').to_owned())
            } else if argument.kind() == "string" {
                self.node_text(argument)
                    .map(|value| value.trim_matches(['\'', '"', ' ']).to_owned())
            } else {
                continue;
            };
            let Some(raw) = raw.filter(|value| !value.is_empty()) else {
                break;
            };
            let source_path = Path::new(&self.source_file);
            let target_path = if raw.starts_with('.') {
                resolve_js_import_path(&lexical_normalize(
                    &source_path
                        .parent()
                        .unwrap_or_else(|| Path::new("."))
                        .join(&raw),
                ))
            } else {
                Path::new(&raw).to_path_buf()
            };
            let target = make_id(&[&target_path.to_string_lossy().replace('\\', "/")]);
            if self
                .seen_dynamic_imports
                .insert((caller.to_owned(), target.clone()))
            {
                let mut attributes = Map::new();
                attributes.insert(
                    "relation".to_owned(),
                    Value::String("imports_from".to_owned()),
                );
                attributes.insert("context".to_owned(), Value::String("import".to_owned()));
                attributes.insert("deferred".to_owned(), Value::Bool(true));
                attributes.insert(
                    "confidence".to_owned(),
                    Value::String("EXTRACTED".to_owned()),
                );
                attributes.insert(
                    "source_file".to_owned(),
                    Value::String(self.source_file.clone()),
                );
                attributes.insert(
                    "source_location".to_owned(),
                    Value::String(format!("L{}", line(node))),
                );
                attributes.insert("weight".to_owned(), Value::from(1.0));
                attributes.insert(
                    "target_file".to_owned(),
                    Value::String(target_path.to_string_lossy().into_owned()),
                );
                self.extraction.edges.push(EdgeRecord {
                    source: caller.to_owned(),
                    target,
                    attributes,
                });
                if let Some(edge) = self.extraction.edges.last_mut() {
                    crate::facts::stamp_node_range(&mut edge.attributes, node);
                }
            }
            break;
        }
        true
    }

    fn declaration_name(&self, node: Node<'tree>) -> Option<String> {
        node.child_by_field_name("name")
            .and_then(|name| self.node_text(name))
            .or_else(|| {
                self.config.name_fallbacks.iter().find_map(|kind| {
                    first_descendant(node, kind).and_then(|name| self.node_text(name))
                })
            })
            .or_else(|| first_identifier(node).and_then(|name| self.node_text(name)))
            .map(clean_name)
            .filter(|name| !name.is_empty())
    }

    fn function_name(&self, node: Node<'tree>) -> Option<String> {
        if self.language == "c" {
            return self
                .c_function_name(node)
                .or_else(|| self.declaration_name(node));
        }
        self.declaration_name(node).or_else(|| {
            node.child_by_field_name("declarator")
                .and_then(first_identifier)
                .and_then(|name| self.node_text(name))
                .map(clean_name)
        })
    }

    fn c_function_name(&self, node: Node<'tree>) -> Option<String> {
        let declarator = node.child_by_field_name("declarator")?;
        let name = c_declarator_name(declarator)?;
        self.node_text(name)
            .map(clean_name)
            .filter(|name| !name.is_empty())
    }

    fn call_name(&self, node: Node<'tree>) -> Option<CallName> {
        let function = if self.config.call_function_field.is_empty() {
            None
        } else {
            node.child_by_field_name(self.config.call_function_field)
        }
        .or_else(|| node.child_by_field_name("name"))
        .or_else(|| node.child_by_field_name("type"))
        .or_else(|| first_identifier(node))?;
        let function_kind = function.kind();
        let member = self.config.accessor_types.contains(&function_kind);
        let name_node = if member && !self.config.accessor_name_field.is_empty() {
            function
                .child_by_field_name(self.config.accessor_name_field)
                .or_else(|| last_identifier(function))
                .unwrap_or(function)
        } else if member {
            last_identifier(function).unwrap_or(function)
        } else {
            function
        };
        let name = self.node_text(name_node).map(clean_name)?;
        if name.is_empty() {
            return None;
        }
        let receiver = if member && !self.config.accessor_object_field.is_empty() {
            function
                .child_by_field_name(self.config.accessor_object_field)
                .and_then(|receiver| self.node_text(receiver))
                .map(clean_name)
        } else {
            None
        };
        Some(CallName {
            name,
            member,
            receiver,
        })
    }

    fn add_import(&mut self, node: Node<'tree>) {
        if self.language == "python" {
            self.add_python_import(node);
            return;
        }
        if self.language == "scala" {
            let mut cursor = node.walk();
            if let Some(target_node) = node
                .children(&mut cursor)
                .find(|child| matches!(child.kind(), "stable_id" | "identifier"))
            {
                let raw = self.node_text(target_node).unwrap_or_default();
                let target = raw
                    .rsplit('.')
                    .next()
                    .unwrap_or_default()
                    .trim_matches(['{', '}', ' ']);
                if !target.is_empty() && target != "_" {
                    self.add_edge(
                        &self.file_id.clone(),
                        &make_id(&[target]),
                        "imports",
                        line(node),
                        Some("import"),
                    );
                }
            }
            return;
        }
        let text = self.node_text(node).unwrap_or_default();
        if matches!(self.language, "javascript" | "typescript" | "tsx")
            && matches!(node.kind(), "import_statement" | "export_statement")
        {
            self.add_js_import(node);
            return;
        }
        if self.language == "c"
            && let Some(path) = node.child_by_field_name("path")
            && path.kind() != "system_lib_string"
            && let Some(raw) = self.node_text(path)
            && let clean = raw.trim_matches(['<', '>', '\'', '"', ' '])
            && !clean.is_empty()
            && let Some(parent) = Path::new(&self.source_file).parent()
            && let Ok(resolved) = fs::canonicalize(parent.join(clean))
            && resolved.is_file()
        {
            self.add_edge(
                &self.file_id.clone(),
                &make_id(&[&resolved.to_string_lossy()]),
                "imports",
                line(node),
                Some("import"),
            );
            return;
        }
        let target = quoted_value(&text)
            .or_else(|| angle_value(&text))
            .or_else(|| {
                last_identifier(node)
                    .and_then(|identifier| self.node_text(identifier))
                    .map(clean_name)
            })
            .unwrap_or_default();
        let target = target
            .rsplit(['/', ':'])
            .next()
            .unwrap_or_default()
            .trim_matches(['\'', '"', '>', '<', ';'])
            .to_owned();
        let target = if matches!(self.language, "c" | "cpp" | "objc") {
            Path::new(&target)
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or(&target)
                .to_owned()
        } else {
            target.rsplit('.').next().unwrap_or(&target).to_owned()
        };
        if !target.is_empty() {
            let target_id = make_id(&[&target]);
            self.add_edge(
                &self.file_id.clone(),
                &target_id,
                "imports",
                line(node),
                Some("import"),
            );
        }
    }

    fn add_python_import(&mut self, node: Node<'tree>) {
        if node.kind() == "import_statement" {
            let mut cursor = node.walk();
            let imports = node
                .children(&mut cursor)
                .filter(|child| matches!(child.kind(), "dotted_name" | "aliased_import"))
                .filter_map(|child| self.node_text(child))
                .map(|raw| {
                    raw.split(" as ")
                        .next()
                        .unwrap_or_default()
                        .trim()
                        .trim_start_matches('.')
                        .to_owned()
                })
                .filter(|module| !module.is_empty())
                .collect::<Vec<_>>();
            for module in imports {
                self.add_edge(
                    &self.file_id.clone(),
                    &make_id(&[&module]),
                    "imports",
                    line(node),
                    Some("import"),
                );
            }
            return;
        }
        let Some(module_node) = node.child_by_field_name("module_name") else {
            return;
        };
        let Some(raw) = self.node_text(module_node) else {
            return;
        };
        let target = if raw.starts_with('.') {
            let dots = raw.len().saturating_sub(raw.trim_start_matches('.').len());
            let module = raw.trim_start_matches('.');
            let mut base = Path::new(&self.source_file)
                .parent()
                .unwrap_or_else(|| Path::new("."));
            for _ in 1..dots {
                base = base.parent().unwrap_or(base);
            }
            let relative = if module.is_empty() {
                "__init__.py".to_owned()
            } else {
                format!("{}.py", module.replace('.', "/"))
            };
            make_id(&[&base.join(relative).to_string_lossy()])
        } else {
            make_id(&[&raw])
        };
        self.add_edge(
            &self.file_id.clone(),
            &target,
            "imports_from",
            line(node),
            Some("import"),
        );
        let mut bindings = Map::new();
        for (_, local) in python_import_entries(node, self.source) {
            if let Some(qualified) = self.python_import_targets.get(&local) {
                bindings.insert(local, Value::String(qualified.clone()));
            }
        }
        if !bindings.is_empty()
            && let Some(edge) = self.extraction.edges.last_mut()
        {
            edge.attributes
                .insert("python_imports".to_owned(), Value::Object(bindings));
        }
    }

    fn add_js_import(&mut self, node: Node<'tree>) {
        let is_reexport = node.kind() == "export_statement";
        let statement_text = self.node_text(node).unwrap_or_default();
        let statement_type_only = js_type_only_statement(&statement_text, is_reexport);
        let mut cursor = node.walk();
        let Some(module_node) = node
            .children(&mut cursor)
            .find(|child| child.kind() == "string")
        else {
            return;
        };
        let Some(raw_module) = self
            .node_text(module_node)
            .map(|value| value.trim_matches(['\'', '"', '`', ' ']).to_owned())
        else {
            return;
        };
        let source_path = Path::new(&self.source_file);
        let target_path = raw_module.starts_with('.').then(|| {
            resolve_js_import_path(&lexical_normalize(
                &source_path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join(&raw_module),
            ))
        });
        let module_id = if let Some(target_path) = &target_path {
            make_id(&[&target_path.to_string_lossy().replace('\\', "/")])
        } else {
            make_id(&["ref", &raw_module])
        };
        self.add_edge(
            &self.file_id.clone(),
            &module_id,
            "imports_from",
            line(node),
            Some(if is_reexport { "re-export" } else { "import" }),
        );
        if let Some(edge) = self.extraction.edges.last_mut() {
            edge.attributes
                .insert("module".to_owned(), Value::String(raw_module.clone()));
            if statement_type_only {
                edge.attributes
                    .insert("type_only".to_owned(), Value::Bool(true));
            }
            if let Some(target_path) = &target_path {
                edge.attributes.insert(
                    "target_file".to_owned(),
                    Value::String(target_path.to_string_lossy().into_owned()),
                );
            }
        }
        let target_stem = target_path
            .as_deref()
            .map(file_stem)
            .unwrap_or_else(|| raw_module.clone());
        if is_reexport {
            let Some(clause) = first_descendant(node, "export_clause") else {
                return;
            };
            let mut specifiers = Vec::new();
            collect_nodes_of_kind(clause, "export_specifier", &mut specifiers);
            for specifier in specifiers {
                let Some(name) = specifier
                    .child_by_field_name("name")
                    .and_then(|name| self.node_text(name))
                    .map(clean_name)
                else {
                    continue;
                };
                if name.is_empty() || name == "default" {
                    continue;
                }
                self.add_edge(
                    &self.file_id.clone(),
                    &make_id(&[&target_stem, &name]),
                    "re_exports",
                    line(node),
                    Some("re-export"),
                );
                if let Some(edge) = self.extraction.edges.last_mut() {
                    if statement_type_only || js_type_only_specifier(&specifier, self.source) {
                        edge.attributes
                            .insert("type_only".to_owned(), Value::Bool(true));
                    }
                    edge.attributes
                        .insert("exported_name".to_owned(), Value::String(name.clone()));
                }
            }
        } else if let Some(clause) = first_descendant(node, "import_clause") {
            let mut bindings = Vec::new();
            let mut clause_cursor = clause.walk();
            for child in clause.named_children(&mut clause_cursor) {
                match child.kind() {
                    // `import Foo from "..."`.
                    "identifier" => {
                        let Some(local_name) = self.node_text(child).map(clean_name) else {
                            continue;
                        };
                        if !local_name.is_empty() {
                            bindings.push((
                                local_name,
                                "default".to_owned(),
                                statement_type_only,
                                child,
                            ));
                        }
                    }
                    // `import * as Foo from "..."`.
                    "namespace_import" => {
                        let Some(local_node) = child
                            .child_by_field_name("name")
                            .or_else(|| first_identifier(child))
                        else {
                            continue;
                        };
                        let Some(local_name) = self.node_text(local_node).map(clean_name) else {
                            continue;
                        };
                        if !local_name.is_empty() {
                            bindings.push((
                                local_name,
                                "*".to_owned(),
                                statement_type_only,
                                local_node,
                            ));
                        }
                    }
                    "named_imports" => {
                        let mut specifiers = Vec::new();
                        collect_nodes_of_kind(child, "import_specifier", &mut specifiers);
                        for specifier in specifiers {
                            let Some(imported_name) = specifier
                                .child_by_field_name("name")
                                .or_else(|| first_identifier(specifier))
                                .and_then(|name| self.node_text(name))
                                .map(clean_name)
                            else {
                                continue;
                            };
                            if imported_name.is_empty() {
                                continue;
                            }
                            let local_name = specifier
                                .child_by_field_name("alias")
                                .or_else(|| {
                                    let identifiers = direct_named_children(specifier)
                                        .into_iter()
                                        .filter(|node| node.kind() == "identifier")
                                        .collect::<Vec<_>>();
                                    (identifiers.len() > 1).then(|| identifiers[1])
                                })
                                .and_then(|alias| self.node_text(alias))
                                .map(clean_name)
                                .filter(|alias| !alias.is_empty())
                                .unwrap_or_else(|| imported_name.clone());
                            bindings.push((
                                local_name,
                                imported_name,
                                statement_type_only
                                    || js_type_only_specifier(&specifier, self.source),
                                specifier,
                            ));
                        }
                    }
                    _ => {}
                }
            }
            for (local_name, imported_name, type_only, binding_node) in bindings {
                let target = if imported_name == "*" {
                    module_id.clone()
                } else if imported_name == "default" {
                    make_id(&[&target_stem, "default"])
                } else {
                    make_id(&[&target_stem, &imported_name])
                };
                self.js_import_targets.insert(
                    local_name.clone(),
                    JsImportTarget {
                        target: target.clone(),
                        module: raw_module.clone(),
                        imported_name: imported_name.clone(),
                        type_only,
                    },
                );
                self.add_edge(
                    &self.file_id.clone(),
                    &target,
                    "imports",
                    line(node),
                    Some("import"),
                );
                if let Some(edge) = self.extraction.edges.last_mut() {
                    edge.attributes
                        .insert("module".to_owned(), Value::String(raw_module.clone()));
                    edge.attributes
                        .insert("imported_name".to_owned(), Value::String(imported_name));
                    edge.attributes
                        .insert("local_name".to_owned(), Value::String(local_name));
                    edge.attributes.insert(
                        "import_kind".to_owned(),
                        Value::String(
                            if target == module_id {
                                "namespace"
                            } else if edge.string("imported_name") == "default" {
                                "default"
                            } else {
                                "named"
                            }
                            .to_owned(),
                        ),
                    );
                    if type_only {
                        edge.attributes
                            .insert("type_only".to_owned(), Value::Bool(true));
                    }
                    if let Some(target_path) = &target_path {
                        edge.attributes.insert(
                            "target_file".to_owned(),
                            Value::String(target_path.to_string_lossy().into_owned()),
                        );
                    }
                }
                // Keep the exact binding anchor available to downstream
                // diagnostics even when the import target is a module file
                // (namespace imports) rather than a declaration.
                if let Some(edge) = self.extraction.edges.last_mut() {
                    crate::facts::stamp_node_range(&mut edge.attributes, binding_node);
                }
            }
        }
    }

    fn add_js_module_bindings(&mut self, node: Node<'tree>) {
        let mut cursor = node.walk();
        let declarations: Vec<_> = node
            .children(&mut cursor)
            .filter(|child| child.kind() == "variable_declarator")
            .collect();
        for declaration in declarations {
            let Some(name_node) = declaration.child_by_field_name("name") else {
                continue;
            };
            let Some(value) = declaration.child_by_field_name("value") else {
                continue;
            };
            self.add_js_require_import(declaration, name_node, value);
            let mut names = Vec::new();
            collect_js_binding_names(name_node, self.source, &mut names);
            names.sort();
            names.dedup();
            if names.is_empty() {
                continue;
            }
            if name_node.kind() == "identifier"
                && matches!(
                    value.kind(),
                    "arrow_function" | "function_expression" | "function"
                )
            {
                let name = &names[0];
                let id = self.js_value_binding_id(name, line(declaration));
                self.add_node(&id, &format!("{name}()"), line(declaration), true, None);
                self.add_edge(
                    &self.file_id.clone(),
                    &id,
                    "contains",
                    line(declaration),
                    None,
                );
                self.callables
                    .entry(name.clone())
                    .or_default()
                    .push(id.clone());
                self.js_value_bindings
                    .entry(name.clone())
                    .or_default()
                    .push(id.clone());
                self.functions.push(FunctionBody {
                    id,
                    node: value,
                    top_level: true,
                });
            } else if matches!(
                value.kind(),
                "object" | "array" | "as_expression" | "call_expression" | "new_expression"
            ) {
                for name in names {
                    let id = self.js_value_binding_id(&name, line(declaration));
                    self.add_node(&id, &name, line(declaration), false, None);
                    if let Some(binding) = self
                        .extraction
                        .nodes
                        .iter_mut()
                        .find(|binding| binding.id == id)
                    {
                        binding.attributes.insert(
                            "symbol_kind".to_owned(),
                            Value::String("variable".to_owned()),
                        );
                    }
                    self.add_edge(
                        &self.file_id.clone(),
                        &id,
                        "contains",
                        line(declaration),
                        None,
                    );
                    self.js_value_bindings.entry(name).or_default().push(id);
                }
            }
        }
    }

    fn js_value_binding_id(&self, name: &str, at: usize) -> String {
        let ordinary = make_id(&[&self.stem, name]);
        if !self.js_type_namespace_names.contains(name) && !self.seen_nodes.contains(&ordinary) {
            return ordinary;
        }
        let value = make_id(&[&self.stem, name, "value"]);
        if !self.seen_nodes.contains(&value) {
            value
        } else {
            make_id(&[&value, "overload", &at.to_string()])
        }
    }

    fn add_js_require_import(
        &mut self,
        declaration: Node<'tree>,
        name_node: Node<'tree>,
        value: Node<'tree>,
    ) -> bool {
        let Some(call) = find_require_call(value, self.source) else {
            return false;
        };
        let Some(arguments) = call.child_by_field_name("arguments") else {
            return false;
        };
        let mut cursor = arguments.walk();
        let Some(module_node) = arguments
            .children(&mut cursor)
            .find(|child| child.kind() == "string")
        else {
            return false;
        };
        let Some(raw_module) = self
            .node_text(module_node)
            .map(|value| value.trim_matches(['\'', '"', '`', ' ']).to_owned())
            .filter(|value| !value.is_empty())
        else {
            return false;
        };
        let source_path = Path::new(&self.source_file);
        let target_path = if raw_module.starts_with('.') {
            resolve_js_import_path(&lexical_normalize(
                &source_path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join(&raw_module),
            ))
        } else {
            Path::new(&raw_module).to_path_buf()
        };
        let module_id = make_id(&[&target_path.to_string_lossy().replace('\\', "/")]);
        self.add_edge(
            &self.file_id.clone(),
            &module_id,
            "imports_from",
            line(declaration),
            Some("require"),
        );
        if let Some(edge) = self.extraction.edges.last_mut() {
            edge.attributes
                .insert("module".to_owned(), Value::String(raw_module.clone()));
            edge.attributes.insert(
                "target_file".to_owned(),
                Value::String(target_path.to_string_lossy().into_owned()),
            );
        }

        let mut symbols = Vec::new();
        if name_node.kind() == "object_pattern" {
            let mut cursor = name_node.walk();
            for property in name_node.children(&mut cursor) {
                let symbol_node = match property.kind() {
                    "shorthand_property_identifier_pattern" => Some(property),
                    "pair_pattern" => property.child_by_field_name("key"),
                    _ => None,
                };
                if let Some(symbol) = symbol_node
                    .and_then(|node| self.node_text(node))
                    .map(clean_name)
                    .filter(|name| !name.is_empty())
                {
                    symbols.push(symbol);
                }
            }
        } else if value.kind() == "member_expression"
            && let Some(symbol) = value
                .child_by_field_name("property")
                .and_then(|node| self.node_text(node))
                .map(clean_name)
                .filter(|name| !name.is_empty())
        {
            symbols.push(symbol);
        }
        let target_stem = file_stem(&target_path);
        for symbol in symbols {
            self.add_edge(
                &self.file_id.clone(),
                &make_id(&[&target_stem, &symbol]),
                "imports",
                line(declaration),
                Some("require"),
            );
        }
        true
    }

    fn add_ts_class_decorators(&mut self, node: Node<'tree>, class_id: &str) {
        let mut decorators = Vec::new();
        let mut cursor = node.walk();
        decorators.extend(
            node.children(&mut cursor)
                .filter(|child| child.kind() == "decorator"),
        );
        if let Some(parent) = node
            .parent()
            .filter(|parent| parent.kind() == "export_statement")
        {
            let mut cursor = parent.walk();
            for child in parent.children(&mut cursor) {
                if child.kind() == "decorator" {
                    decorators.push(child);
                } else if matches!(
                    child.kind(),
                    "class_declaration" | "abstract_class_declaration"
                ) {
                    break;
                }
            }
        }
        for decorator in decorators {
            let mut cursor = decorator.walk();
            let Some(mut target) = decorator
                .children(&mut cursor)
                .find(|child| child.is_named())
            else {
                continue;
            };
            if target.kind() == "call_expression" {
                target = target.child_by_field_name("function").unwrap_or(target);
            }
            if target.kind() == "member_expression" {
                let Some(property) = target.child_by_field_name("property") else {
                    continue;
                };
                target = property;
            }
            if target.kind() != "identifier" {
                continue;
            }
            let Some(name) = self
                .node_text(target)
                .map(clean_name)
                .filter(|name| !name.is_empty())
            else {
                continue;
            };
            let target_id = self.ensure_type_node(&name, true);
            if target_id != class_id {
                self.add_edge(
                    class_id,
                    &target_id,
                    "references",
                    line(decorator),
                    Some("decorator"),
                );
            }
        }
    }

    fn add_js_parent_edges(&mut self, node: Node<'tree>, class_id: &str) {
        let mut clauses = Vec::new();
        if let Some(heritage) = direct_named_children(node)
            .into_iter()
            .find(|child| child.kind() == "class_heritage")
        {
            let nested = direct_named_children(heritage);
            if nested
                .iter()
                .any(|child| matches!(child.kind(), "extends_clause" | "implements_clause"))
            {
                clauses.extend(nested);
            } else {
                clauses.push(heritage);
            }
        }
        clauses.extend(
            direct_named_children(node)
                .into_iter()
                .filter(|child| child.kind() == "extends_type_clause"),
        );

        for clause in clauses {
            let relation = if clause.kind() == "implements_clause" {
                "implements"
            } else {
                "inherits"
            };
            let context = if relation == "implements" {
                "implements"
            } else {
                "extends"
            };
            let mut cursor = clause.walk();
            let targets = match clause.kind() {
                "extends_clause" => clause
                    .children_by_field_name("value", &mut cursor)
                    .collect::<Vec<_>>(),
                "extends_type_clause" => clause
                    .children_by_field_name("type", &mut cursor)
                    .collect::<Vec<_>>(),
                _ => clause
                    .children(&mut cursor)
                    .filter(|child| child.is_named())
                    .collect::<Vec<_>>(),
            };
            for target_node in targets {
                let Some(qualified_name) = js_heritage_name(target_node, self.source) else {
                    continue;
                };
                let spelling = qualified_name
                    .rsplit('.')
                    .next()
                    .unwrap_or(&qualified_name)
                    .trim();
                if spelling.is_empty() {
                    continue;
                }
                let target = self
                    .js_import_targets
                    .get(spelling)
                    .map(|binding| binding.target.clone())
                    .unwrap_or_else(|| self.ensure_type_node(spelling, true));
                if target == class_id {
                    continue;
                }
                self.add_edge_at(class_id, &target, relation, target_node, Some(context));
                if qualified_name != spelling
                    && let Some(edge) = self.extraction.edges.last_mut()
                {
                    edge.attributes.insert(
                        "target_qualified_name".to_owned(),
                        Value::String(qualified_name),
                    );
                }
            }
        }
    }

    fn add_python_parent_edges(&mut self, node: Node<'tree>, class_id: &str) {
        let Some(superclasses) = node.child_by_field_name("superclasses") else {
            return;
        };
        let mut cursor = superclasses.walk();
        for superclass in superclasses
            .children(&mut cursor)
            .filter(|child| child.kind() == "identifier")
        {
            let Some(text) = self.node_text(superclass) else {
                continue;
            };
            let name = text;
            if name.is_empty() {
                continue;
            }
            let target = self.ensure_type_node(&name, true);
            self.add_edge(class_id, &target, "inherits", line(node), None);
            if let Some(qualified) = self.python_import_targets.get(&name)
                && let Some(edge) = self.extraction.edges.last_mut()
            {
                edge.attributes.insert(
                    "target_qualified_name".to_owned(),
                    Value::String(qualified.clone()),
                );
            }
        }
    }

    fn add_python_decorators(&mut self, node: Node<'tree>, owner_id: &str) {
        let Some(decorated) = node
            .parent()
            .filter(|parent| parent.kind() == "decorated_definition")
        else {
            return;
        };
        let mut cursor = decorated.walk();
        let decorators = decorated
            .children(&mut cursor)
            .take_while(|child| child.id() != node.id())
            .filter(|child| child.kind() == "decorator")
            .collect::<Vec<_>>();
        for decorator in decorators {
            let mut inner = decorator.walk();
            let Some(mut expression) = decorator
                .children(&mut inner)
                .find(|child| child.is_named())
            else {
                continue;
            };
            if expression.kind() == "call" {
                expression = expression
                    .child_by_field_name("function")
                    .unwrap_or(expression);
            }
            let Some(spelling) = self
                .node_text(expression)
                .map(|text| text.trim().to_owned())
                .filter(|text| !text.is_empty())
            else {
                continue;
            };
            let name = spelling.rsplit('.').next().unwrap_or_default().trim();
            if name.is_empty() {
                continue;
            }
            let target = self.ensure_type_node(name, true);
            if target == owner_id {
                continue;
            }
            self.add_edge(
                owner_id,
                &target,
                "references",
                line(decorator),
                Some("decorator"),
            );
            let qualified = self
                .python_import_targets
                .get(&spelling)
                .cloned()
                .or_else(|| {
                    let (root, suffix) = spelling.split_once('.')?;
                    let imported = self.python_import_targets.get(root)?;
                    Some(format!("{imported}.{suffix}"))
                });
            if let Some(edge) = self.extraction.edges.last_mut() {
                if let Some(qualified) = qualified {
                    edge.attributes
                        .insert("target_qualified_name".to_owned(), Value::String(qualified));
                }
                crate::facts::stamp_node_range(&mut edge.attributes, decorator);
            }
        }
    }

    fn add_scala_class_references(&mut self, node: Node<'tree>, class_id: &str) {
        let extends = node
            .child_by_field_name("extend")
            .or_else(|| first_descendant(node, "extends_clause"));
        if let Some(extends) = extends {
            let mut bases = Vec::new();
            let mut cursor = extends.walk();
            for child in extends.children(&mut cursor) {
                let name_node = if child.kind() == "type_identifier" {
                    Some(child)
                } else if child.kind() == "generic_type" {
                    child
                        .child_by_field_name("type")
                        .or_else(|| first_descendant(child, "type_identifier"))
                } else {
                    None
                };
                if let Some(name) = name_node
                    .and_then(|name| self.node_text(name))
                    .map(clean_name)
                {
                    bases.push((name, line(child)));
                }
            }
            for (index, (name, at)) in bases.into_iter().enumerate() {
                let target = self.ensure_type_node(&name, true);
                if target != class_id {
                    self.add_edge(
                        class_id,
                        &target,
                        if index == 0 { "inherits" } else { "mixes_in" },
                        at,
                        None,
                    );
                }
            }
        }

        let mut parameters = Vec::new();
        collect_nodes_of_kind(node, "class_parameter", &mut parameters);
        for parameter in parameters {
            if let Some(type_node) = parameter.child_by_field_name("type") {
                let mut refs = Vec::new();
                collect_scala_type_refs(type_node, self.source, false, &mut refs);
                self.add_scala_type_references(class_id, &refs, "field", line(parameter));
            }
        }
    }

    fn add_scala_field_reference(&mut self, node: Node<'tree>, class_id: &str) {
        let Some(type_node) = node.child_by_field_name("type") else {
            return;
        };
        let mut refs = Vec::new();
        collect_scala_type_refs(type_node, self.source, false, &mut refs);
        self.add_scala_type_references(class_id, &refs, "field", line(node));
    }

    fn add_scala_function_references(&mut self, node: Node<'tree>, function_id: &str) {
        if let Some(parameters) = first_descendant(node, "parameters") {
            let mut cursor = parameters.walk();
            for parameter in parameters
                .children(&mut cursor)
                .filter(|child| child.kind() == "parameter")
            {
                if let Some(type_node) = parameter.child_by_field_name("type") {
                    let mut refs = Vec::new();
                    collect_scala_type_refs(type_node, self.source, false, &mut refs);
                    self.add_scala_type_references(
                        function_id,
                        &refs,
                        "parameter_type",
                        line(node),
                    );
                }
            }
        }
        if let Some(return_type) = node.child_by_field_name("return_type") {
            let mut refs = Vec::new();
            collect_scala_type_refs(return_type, self.source, false, &mut refs);
            self.add_scala_type_references(function_id, &refs, "return_type", line(node));
        }
    }

    fn add_scala_type_references(
        &mut self,
        source: &str,
        refs: &[(String, bool)],
        context: &str,
        at: usize,
    ) {
        for (name, generic) in refs {
            let target = self.ensure_type_node(name, true);
            if target != source {
                self.add_edge(
                    source,
                    &target,
                    "references",
                    at,
                    Some(if *generic { "generic_arg" } else { context }),
                );
            }
        }
    }

    fn add_c_function_references(&mut self, node: Node<'tree>, function_id: &str) {
        if let Some(return_type) = node.child_by_field_name("type") {
            let mut names = Vec::new();
            collect_c_type_names(return_type, self.source, &mut names);
            self.add_c_type_references(function_id, &names, "return_type", line(node));
        }
        let mut declarator = node.child_by_field_name("declarator");
        while declarator.is_some_and(|candidate| {
            matches!(
                candidate.kind(),
                "pointer_declarator" | "reference_declarator"
            )
        }) {
            declarator =
                declarator.and_then(|candidate| candidate.child_by_field_name("declarator"));
        }
        if let Some(parameters) = declarator
            .filter(|candidate| candidate.kind() == "function_declarator")
            .and_then(|candidate| candidate.child_by_field_name("parameters"))
        {
            let mut cursor = parameters.walk();
            for parameter in parameters
                .children(&mut cursor)
                .filter(|candidate| candidate.kind() == "parameter_declaration")
            {
                if let Some(type_node) = parameter.child_by_field_name("type") {
                    let mut names = Vec::new();
                    collect_c_type_names(type_node, self.source, &mut names);
                    self.add_c_type_references(function_id, &names, "parameter_type", line(node));
                }
            }
        }
    }

    fn add_c_type_references(
        &mut self,
        function_id: &str,
        names: &[String],
        context: &str,
        line: usize,
    ) {
        for name in names {
            let target = self.ensure_type_node(name, true);
            self.add_edge(function_id, &target, "references", line, Some(context));
        }
    }

    fn add_python_function_references(&mut self, node: Node<'tree>, function_id: &str) {
        if let Some(parameters) = node.child_by_field_name("parameters") {
            let mut cursor = parameters.walk();
            for parameter in parameters.children(&mut cursor).filter(|parameter| {
                matches!(
                    parameter.kind(),
                    "typed_parameter" | "typed_default_parameter"
                )
            }) {
                let mut references = Vec::new();
                collect_python_type_references(
                    parameter.child_by_field_name("type"),
                    self.source,
                    false,
                    &mut references,
                );
                self.emit_python_type_references(
                    function_id,
                    references,
                    "parameter_type",
                    line(node),
                );
            }
        }
        let mut references = Vec::new();
        collect_python_type_references(
            node.child_by_field_name("return_type"),
            self.source,
            false,
            &mut references,
        );
        self.emit_python_type_references(function_id, references, "return_type", line(node));
    }

    fn emit_python_type_references(
        &mut self,
        function_id: &str,
        references: Vec<(String, bool)>,
        ordinary_context: &str,
        line: usize,
    ) {
        for (name, generic) in references {
            let target = self.ensure_type_node(&name, true);
            if target != function_id {
                self.add_edge(
                    function_id,
                    &target,
                    "references",
                    line,
                    Some(if generic {
                        "generic_arg"
                    } else {
                        ordinary_context
                    }),
                );
            }
        }
    }

    fn ensure_type_node(&mut self, name: &str, origin_file: bool) -> String {
        if let Some(id) = self.types.get(name) {
            return id.clone();
        }
        let local_id = make_id(&[&self.stem, name]);
        if self.seen_nodes.contains(&local_id) {
            return local_id;
        }
        let id = make_id(&[name]);
        if self.seen_nodes.insert(id.clone()) {
            let mut attributes = Map::new();
            attributes.insert("label".to_owned(), Value::String(name.to_owned()));
            attributes.insert("file_type".to_owned(), Value::String("code".to_owned()));
            // This helper is used only for type references discovered while
            // extracting a source file.  Emit the closed semantic kind up
            // front so publication can retain the evidence even when the
            // declaration lives in another file or dependency.  The resolver
            // still marks the resulting source-less node as inferred and
            // requires an exact wiring site; no declaration is fabricated.
            attributes.insert(
                "symbol_kind".to_owned(),
                Value::String("type_alias".to_owned()),
            );
            attributes.insert("source_file".to_owned(), Value::String(String::new()));
            attributes.insert("source_location".to_owned(), Value::String(String::new()));
            if origin_file {
                attributes.insert(
                    "origin_file".to_owned(),
                    Value::String(self.source_file.clone()),
                );
            }
            self.extraction.nodes.push(NodeRecord {
                id: id.clone(),
                attributes,
            });
        }
        self.types
            .entry(name.to_owned())
            .or_insert_with(|| id.clone());
        id
    }

    fn add_js_prototype_method(&mut self, node: Node<'tree>) -> bool {
        let Some(left) = node.child_by_field_name("left") else {
            return false;
        };
        let Some(right) = node.child_by_field_name("right") else {
            return false;
        };
        if !matches!(
            right.kind(),
            "function_expression" | "arrow_function" | "generator_function"
        ) {
            return false;
        }
        let Some(owner) = left
            .child_by_field_name("object")
            .and_then(|owner| self.node_text(owner))
            .map(|owner| owner.trim().to_owned())
            .filter(|owner| owner.contains(".prototype") || owner.ends_with(".fn"))
        else {
            return false;
        };
        let Some(name) = left
            .child_by_field_name("property")
            .or_else(|| last_identifier(left))
            .and_then(|name| self.node_text(name))
            .map(clean_name)
            .filter(|name| !name.is_empty())
        else {
            return false;
        };
        let id = make_id(&[&self.stem, &owner, &name]);
        let label = format!(".{name}()");
        self.add_node(&id, &label, line(node), true, None);
        if let Some(method) = self
            .extraction
            .nodes
            .iter_mut()
            .find(|method| method.id == id)
        {
            method
                .attributes
                .insert("lexical_owner".to_owned(), Value::String(owner.clone()));
            method.attributes.insert(
                "qualified_name".to_owned(),
                Value::String(format!("{owner}::{name}")),
            );
        }
        let owner_name = owner
            .strip_suffix(".prototype")
            .or_else(|| owner.strip_suffix(".fn"))
            .and_then(|owner| owner.rsplit('.').next())
            .unwrap_or_default();
        let containment_source = self
            .types
            .get(owner_name)
            .cloned()
            .or_else(|| {
                self.callables
                    .get(owner_name)
                    .and_then(|owners| owners.last())
                    .cloned()
            })
            .unwrap_or_else(|| self.file_id.clone());
        self.add_edge(
            &containment_source,
            &id,
            "contains",
            line(node),
            Some("prototype_method"),
        );
        self.callables.entry(name).or_default().push(id.clone());
        self.functions.push(FunctionBody {
            id,
            node: right,
            top_level: false,
        });
        true
    }

    fn add_js_commonjs_export(&mut self, node: Node<'tree>) -> bool {
        let Some(left) = node.child_by_field_name("left") else {
            return false;
        };
        let Some(export_name) = js_commonjs_export_name(left, self.source) else {
            return false;
        };
        let Some(value) = node.child_by_field_name("right") else {
            return false;
        };

        let mut bindings = Vec::new();
        if export_name == "default" && value.kind() == "object" {
            let mut cursor = value.walk();
            for property in value.children(&mut cursor).filter(|child| child.is_named()) {
                match property.kind() {
                    "pair" => {
                        let Some(key) = property
                            .child_by_field_name("key")
                            .and_then(|key| self.node_text(key))
                            .map(clean_name)
                            .filter(|key| !key.is_empty())
                        else {
                            continue;
                        };
                        let Some(value) = property.child_by_field_name("value") else {
                            continue;
                        };
                        bindings.push((key, value));
                    }
                    "shorthand_property_identifier" => {
                        let Some(name) = self.node_text(property).map(clean_name) else {
                            continue;
                        };
                        if !name.is_empty() {
                            bindings.push((name, property));
                        }
                    }
                    _ => {}
                }
            }
        } else {
            bindings.push((export_name.clone(), value));
        }

        let mut emitted = false;
        for (name, expression) in bindings {
            let Some(expression_name) = expression
                .is_named()
                .then(|| self.node_text(expression))
                .flatten()
                .map(clean_name)
                .filter(|name| !name.is_empty())
            else {
                continue;
            };
            let Some(target) = self.js_local_target(&expression_name) else {
                continue;
            };
            self.add_edge_at(
                &self.file_id.clone(),
                &target,
                "exports",
                node,
                Some("commonjs"),
            );
            if let Some(edge) = self.extraction.edges.last_mut() {
                edge.attributes
                    .insert("export_name".to_owned(), Value::String(name));
                edge.attributes.insert(
                    "module_format".to_owned(),
                    Value::String("commonjs".to_owned()),
                );
            }
            emitted = true;
        }
        emitted
    }

    fn js_local_target(&self, name: &str) -> Option<String> {
        let value = self
            .js_value_bindings
            .get(name)
            .filter(|targets| targets.len() == 1)
            .and_then(|targets| targets.first())
            .cloned();
        let callable = self
            .callables
            .get(name)
            .filter(|targets| targets.len() == 1)
            .and_then(|targets| targets.first())
            .cloned();
        match (value, callable) {
            (Some(value), None) | (None, Some(value)) => Some(value),
            (Some(value), Some(callable)) if value == callable => Some(value),
            _ => self.types.get(name).cloned(),
        }
    }

    fn node_text(&self, node: Node<'tree>) -> Option<String> {
        node.utf8_text(self.source).ok().map(str::to_owned)
    }

    fn add_node(
        &mut self,
        id: &str,
        label: &str,
        line: usize,
        callable: bool,
        node_type: Option<&str>,
    ) {
        if !self.seen_nodes.insert(id.to_owned()) {
            return;
        }
        let mut attributes = Map::new();
        attributes.insert("label".to_owned(), Value::String(label.to_owned()));
        attributes.insert("file_type".to_owned(), Value::String("code".to_owned()));
        attributes.insert(
            "source_file".to_owned(),
            Value::String(self.source_file.clone()),
        );
        attributes.insert(
            "source_location".to_owned(),
            Value::String(format!("L{line}")),
        );
        if callable {
            attributes.insert("_callable".to_owned(), Value::Bool(true));
        }
        if let Some(node_type) = node_type {
            attributes.insert("type".to_owned(), Value::String(node_type.to_owned()));
        }
        self.extraction.nodes.push(NodeRecord {
            id: id.to_owned(),
            attributes,
        });
    }

    fn add_edge(
        &mut self,
        source: &str,
        target: &str,
        relation: &str,
        line: usize,
        context: Option<&str>,
    ) {
        let mut attributes = Map::new();
        attributes.insert("relation".to_owned(), Value::String(relation.to_owned()));
        if let Some(context) = context {
            attributes.insert("context".to_owned(), Value::String(context.to_owned()));
        }
        attributes.insert(
            "confidence".to_owned(),
            Value::String("EXTRACTED".to_owned()),
        );
        attributes.insert(
            "source_file".to_owned(),
            Value::String(self.source_file.clone()),
        );
        attributes.insert(
            "source_location".to_owned(),
            Value::String(format!("L{line}")),
        );
        attributes.insert("weight".to_owned(), Value::from(1.0));
        self.extraction.edges.push(EdgeRecord {
            source: source.to_owned(),
            target: target.to_owned(),
            attributes,
        });
    }

    fn add_edge_at(
        &mut self,
        source: &str,
        target: &str,
        relation: &str,
        node: Node<'tree>,
        context: Option<&str>,
    ) {
        self.add_edge(source, target, relation, line(node), context);
        if let Some(edge) = self.extraction.edges.last_mut() {
            crate::facts::stamp_node_range(&mut edge.attributes, node);
        }
    }
}

struct CallName {
    name: String,
    member: bool,
    receiver: Option<String>,
}

fn collect_python_type_references(
    node: Option<Node<'_>>,
    source: &[u8],
    generic: bool,
    output: &mut Vec<(String, bool)>,
) {
    const CONTAINERS: &[&str] = &[
        "list",
        "dict",
        "set",
        "tuple",
        "frozenset",
        "type",
        "List",
        "Dict",
        "Set",
        "Tuple",
        "FrozenSet",
        "Type",
        "Optional",
        "Union",
        "Sequence",
        "Iterable",
        "Mapping",
        "MutableMapping",
        "Iterator",
        "Callable",
        "Awaitable",
        "AsyncIterable",
        "AsyncIterator",
        "Coroutine",
        "Generator",
        "AsyncGenerator",
        "ContextManager",
        "AsyncContextManager",
        "Annotated",
        "ClassVar",
        "Final",
        "Literal",
        "Concatenate",
        "ParamSpec",
        "TypeVar",
        "None",
        "Ellipsis",
    ];
    const NOISE: &[&str] = &[
        "str",
        "int",
        "float",
        "bool",
        "bytes",
        "bytearray",
        "complex",
        "object",
        "True",
        "False",
        "MagicMock",
        "Mock",
        "AsyncMock",
        "NonCallableMock",
        "NonCallableMagicMock",
        "PropertyMock",
        "patch",
        "sentinel",
    ];
    let Some(node) = node else {
        return;
    };
    let accepted =
        |name: &str| !name.is_empty() && !CONTAINERS.contains(&name) && !NOISE.contains(&name);
    match node.kind() {
        "identifier" => {
            if let Ok(name) = node.utf8_text(source)
                && accepted(name)
            {
                output.push((name.to_owned(), generic));
            }
        }
        "attribute" => {
            if let Ok(text) = node.utf8_text(source) {
                let name = text.rsplit('.').next().unwrap_or_default();
                if accepted(name) {
                    output.push((name.to_owned(), generic));
                }
            }
        }
        "generic_type" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    if let Ok(name) = child.utf8_text(source)
                        && accepted(name)
                    {
                        output.push((name.to_owned(), generic));
                    }
                } else if child.kind() == "type_parameter" {
                    let mut nested = child.walk();
                    for argument in child.children(&mut nested).filter(|child| child.is_named()) {
                        collect_python_type_references(Some(argument), source, true, output);
                    }
                }
            }
        }
        "subscript" => {
            let value = node.child_by_field_name("value");
            collect_python_type_references(value, source, generic, output);
            let mut cursor = node.walk();
            for child in node.children(&mut cursor).filter(|child| {
                child.is_named() && value.is_none_or(|value| child.id() != value.id())
            }) {
                collect_python_type_references(Some(child), source, true, output);
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor).filter(|child| child.is_named()) {
                collect_python_type_references(Some(child), source, generic, output);
            }
        }
    }
}

fn python_import_maps(
    root: Node<'_>,
    source: &[u8],
) -> (HashMap<String, String>, HashMap<String, String>) {
    fn collect(
        node: Node<'_>,
        source: &[u8],
        aliases: &mut HashMap<String, String>,
        targets: &mut HashMap<String, String>,
    ) {
        if node.kind() == "import_from_statement"
            && let Some(module) = node
                .child_by_field_name("module_name")
                .and_then(|module| module.utf8_text(source).ok())
                .map(str::trim)
                .filter(|module| !module.is_empty())
        {
            for (imported, local) in python_import_entries(node, source) {
                let imported = imported.trim();
                let local = local.trim();
                if !imported.is_empty() && !local.is_empty() && imported != "*" {
                    let callable_name = imported.rsplit('.').next().unwrap_or_default().trim();
                    let local_name = local.rsplit('.').next().unwrap_or_default().trim();
                    if !callable_name.is_empty() && !local_name.is_empty() {
                        aliases.insert(local_name.to_owned(), callable_name.to_owned());
                    }
                    let qualified = format!("{module}.{imported}");
                    match targets.get_mut(local) {
                        Some(existing) if existing != &qualified => existing.clear(),
                        Some(_) => {}
                        None => {
                            targets.insert(local.to_owned(), qualified);
                        }
                    }
                }
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            collect(child, source, aliases, targets);
        }
    }
    let mut aliases = HashMap::new();
    let mut targets = HashMap::new();
    // Compass's collection-level symbol pass indexes every `from ... import`
    // alias in the file, including function-local imports. It then scans only
    // undecorated top-level function bodies for uses. Preserve that observable
    // ordering here; `FunctionBody::top_level` supplies the matching use gate.
    collect(root, source, &mut aliases, &mut targets);
    (aliases, targets)
}

fn python_import_entries(node: Node<'_>, source: &[u8]) -> Vec<(String, String)> {
    fn collect(node: Node<'_>, source: &[u8], output: &mut Vec<(String, String)>) {
        if node.kind() == "aliased_import" {
            let imported = node
                .child_by_field_name("name")
                .and_then(|name| name.utf8_text(source).ok());
            let local = node
                .child_by_field_name("alias")
                .and_then(|name| name.utf8_text(source).ok());
            if let (Some(imported), Some(local)) = (imported, local) {
                output.push((imported.to_owned(), local.to_owned()));
            }
            return;
        }
        if matches!(node.kind(), "dotted_name" | "identifier") {
            if let Ok(name) = node.utf8_text(source) {
                output.push((name.to_owned(), name.to_owned()));
            }
            return;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(|child| child.is_named()) {
            collect(child, source, output);
        }
    }

    let mut output = Vec::new();
    let mut past_import = false;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "import" {
            past_import = true;
        } else if past_import && child.is_named() {
            collect(child, source, &mut output);
        }
    }
    output
}

pub(crate) fn python_bound_names(node: Node<'_>, source: &[u8], module: bool) -> HashSet<String> {
    let mut output = HashSet::new();
    if !module && let Some(parameters) = node.child_by_field_name("parameters") {
        let mut cursor = parameters.walk();
        for parameter in parameters.children(&mut cursor) {
            if parameter.kind() == "identifier" {
                collect_python_assignment_targets(Some(parameter), source, &mut output);
            } else if matches!(
                parameter.kind(),
                "typed_parameter"
                    | "default_parameter"
                    | "typed_default_parameter"
                    | "list_splat_pattern"
                    | "dictionary_splat_pattern"
            ) {
                let name = parameter.child_by_field_name("name").or_else(|| {
                    let mut nested = parameter.walk();
                    parameter
                        .children(&mut nested)
                        .find(|child| child.kind() == "identifier")
                });
                collect_python_assignment_targets(name, source, &mut output);
            }
        }
    }
    fn walk(node: Node<'_>, source: &[u8], root: bool, output: &mut HashSet<String>) {
        if !root && matches!(node.kind(), "function_definition" | "class_definition") {
            return;
        }
        match node.kind() {
            "assignment"
            | "annotated_assignment"
            | "augmented_assignment"
            | "for_statement"
            | "for_in_clause" => {
                collect_python_assignment_targets(node.child_by_field_name("left"), source, output);
            }
            "with_statement" => {
                let mut cursor = node.walk();
                for clause in node
                    .children(&mut cursor)
                    .filter(|child| child.kind() == "with_clause")
                {
                    let mut nested = clause.walk();
                    for item in clause
                        .children(&mut nested)
                        .filter(|child| child.kind() == "with_item")
                    {
                        collect_python_assignment_targets(
                            item.child_by_field_name("alias"),
                            source,
                            output,
                        );
                    }
                }
            }
            "named_expression" => {
                collect_python_assignment_targets(node.child_by_field_name("name"), source, output)
            }
            _ => {}
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            walk(child, source, false, output);
        }
    }
    let start = if module {
        Some(node)
    } else {
        node.child_by_field_name("body")
    };
    if let Some(start) = start {
        walk(start, source, true, &mut output);
    }
    output
}

fn collect_python_assignment_targets(
    node: Option<Node<'_>>,
    source: &[u8],
    output: &mut HashSet<String>,
) {
    let Some(node) = node else {
        return;
    };
    if node.kind() == "identifier" {
        if let Ok(name) = node.utf8_text(source) {
            output.insert(name.to_owned());
        }
    } else if matches!(
        node.kind(),
        "pattern_list" | "tuple_pattern" | "list_pattern"
    ) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            collect_python_assignment_targets(Some(child), source, output);
        }
    }
}

fn collect_js_collection_values<'tree>(node: Node<'tree>, output: &mut Vec<Node<'tree>>) {
    let mut cursor = node.walk();
    if node.kind() == "object" {
        for property in node.children(&mut cursor) {
            if property.kind() == "pair" {
                if let Some(value) = property.child_by_field_name("value")
                    && value.kind() == "identifier"
                {
                    output.push(value);
                }
            } else if property.kind() == "shorthand_property_identifier" {
                output.push(property);
            }
        }
        return;
    }
    for element in node.children(&mut cursor).filter(|child| child.is_named()) {
        if element.kind() == "identifier" {
            output.push(element);
        }
    }
}

pub(crate) fn collect_python_collection_values<'tree>(
    node: Node<'tree>,
    output: &mut Vec<Node<'tree>>,
) {
    let mut cursor = node.walk();
    if node.kind() == "dictionary" {
        for pair in node
            .children(&mut cursor)
            .filter(|child| child.kind() == "pair")
        {
            if let Some(value) = pair.child_by_field_name("value")
                && value.kind() == "identifier"
            {
                output.push(value);
            }
        }
        return;
    }
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        if child.kind() == "identifier" {
            output.push(child);
        }
    }
}

pub(crate) fn collect_python_reference_values<'tree>(
    node: Node<'tree>,
    output: &mut Vec<Node<'tree>>,
) {
    if node.kind() == "identifier" {
        output.push(node);
    } else if node.kind() == "expression_list" {
        let mut cursor = node.walk();
        for child in node
            .children(&mut cursor)
            .filter(|child| child.kind() == "identifier")
        {
            output.push(child);
        }
    }
}

fn line(node: Node<'_>) -> usize {
    node.start_position().row + 1
}

fn first_descendant<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            return Some(child);
        }
        if let Some(found) = first_descendant(child, kind) {
            return Some(found);
        }
    }
    None
}

fn direct_named_children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

fn first_identifier(node: Node<'_>) -> Option<Node<'_>> {
    [
        "identifier",
        "type_identifier",
        "simple_identifier",
        "name",
        "word",
    ]
    .iter()
    .find_map(|kind| first_descendant(node, kind))
}

fn c_declarator_name(node: Node<'_>) -> Option<Node<'_>> {
    if node.kind() == "identifier" {
        return Some(node);
    }
    if let Some(declarator) = node.child_by_field_name("declarator") {
        return c_declarator_name(declarator);
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if let Some(name) = c_declarator_name(child) {
            return Some(name);
        }
    }
    None
}

fn last_identifier(node: Node<'_>) -> Option<Node<'_>> {
    let mut result = None;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(
            child.kind(),
            "identifier" | "type_identifier" | "simple_identifier" | "name" | "word"
        ) {
            result = Some(child);
        }
        if let Some(found) = last_identifier(child) {
            result = Some(found);
        }
    }
    result
}

fn clean_name(value: String) -> String {
    value
        .trim()
        .trim_matches(['\'', '"', '`', '&', '*', '$', '@'])
        .trim_end_matches(['!', '?'])
        .to_owned()
}

fn js_heritage_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    let target = if node.kind() == "generic_type" {
        node.child_by_field_name("name")?
    } else {
        node
    };
    if !matches!(
        target.kind(),
        "identifier" | "type_identifier" | "nested_type_identifier" | "member_expression"
    ) {
        return None;
    }
    target
        .utf8_text(source)
        .ok()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

fn quoted_value(value: &str) -> Option<String> {
    for quote in ['\'', '"'] {
        if let Some(start) = value.find(quote) {
            let rest = &value[start + quote.len_utf8()..];
            if let Some(end) = rest.find(quote) {
                return Some(rest[..end].to_owned());
            }
        }
    }
    None
}

fn angle_value(value: &str) -> Option<String> {
    let start = value.find('<')?;
    let rest = &value[start + 1..];
    let end = rest.find('>')?;
    Some(rest[..end].to_owned())
}

fn collect_c_type_names(node: Node<'_>, source: &[u8], output: &mut Vec<String>) {
    if node.kind() == "type_identifier" {
        if let Ok(text) = node.utf8_text(source) {
            output.push(text.to_owned());
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_c_type_names(child, source, output);
    }
}

fn collect_scala_type_refs(
    node: Node<'_>,
    source: &[u8],
    generic: bool,
    output: &mut Vec<(String, bool)>,
) {
    if node.kind() == "type_identifier" {
        if let Ok(name) = node.utf8_text(source)
            && !name.is_empty()
        {
            output.push((name.to_owned(), generic));
        }
        return;
    }
    if node.kind() == "generic_type" {
        let base = node
            .child_by_field_name("type")
            .or_else(|| first_descendant(node, "type_identifier"));
        if let Some(base) = base
            && let Ok(name) = base.utf8_text(source)
            && !name.is_empty()
        {
            output.push((name.to_owned(), generic));
        }
        let mut cursor = node.walk();
        for arguments in node
            .children(&mut cursor)
            .filter(|child| child.kind() == "type_arguments")
        {
            let mut argument_cursor = arguments.walk();
            for argument in arguments
                .children(&mut argument_cursor)
                .filter(|child| child.is_named())
            {
                collect_scala_type_refs(argument, source, true, output);
            }
        }
        return;
    }
    if matches!(
        node.kind(),
        "compound_type"
            | "infix_type"
            | "function_type"
            | "tuple_type"
            | "annotated_type"
            | "projected_type"
    ) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(|child| child.is_named()) {
            collect_scala_type_refs(child, source, generic, output);
        }
    }
}

fn collect_nodes_of_kind<'tree>(node: Node<'tree>, kind: &str, output: &mut Vec<Node<'tree>>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            output.push(child);
        } else {
            collect_nodes_of_kind(child, kind, output);
        }
    }
}

fn find_require_call<'tree>(node: Node<'tree>, source: &[u8]) -> Option<Node<'tree>> {
    if node.kind() == "call_expression"
        && node
            .child_by_field_name("function")
            .is_some_and(|function| source_node_text(function, source) == "require")
    {
        return Some(node);
    }
    if node.kind() == "member_expression"
        && let Some(object) = node.child_by_field_name("object")
    {
        return find_require_call(object, source);
    }
    None
}

fn lexical_normalize(path: &Path) -> std::path::PathBuf {
    use std::path::Component;

    let mut output = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                output.pop();
            }
            other => output.push(other.as_os_str()),
        }
    }
    output
}

fn resolve_js_import_path(path: &Path) -> std::path::PathBuf {
    if path.is_file() {
        return path.to_path_buf();
    }
    if path.extension().and_then(|value| value.to_str()) == Some("js") {
        let candidate = path.with_extension("ts");
        if candidate.is_file() {
            return candidate;
        }
    } else if path.extension().and_then(|value| value.to_str()) == Some("jsx") {
        let candidate = path.with_extension("tsx");
        if candidate.is_file() {
            return candidate;
        }
    }
    for extension in [
        "ts", "tsx", "mts", "cts", "svelte", "js", "jsx", "mjs", "cjs",
    ] {
        let candidate = path.with_file_name(format!(
            "{}.{extension}",
            path.file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
        ));
        if candidate.is_file() {
            return candidate;
        }
    }
    if path.is_dir() {
        for name in [
            "index.ts",
            "index.tsx",
            "index.svelte",
            "index.js",
            "index.jsx",
            "index.mjs",
        ] {
            let candidate = path.join(name);
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    path.to_path_buf()
}

#[cfg(test)]
mod rationale_tests {
    use super::*;

    #[test]
    fn explicit_source_identity_controls_universal_framework_anchors()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("src/orders.ts");
        let source = br#"import { Controller } from '@nestjs/common';
import { EventPattern } from '@nestjs/microservices';
@Controller()
export class OrdersConsumer {
  @EventPattern('orders.cancelled')
  handleCancelled() {}
}
"#;

        let extraction =
            Engine::default().extract_source_graph_only(&path, "src/orders.ts", source)?;

        assert!(!extraction.framework_facts.is_empty());
        assert!(extraction.framework_facts.iter().all(|fact| {
            let anchor = match fact {
                crate::RawFrameworkFact::Route(route) => &route.anchor,
                crate::RawFrameworkFact::Domain(domain) => &domain.anchor,
                crate::RawFrameworkFact::Annotation(annotation) => &annotation.anchor,
            };
            anchor.source_file == "src/orders.ts"
        }));
        Ok(())
    }

    #[test]
    fn explicit_source_identity_controls_csharp_universal_evidence()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("Controllers/OrdersController.cs");
        let source = br#"using Microsoft.AspNetCore.Mvc;
namespace Store.Controllers;
[ApiController]
[Route("api/[controller]")]
public class OrdersController : ControllerBase {
    [HttpGet("{id:int}")]
    public string Get(int id) => id.ToString();
}
"#;

        let extraction = Engine::default().extract_source_graph_only(
            &path,
            "Controllers/OrdersController.cs",
            source,
        )?;
        let evidence = extraction
            .semantic_evidence
            .ok_or("C# universal evidence was not emitted")?;

        assert!(
            evidence
                .declarations
                .iter()
                .all(|fact| { fact.range.source_file == "Controllers/OrdersController.cs" })
        );
        assert!(extraction.framework_facts.iter().all(|fact| {
            let anchor = match fact {
                crate::RawFrameworkFact::Route(route) => &route.anchor,
                crate::RawFrameworkFact::Domain(domain) => &domain.anchor,
                crate::RawFrameworkFact::Annotation(annotation) => &annotation.anchor,
            };
            anchor.source_file == "Controllers/OrdersController.cs"
        }));
        Ok(())
    }

    #[test]
    fn c_function_declarators_prefer_callable_names_over_types()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("declarators.c");
        fs::write(
            &source,
            "SQLITE_PRIVATE char *sqlite3CompileOptions(void) { return 0; }\n\
             SQLITE_PRIVATE sqlite3_int64 sqlite3StatusValue(int op) { return op; }\n\
             int valueFromExpr(void) { int (*callback)(op *); return callback != 0; }\n\
             int acceptsOp(Op *value) { return value != 0; }\n",
        )?;

        let extraction = Engine::default().extract(&source)?;
        let labels = extraction
            .nodes
            .iter()
            .map(NodeRecord::label)
            .collect::<HashSet<_>>();

        for expected in ["sqlite3CompileOptions()", "sqlite3StatusValue()"] {
            assert!(labels.contains(expected), "missing {expected}");
        }
        for invalid in ["char()", "sqlite3_int64()"] {
            assert!(!labels.contains(invalid), "unexpected {invalid}");
        }
        assert!(labels.contains("Op"));
        assert!(!labels.contains("op"));
        Ok(())
    }

    #[test]
    fn c_quoted_include_targets_the_existing_header_file() -> Result<(), Box<dyn std::error::Error>>
    {
        let directory = tempfile::tempdir()?;
        let header = directory.path().join("shm_lock.h");
        let source = directory.path().join("shm_lock.c");
        fs::write(&header, "int lock(void);\n")?;
        fs::write(
            &source,
            "#include \"shm_lock.h\"\nint main(void) { return lock(); }\n",
        )?;

        let extraction = Engine::default().extract(&source)?;
        let expected = make_id(&[&fs::canonicalize(header)?.to_string_lossy()]);
        assert!(extraction.edges.iter().any(|edge| {
            edge.target == expected
                && edge.attributes.get("relation").and_then(Value::as_str) == Some("imports")
        }));
        Ok(())
    }

    #[test]
    fn definitions_include_display_metadata() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("fixture.py");
        fs::write(
            &source,
            "def reveal(t, hold_val=\"1\", start_val=\"0\"):\n\
             \x20   value = t + hold_val\n\
             \x20   return value + start_val\n",
        )?;

        let extraction = Engine::default().extract(&source)?;
        let declaration = extraction
            .semantic_evidence
            .as_ref()
            .ok_or("missing Python semantic evidence")?
            .declarations
            .iter()
            .find(|declaration| declaration.name == "reveal")
            .ok_or("missing reveal function")?;

        assert_eq!(declaration.kind, "function");
        assert_eq!(declaration.language, "python");
        assert_eq!(declaration.range.start_line, 1);
        assert_eq!(declaration.range.end_line, 1);
        assert_eq!(
            declaration.signature.as_deref(),
            Some("def reveal(t, hold_val=\"1\", start_val=\"0\")")
        );
        Ok(())
    }

    #[test]
    fn javascript_prototype_assignments_are_methods_with_callable_bodies()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("widget.js");
        fs::write(
            &source,
            "function Widget() {}\n\
             Widget.prototype.render = function () { helper(); };\n\
             function helper() { return true; }\n",
        )?;

        let source_bytes = fs::read(&source)?;
        let extraction = Engine::default().extract(&source)?;
        let evidence = extraction
            .semantic_evidence
            .as_ref()
            .ok_or("missing JavaScript universal evidence")?;
        let method = evidence
            .declarations
            .iter()
            .find(|declaration| {
                declaration.name == "render"
                    && declaration.qualified_name.contains("Widget.prototype")
            })
            .ok_or("missing prototype method")?;
        assert_eq!(method.kind, "property");
        let direct = Engine::default().extract_source_universal_candidate_evidence(
            &source,
            "widget.js",
            &source_bytes,
        )?;
        let helper = direct
            .declarations
            .iter()
            .find(|declaration| declaration.name == "helper" && declaration.kind == "function")
            .ok_or("missing helper declaration")?;
        assert!(evidence.candidates.iter().any(|candidate| {
            candidate.relation == crate::CandidateRelation::Calls
                && candidate.target_spelling == "helper"
                && candidate.constraints.exact_target_declaration_id.as_deref()
                    == Some(helper.id.as_str())
        }));
        Ok(())
    }

    #[test]
    fn typescript_variance_modifiers_parse_without_recovery()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("schemas.ts");
        let fixture = "export interface ZodType<out Output = unknown, in Input = unknown> {}\n\
                       export interface _ZodType<out Internals extends ZodType = ZodType>\n\
                         extends ZodType<unknown, unknown> {}\n\
                       export interface ZodAny extends _ZodType<ZodType> {}\n\
                       export const ZodAny: Constructor<ZodAny> = factory();\n\
                       export function create(): ZodAny { return new ZodAny(); }\n\
                       export type Keys<T> = { [K in keyof T]: T[K] };\n\
                       export const out = 1;\n\
                       export interface $ZodCheck<in T = never> {}\n\
                       export interface ZodEnum<\n\
                         /** @ts-ignore Cast variance */\n\
                         out T extends Record<string, string> = Record<string, string>,\n\
                       > {}\n\
                       export const First: Constructor<First> = factory();\n\
                       export interface First extends _ZodType<ZodType> {}\n\
                       export function createFirst(): First { return new First(); }\n";
        let control = fixture
            .replace("out Output", "    Output")
            .replace("in Input", "   Input")
            .replace("out Internals", "    Internals")
            .replace("<in T", "<   T")
            .replace("out T extends", "    T extends");
        fs::write(&source, control)?;
        let control_extraction = Engine::default().extract(&source)?;
        assert_ne!(
            control_extraction
                .extensions
                .get(EXTRACTION_QUALITY_EXTENSION)
                .and_then(Value::as_str),
            Some(EXTRACTION_QUALITY_PARTIAL),
            "the byte-preserving control must parse exactly"
        );

        fs::write(&source, fixture)?;
        let extraction = Engine::default().extract(&source)?;
        assert_ne!(
            extraction
                .extensions
                .get(EXTRACTION_QUALITY_EXTENSION)
                .and_then(Value::as_str),
            Some(EXTRACTION_QUALITY_PARTIAL),
            "valid TypeScript variance syntax must not trigger parser recovery"
        );
        let evidence = extraction
            .semantic_evidence
            .as_ref()
            .ok_or("missing TypeScript universal evidence")?;
        let zod_any = evidence
            .declarations
            .iter()
            .find(|declaration| declaration.name == "ZodAny" && declaration.kind == "interface")
            .ok_or("missing ZodAny interface")?;
        let private_zod_type = evidence
            .declarations
            .iter()
            .find(|declaration| declaration.name == "_ZodType" && declaration.kind == "interface")
            .ok_or("missing _ZodType interface")?;
        let zod_any_value = evidence
            .declarations
            .iter()
            .find(|declaration| declaration.name == "ZodAny" && declaration.kind == "variable")
            .ok_or("missing ZodAny runtime value")?;
        let create = evidence
            .declarations
            .iter()
            .find(|declaration| declaration.name == "create" && declaration.kind == "function")
            .ok_or("missing create function")?;
        let first_type = evidence
            .declarations
            .iter()
            .find(|declaration| declaration.name == "First" && declaration.kind == "interface")
            .ok_or("missing reverse-order First interface")?;
        let first_value = evidence
            .declarations
            .iter()
            .find(|declaration| declaration.name == "First" && declaration.kind == "variable")
            .ok_or("missing reverse-order First runtime value")?;
        let create_first = evidence
            .declarations
            .iter()
            .find(|declaration| declaration.name == "createFirst" && declaration.kind == "function")
            .ok_or("missing createFirst function")?;
        let keys = evidence
            .declarations
            .iter()
            .find(|declaration| declaration.name == "Keys")
            .ok_or("mapped-type `in` must remain a Keys declaration")?;
        assert_eq!(zod_any.range.start_line, 4);
        assert_eq!(zod_any_value.range.start_line, 5);
        assert_eq!(keys.range.start_line, 7);
        assert!(evidence.candidates.iter().any(|candidate| {
            candidate.relation == crate::CandidateRelation::Extends
                && candidate.source_declaration_id == zod_any.id
                && candidate.constraints.exact_target_declaration_id.as_deref()
                    == Some(private_zod_type.id.as_str())
        }));
        assert!(evidence.candidates.iter().any(|candidate| {
            candidate.relation == crate::CandidateRelation::Constructs
                && candidate.source_declaration_id == create.id
                && candidate.constraints.exact_target_declaration_id.as_deref()
                    == Some(zod_any_value.id.as_str())
        }));
        assert!(!evidence.candidates.iter().any(|candidate| {
            candidate.relation == crate::CandidateRelation::Calls
                && candidate.source_declaration_id == create.id
                && candidate.constraints.exact_target_declaration_id.as_deref()
                    == Some(zod_any.id.as_str())
        }));
        assert_ne!(first_type.id, first_value.id);
        assert!(evidence.candidates.iter().any(|candidate| {
            candidate.relation == crate::CandidateRelation::Constructs
                && candidate.source_declaration_id == create_first.id
                && candidate.constraints.exact_target_declaration_id.as_deref()
                    == Some(first_value.id.as_str())
        }));
        assert!(!evidence.candidates.iter().any(|candidate| {
            candidate.relation == crate::CandidateRelation::Constructs
                && candidate.source_declaration_id == create_first.id
                && candidate.constraints.exact_target_declaration_id.as_deref()
                    == Some(first_type.id.as_str())
        }));

        fs::write(&source, "export interface Broken<out T extends> {}\n")?;
        let malformed = Engine::default().extract(&source)?;
        assert_eq!(
            malformed
                .extensions
                .get(EXTRACTION_QUALITY_EXTENSION)
                .and_then(Value::as_str),
            Some(EXTRACTION_QUALITY_PARTIAL),
            "variance recovery must not hide genuinely malformed source"
        );

        let tsx = directory.path().join("component.tsx");
        fs::write(
            &tsx,
            "export interface Props<out Value> { value: Value }\n\
             export const Component = <Value,>(props: Props<Value>) => <div>{props.value}</div>;\n",
        )?;
        let tsx_extraction = Engine::default().extract(&tsx)?;
        assert_ne!(
            tsx_extraction
                .extensions
                .get(EXTRACTION_QUALITY_EXTENSION)
                .and_then(Value::as_str),
            Some(EXTRACTION_QUALITY_PARTIAL),
            "valid TSX variance syntax must not trigger parser recovery"
        );
        Ok(())
    }

    #[test]
    fn typescript_duplicate_runtime_values_fail_closed() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("ambiguous.ts");
        fs::write(
            &source,
            "const Widget = factory();\n\
             const Widget = replacement();\n\
             export function create() { return new Widget(); }\n",
        )?;

        let extraction = Engine::default().extract(&source)?;
        let evidence = extraction
            .semantic_evidence
            .as_ref()
            .ok_or("missing TypeScript universal evidence")?;
        let create = evidence
            .declarations
            .iter()
            .find(|declaration| declaration.name == "create" && declaration.kind == "function")
            .ok_or("missing create function")?;
        let widget_values = evidence
            .declarations
            .iter()
            .filter(|declaration| declaration.name == "Widget" && declaration.kind == "variable")
            .map(|declaration| declaration.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(widget_values.len(), 2);
        assert!(!evidence.candidates.iter().any(|candidate| {
            candidate.source_declaration_id == create.id
                && matches!(
                    candidate.relation,
                    crate::CandidateRelation::Calls | crate::CandidateRelation::Constructs
                )
                && candidate
                    .constraints
                    .exact_target_declaration_id
                    .as_deref()
                    .is_some_and(|target| widget_values.contains(&target))
        }));
        assert!(evidence.candidates.iter().any(|candidate| {
            candidate.source_declaration_id == create.id
                && candidate.relation == crate::CandidateRelation::Constructs
                && candidate.target_spelling == "Widget"
                && candidate.constraints.exact_target_declaration_id.is_none()
        }));
        Ok(())
    }

    #[test]
    fn python_parenthesized_imports_qualify_inherited_types()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("tests.py");
        fs::write(
            &source,
            "from django.test import (\n    RequestFactory,\n    TestCase,\n)\n\
             class Example(TestCase):\n    pass\n",
        )?;

        let extraction = Engine::default().extract(&source)?;
        let evidence = extraction
            .semantic_evidence
            .as_ref()
            .ok_or("missing Python semantic evidence")?;
        let example = evidence
            .declarations
            .iter()
            .find(|declaration| declaration.name == "Example")
            .ok_or("missing Example class")?;
        assert!(evidence.candidates.iter().any(|candidate| {
            candidate.source_declaration_id == example.id
                && candidate.relation == crate::CandidateRelation::Extends
                && candidate.constraints.qualified_name.as_deref() == Some("django.test.TestCase")
        }));
        assert!(evidence.bindings.iter().any(|binding| {
            binding.spelling == "TestCase" && binding.qualified_target == "django.test.TestCase"
        }));
        Ok(())
    }

    #[test]
    fn python_member_calls_retain_unambiguous_import_qualification()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("tests.py");
        fs::write(
            &source,
            "from unittest import mock\n\
             def exercise():\n    mock.patch('service.call')\n",
        )?;

        let extraction = Engine::default().extract(&source)?;
        assert!(extraction.raw_calls.is_none());
        let evidence = extraction
            .semantic_evidence
            .as_ref()
            .ok_or("missing Python semantic evidence")?;
        let call = evidence
            .occurrences
            .iter()
            .find(|occurrence| {
                occurrence.role == crate::SemanticRole::Call
                    && occurrence.spelling == "patch"
                    && occurrence.qualifier.as_deref() == Some("mock")
            })
            .ok_or("missing mock.patch occurrence")?;
        let candidate = evidence
            .candidates
            .iter()
            .find(|candidate| candidate.occurrence_id.as_deref() == Some(&call.id))
            .ok_or("missing mock.patch candidate")?;
        assert_eq!(
            candidate.constraints.qualified_name.as_deref(),
            Some("unittest.mock.patch")
        );
        assert!(call.range.start_byte < call.range.end_byte);

        fs::write(
            &source,
            "from unittest import mock\n\
             from vendor import mock\n\
             def exercise():\n    mock.patch('service.call')\n",
        )?;
        let rebound = Engine::default().extract(&source)?;
        assert!(rebound.raw_calls.is_none());
        let rebound_evidence = rebound
            .semantic_evidence
            .as_ref()
            .ok_or("missing rebound Python semantic evidence")?;
        let rebound_call = rebound_evidence
            .occurrences
            .iter()
            .find(|occurrence| {
                occurrence.role == crate::SemanticRole::Call
                    && occurrence.spelling == "patch"
                    && occurrence.qualifier.as_deref() == Some("mock")
            })
            .ok_or("missing rebound mock.patch occurrence")?;
        assert!(rebound_evidence.candidates.iter().any(|candidate| {
            candidate.occurrence_id.as_deref() == Some(&rebound_call.id)
                && candidate.constraints.qualified_name.as_deref() == Some("vendor.mock.patch")
        }));
        Ok(())
    }

    #[test]
    fn definition_hashes_separate_signature_implementation_and_source_changes()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("fixture.cpp");
        let mut engine = Engine::default();
        let extract_hashes = |engine: &mut Engine,
                              path: &Path,
                              text: &str|
         -> Result<[String; 3], Box<dyn std::error::Error>> {
            fs::write(path, text)?;
            let extraction = engine.extract(path)?;
            let node = extraction
                .nodes
                .iter()
                .find(|node| node.label() == "value()")
                .ok_or("missing value function")?;
            Ok([
                node.string("signature_hash"),
                node.string("implementation_hash"),
                node.string("source_hash"),
            ])
        };
        let original = extract_hashes(&mut engine, &source, "int value() { return 1; }\n")?;
        let formatted = extract_hashes(
            &mut engine,
            &source,
            "int value() {\n  // retained only by source_hash\n  return 1;\n}\n",
        )?;
        let changed = extract_hashes(&mut engine, &source, "int value() { return 2; }\n")?;

        assert_eq!(original[0], formatted[0]);
        assert_eq!(original[1], formatted[1]);
        assert_ne!(original[2], formatted[2]);
        assert_eq!(original[0], changed[0]);
        assert_ne!(original[1], changed[1]);
        assert_ne!(original[2], changed[2]);
        assert!(original.iter().all(|digest| digest.len() == 64));
        Ok(())
    }

    #[test]
    fn definition_hashes_attach_to_qualified_cpp_methods() -> Result<(), Box<dyn std::error::Error>>
    {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("db_impl.cc");
        fs::write(
            &source,
            "int DBImpl::Compact() {\n  return shutting_down ? 0 : 1;\n}\n",
        )?;

        let extraction = Engine::default().extract(&source)?;
        let node = extraction
            .nodes
            .iter()
            .find(|node| node.label() == "DBImpl::Compact()")
            .ok_or("missing qualified C++ method")?;

        assert_eq!(node.string("signature_hash").len(), 64);
        assert_eq!(node.string("implementation_hash").len(), 64);
        assert_eq!(node.string("source_hash").len(), 64);
        Ok(())
    }

    #[test]
    fn python_import_alias_uses_match_top_level_function_scan()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("aliases.py");
        fs::write(
            &source,
            "from package import top as module_alias\n\
             def first():\n    module_alias()\n\
             @marker\n\
             def decorated():\n    from package import nested as local_alias\n    local_alias()\n\
             def third():\n    local_alias()\n",
        )?;

        let extraction = Engine::default().extract(&source)?;
        assert!(extraction.raw_calls.is_none());
        let evidence = extraction
            .semantic_evidence
            .ok_or("missing Python semantic evidence")?;
        let import_uses = evidence
            .candidates
            .iter()
            .filter(|candidate| candidate.relation == crate::CandidateRelation::Calls)
            .filter_map(|candidate| {
                candidate
                    .constraints
                    .qualified_name
                    .as_deref()
                    .map(|qualified_name| (candidate, qualified_name))
            })
            .collect::<Vec<_>>();

        assert_eq!(import_uses.len(), 2);
        assert!(import_uses.iter().any(|(candidate, qualified_name)| {
            candidate.target_spelling == "module_alias" && *qualified_name == "package.top"
        }));
        assert!(import_uses.iter().any(|(candidate, qualified_name)| {
            candidate.target_spelling == "local_alias" && *qualified_name == "package.nested"
        }));
        assert!(evidence.candidates.iter().any(|candidate| {
            candidate.target_spelling == "local_alias"
                && candidate.binding_id.is_none()
                && candidate.constraints.qualified_name.is_none()
        }));
        Ok(())
    }

    #[test]
    fn rust_calls_named_like_other_language_builtins_resolve_locally()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("builtins.rs");
        fs::write(
            &source,
            "fn open() {}\nfn list() {}\nfn caller() { open(); list(); }\n",
        )?;

        let extraction = Engine::default().extract(&source)?;
        let evidence = extraction
            .semantic_evidence
            .as_ref()
            .ok_or("missing Rust semantic evidence")?;
        let caller = evidence
            .declarations
            .iter()
            .find(|declaration| declaration.name == "caller")
            .ok_or("missing caller")?;
        assert_eq!(
            evidence
                .candidates
                .iter()
                .filter(|candidate| {
                    candidate.source_declaration_id == caller.id
                        && candidate.relation == crate::CandidateRelation::Calls
                        && matches!(candidate.target_spelling.as_str(), "open" | "list")
                })
                .count(),
            2
        );
        assert!(extraction.nodes.is_empty());
        assert!(extraction.edges.is_empty());
        assert!(extraction.raw_calls.is_none());
        Ok(())
    }

    #[test]
    fn rust_scoped_calls_use_type_namespace_imports_over_values()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("namespace.rs");
        fs::write(
            &source,
            "use std::thread;\n\
             struct ThreadBuilder;\n\
             impl ThreadBuilder { fn name(&self) -> Option<&str> { None } }\n\
             fn spawn(thread: ThreadBuilder) {\n\
                 let _builder = thread::Builder::new();\n\
                 let _name = thread.name();\n\
             }\n",
        )?;

        let extraction = Engine::default().extract(&source)?;
        let evidence = extraction
            .semantic_evidence
            .as_ref()
            .ok_or("missing Rust semantic evidence")?;
        let associated_call = evidence
            .candidates
            .iter()
            .find(|candidate| {
                candidate.relation == crate::CandidateRelation::Calls
                    && candidate.target_spelling == "new"
            })
            .ok_or("missing scoped associated call")?;
        assert_eq!(
            associated_call.constraints.qualified_name.as_deref(),
            Some("std::thread::Builder::new")
        );
        assert!(associated_call.constraints.allow_external);
        let associated_binding = associated_call
            .binding_id
            .as_deref()
            .and_then(|binding_id| {
                evidence
                    .bindings
                    .iter()
                    .find(|binding| binding.id == binding_id)
            })
            .ok_or("missing associated-call binding")?;
        assert_eq!(associated_binding.kind, crate::BindingKind::Import);
        assert_eq!(associated_binding.qualified_target, "std::thread");

        let value_call = evidence
            .candidates
            .iter()
            .find(|candidate| {
                candidate.relation == crate::CandidateRelation::Calls
                    && candidate.target_spelling == "name"
            })
            .ok_or("missing value receiver call")?;
        assert_eq!(
            value_call.constraints.qualified_name.as_deref(),
            Some("crate::namespace::ThreadBuilder::name")
        );
        let value_binding = value_call
            .binding_id
            .as_deref()
            .and_then(|binding_id| {
                evidence
                    .bindings
                    .iter()
                    .find(|binding| binding.id == binding_id)
            })
            .ok_or("missing value-call binding")?;
        assert_eq!(value_binding.kind, crate::BindingKind::LocalAlias);
        assert_eq!(
            value_binding.qualified_target,
            "crate::namespace::ThreadBuilder"
        );
        Ok(())
    }

    #[test]
    fn rust_ambiguous_type_namespace_imports_fail_closed_over_values()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("ambiguous_namespace.rs");
        fs::write(
            &source,
            "use first::thread;\n\
             use second::thread;\n\
             struct ThreadBuilder;\n\
             impl ThreadBuilder { fn name(&self) -> Option<&str> { None } }\n\
             fn spawn(thread: ThreadBuilder) {\n\
                 let _builder = thread::Builder::new();\n\
                 let _name = thread.name();\n\
             }\n",
        )?;

        let extraction = Engine::default().extract(&source)?;
        let evidence = extraction
            .semantic_evidence
            .as_ref()
            .ok_or("missing Rust semantic evidence")?;
        assert!(evidence.candidates.iter().all(|candidate| {
            candidate.relation != crate::CandidateRelation::Calls
                || candidate.target_spelling != "new"
        }));
        assert!(evidence.candidates.iter().any(|candidate| {
            candidate.relation == crate::CandidateRelation::Calls
                && candidate.target_spelling == "name"
                && candidate.constraints.qualified_name.as_deref()
                    == Some("crate::ambiguous_namespace::ThreadBuilder::name")
        }));
        Ok(())
    }

    #[test]
    fn python_imported_module_member_calls_are_deferred_as_resolvable_symbols()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("cli.py");
        fs::write(
            &source,
            "def dispatch():\n    from compass import querylog\n    querylog.log_query(kind='query')\n",
        )?;

        let extraction = Engine::default().extract(&source)?;
        assert!(extraction.raw_calls.is_none());
        let evidence = extraction
            .semantic_evidence
            .ok_or("missing Python semantic evidence")?;
        let call = evidence
            .occurrences
            .iter()
            .find(|occurrence| {
                occurrence.role == crate::SemanticRole::Call
                    && occurrence.spelling == "log_query"
                    && occurrence.qualifier.as_deref() == Some("querylog")
            })
            .ok_or("missing querylog.log_query occurrence")?;
        assert!(evidence.candidates.iter().any(|candidate| {
            candidate.occurrence_id.as_deref() == Some(&call.id)
                && candidate.constraints.qualified_name.as_deref()
                    == Some("compass.querylog.log_query")
        }));
        Ok(())
    }

    #[test]
    fn recognizes_python_module_docstring() -> Result<(), Box<dyn std::error::Error>> {
        let source = b"\"\"\"A sufficiently long architectural rationale for the module.\"\"\"\n";
        let language = tree_sitter_language_pack::get_language("python")?;
        let mut parser = Parser::new();
        parser.set_language(&language)?;
        let tree = parser.parse(source, None).ok_or("missing tree")?;
        assert_eq!(
            python_docstring(tree.root_node(), source),
            Some((
                "A sufficiently long architectural rationale for the module.".to_owned(),
                1
            )),
            "{}",
            tree.root_node().to_sexp()
        );
        Ok(())
    }

    #[test]
    fn generic_symbols_emit_only_canonical_v1_kinds() {
        assert_eq!(symbol_kind("record_declaration", true, false), "struct");
        assert_eq!(
            symbol_kind("type_alias_declaration", true, false),
            "type_alias"
        );
        assert_eq!(symbol_kind("deinit_declaration", false, false), "method");
    }
}
