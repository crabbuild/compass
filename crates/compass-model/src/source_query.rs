//! Immutable auxiliary lexical evidence. It never adds semantic graph records.
use crate::provenance::SourceAnchor;
use crate::search::{
    SEARCH_FIELD_MAX_BYTES, SEARCH_FIELD_MAX_TERMS, SearchDocument, identifier_search_terms,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const SOURCE_QUERY_SCHEMA: &str = "compass.source-query/1";
pub const SOURCE_QUERY_PROFILE_V1: &str = "source-query/1-ast-callable-lexemes";
pub const SOURCE_QUERY_PROFILE_V2: &str = "source-query/2-callable-definition-envelopes";
pub const SOURCE_QUERY_PROFILE_V3: &str = "source-query/3-owned-code-windows";
pub const SOURCE_QUERY_PROFILE_V4: &str = "source-query/4-nested-callable-context";
pub const SOURCE_QUERY_PROFILE_V5: &str = "source-query/5-owned-code-lexemes";
pub const SOURCE_QUERY_PROFILE_V6: &str = "source-query/6-parameter-bindings";
pub const SOURCE_QUERY_PROFILE: &str = "source-query/7-literal-seeded-code";
pub const SOURCE_QUERY_MAX_PARAMETER_SITES: usize = 64;
pub const SOURCE_QUERY_MAX_PARAMETER_FILE_BYTES: usize = 256 * 1024;
pub const SOURCE_QUERY_MAX_FILE_TEXT_BYTES: usize = 1024 * 1024;
pub const SOURCE_QUERY_MAX_NESTED_CONTEXTS: usize = 8;
pub const SOURCE_QUERY_MAX_CODE_BYTES: usize = 8 * 1024 * 1024;
pub const SOURCE_QUERY_MAX_CODE_WINDOWS: usize = 32_768;
pub const SOURCE_QUERY_FILE: &str = "source-query.json";
pub const SOURCE_QUERY_MAX_BYTES: u64 = 64 * 1024 * 1024;
pub const SOURCE_QUERY_MAX_OWNERS: usize = 32_768;
pub const SOURCE_QUERY_MAX_SITES: usize = 65_536;
pub const SOURCE_QUERY_MAX_TEXT_BYTES: usize = 16 * 1024 * 1024;

#[must_use]
pub fn is_supported_source_query_profile(profile: &str) -> bool {
    matches!(
        profile,
        SOURCE_QUERY_PROFILE_V1
            | SOURCE_QUERY_PROFILE_V2
            | SOURCE_QUERY_PROFILE_V3
            | SOURCE_QUERY_PROFILE_V4
            | SOURCE_QUERY_PROFILE_V5
            | SOURCE_QUERY_PROFILE_V6
            | SOURCE_QUERY_PROFILE
    )
}

/// Profile capabilities are explicit: advancing current must not change V5.
#[must_use]
pub fn source_query_uses_code_lexemes(profile: &str) -> bool {
    matches!(
        profile,
        SOURCE_QUERY_PROFILE_V5 | SOURCE_QUERY_PROFILE_V6 | SOURCE_QUERY_PROFILE
    )
}

/// Historical profile capabilities are explicit, not ordered string versions.
#[must_use]
pub fn source_query_uses_code_windows(profile: &str) -> bool {
    matches!(
        profile,
        SOURCE_QUERY_PROFILE_V3
            | SOURCE_QUERY_PROFILE_V4
            | SOURCE_QUERY_PROFILE_V5
            | SOURCE_QUERY_PROFILE_V6
            | SOURCE_QUERY_PROFILE
    )
}
#[must_use]
pub fn source_query_uses_nested_context(profile: &str) -> bool {
    matches!(
        profile,
        SOURCE_QUERY_PROFILE_V4
            | SOURCE_QUERY_PROFILE_V5
            | SOURCE_QUERY_PROFILE_V6
            | SOURCE_QUERY_PROFILE
    )
}
#[must_use]
pub fn source_query_uses_parameter_bindings(profile: &str) -> bool {
    matches!(profile, SOURCE_QUERY_PROFILE_V6 | SOURCE_QUERY_PROFILE)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParameterBindingStatus {
    Complete,
    Partial,
    Unsupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParameterBindingOmission {
    UnsupportedPattern,
    UnsupportedReceiver,
    UnsupportedSpelling,
    SyntaxRecovery,
    TraversalLimit,
    SiteLimit,
    TextLimit,
    AggregateLimit,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ParameterBindings {
    pub sites: Vec<SourceQuerySite>,
    pub status: ParameterBindingStatus,
    /// Known omitted bindings; an unsupported suffix is not an exact count.
    pub omitted_sites: u64,
    pub reasons: BTreeSet<ParameterBindingOmission>,
}
impl Default for ParameterBindings {
    fn default() -> Self {
        Self {
            sites: Vec::new(),
            status: ParameterBindingStatus::Complete,
            omitted_sites: 0,
            reasons: BTreeSet::new(),
        }
    }
}

impl ParameterBindings {
    /// Enumerated omissions have a known positive count. Unavailable syntax or
    /// an unvisited suffix can have an unknown count, represented by zero.
    #[must_use]
    pub fn has_coherent_coverage(&self) -> bool {
        let enumerated = self.reasons.iter().any(|reason| {
            matches!(
                reason,
                ParameterBindingOmission::UnsupportedSpelling
                    | ParameterBindingOmission::SiteLimit
                    | ParameterBindingOmission::TextLimit
                    | ParameterBindingOmission::AggregateLimit
            )
        });
        match self.status {
            ParameterBindingStatus::Complete => self.omitted_sites == 0 && self.reasons.is_empty(),
            ParameterBindingStatus::Partial => {
                !self.reasons.is_empty() && (self.omitted_sites > 0) == enumerated
            }
            ParameterBindingStatus::Unsupported => {
                self.sites.is_empty()
                    && self.omitted_sites == 0
                    && !self.reasons.is_empty()
                    && !enumerated
            }
        }
    }
}

/// Supported bounded identifier shape. This is not a source/AST role proof.
/// Unicode XID spellings, TS dollar identifiers and Rust raw identifiers are
/// retained whole; escaped JS spellings are explicitly unavailable in profile6.
fn parameter_spelling(text: &str, language: &str) -> bool {
    if text.is_empty() || text.len() > 256 {
        return false;
    }
    let text = if language == "rust" {
        text.strip_prefix("r#").unwrap_or(text)
    } else {
        text
    };
    let dollar = matches!(language, "typescript" | "tsx");
    let mut chars = text.chars();
    chars
        .next()
        .is_some_and(|c| c == '_' || (dollar && c == '$') || unicode_ident::is_xid_start(c))
        && chars.all(|c| c == '_' || (dollar && c == '$') || unicode_ident::is_xid_continue(c))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceQuerySite {
    pub source: SourceAnchor,
    pub text: String,
}

/// Lexically nested anonymous-callable inspection; not a call relationship.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceQueryNestedContext {
    pub source: SourceAnchor,
    pub syntax: String,
    pub code_windows: Vec<SourceQuerySite>,
    pub omitted_code_windows: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceQueryOwner {
    pub node_id: String,
    pub source: SourceAnchor,
    pub source_digest: String,
    pub identifiers: Vec<SourceQuerySite>,
    pub comments: Vec<SourceQuerySite>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameter_bindings: Option<ParameterBindings>,
    /// Exact AST-owned text. Profiles 1–4 use it only for inspection; profiles 5–7
    /// also index vocabulary absent from the identifier/comment fields.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub code_windows: Vec<SourceQuerySite>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub omitted_code_windows: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nested_contexts: Vec<SourceQueryNestedContext>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub omitted_nested_contexts: u64,
    pub omitted_sites: u64,
    pub truncated: bool,
    pub syntax_recovery: bool,
}

/// Per-file coverage survives deterministic fact-neutral incremental updates.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceQueryFile {
    pub file: String,
    pub source_digest: String,
    pub byte_size: u64,
    pub language: String,
    pub truncated: bool,
    pub syntax_recovery: bool,
    pub omitted_owners: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceQueryArtifact {
    pub schema: String,
    pub profile: String,
    pub graph_digest: String,
    pub files: Vec<SourceQueryFile>,
    pub owners: Vec<SourceQueryOwner>,
    pub aggregate_truncated: bool,
    pub omitted_owners: u64,
    pub omitted_files: u64,
    /// Count only owner documents where the term is absent from the complete
    /// graph-only field projection. This adds to the same backend's corpus DF.
    pub additional_document_frequencies: BTreeMap<String, u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("invalid source-query artifact: {0}")]
pub struct SourceQueryError(pub String);

fn is_zero(value: &u64) -> bool {
    *value == 0
}

fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
fn contained(parent: &SourceAnchor, child: &SourceAnchor) -> bool {
    parent.file == child.file
        && parent.start_byte <= child.start_byte
        && child.end_byte <= parent.end_byte
        && (parent.start_line, parent.start_column) <= (child.start_line, child.start_column)
        && (child.end_line, child.end_column) <= (parent.end_line, parent.end_column)
}
fn safe_source(source: &SourceAnchor) -> bool {
    source.is_valid()
        && source.start_byte < source.end_byte
        && source.file.len() <= 4096
        && !source
            .file
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
        && !source.file.contains([':', '\0'])
}

impl SourceQueryOwner {
    /// Exactly the same bounded body projection is used for recall, scoring
    /// and document-frequency increments on every backend.
    #[must_use]
    pub fn document(&self) -> SearchDocument {
        self.project_document(true)
    }

    /// Preserve the lexical/statistical meaning of immutable older profiles.
    pub fn document_for_profile(&self, profile: &str) -> Result<SearchDocument, SourceQueryError> {
        if !is_supported_source_query_profile(profile) {
            return Err(SourceQueryError(
                "unsupported lexical projection profile".to_owned(),
            ));
        }
        Ok(self.project_document(source_query_uses_code_lexemes(profile)))
    }

    fn project_document(&self, include_code: bool) -> SearchDocument {
        let mut result = SearchDocument {
            truncated: self.truncated || (include_code && self.omitted_code_windows > 0),
            ..SearchDocument::default()
        };
        for (field, sites) in [
            ("body_identifier", self.identifiers.as_slice()),
            ("body_comment", self.comments.as_slice()),
            (
                "body_code",
                if include_code {
                    self.code_windows.as_slice()
                } else {
                    &[]
                },
            ),
        ] {
            let mut bytes = 0_usize;
            let mut terms = BTreeSet::new();
            for site in sites {
                let remaining = SEARCH_FIELD_MAX_BYTES.saturating_sub(bytes);
                let mut end = site.text.len().min(remaining);
                while !site.text.is_char_boundary(end) {
                    end -= 1;
                }
                bytes = bytes.saturating_add(end);
                result.truncated |= end < site.text.len();
                terms.extend(
                    identifier_search_terms(&site.text[..end])
                        .into_iter()
                        .map(crate::canonical_code_token),
                );
            }
            if field == "body_code" {
                // A code window repeats identifiers/comments. Do not count those
                // words twice or turn their quotation into corroborating evidence.
                let existing = result.terms();
                terms.retain(|term| !existing.contains(term));
            }
            result.truncated |= terms.len() > SEARCH_FIELD_MAX_TERMS;
            if !terms.is_empty() {
                result.fields.insert(
                    field,
                    terms.into_iter().take(SEARCH_FIELD_MAX_TERMS).collect(),
                );
            }
        }
        result
    }
}

impl SourceQueryArtifact {
    pub fn empty(graph_digest: String) -> Self {
        Self {
            schema: SOURCE_QUERY_SCHEMA.to_owned(),
            profile: SOURCE_QUERY_PROFILE.to_owned(),
            graph_digest,
            files: Vec::new(),
            owners: Vec::new(),
            aggregate_truncated: false,
            omitted_owners: 0,
            omitted_files: 0,
            additional_document_frequencies: BTreeMap::new(),
        }
    }
    pub fn validate(&self) -> Result<(), SourceQueryError> {
        let fail = |message: &str| SourceQueryError(message.to_owned());
        if self.schema != SOURCE_QUERY_SCHEMA || !is_supported_source_query_profile(&self.profile) {
            return Err(fail(
                "unsupported schema or projection profile; update Compass",
            ));
        }
        if !digest(&self.graph_digest) {
            return Err(fail("invalid graph digest"));
        }
        if self.files.len() > SOURCE_QUERY_MAX_OWNERS || self.owners.len() > SOURCE_QUERY_MAX_OWNERS
        {
            return Err(fail("owner limit exceeded"));
        }
        let mut previous_file = None::<&str>;
        let mut files = BTreeMap::new();
        for file in &self.files {
            if file.file.is_empty()
                || file.file.len() > 4096
                || previous_file.is_some_and(|p| p >= file.file.as_str())
                || !file
                    .source_digest
                    .strip_prefix("sha256:")
                    .is_some_and(digest)
                || !matches!(
                    file.language.as_str(),
                    "python" | "rust" | "go" | "typescript" | "tsx"
                )
            {
                return Err(fail("invalid or noncanonical source-file inventory"));
            }
            previous_file = Some(&file.file);
            files.insert(file.file.as_str(), file);
        }
        let mut previous = None::<&str>;
        let mut total_code_sites = 0_usize;
        let mut total_code_text = 0_usize;
        let mut parameter_file_bytes = BTreeMap::<&str, usize>::new();
        let mut file_text_bytes = BTreeMap::<&str, usize>::new();
        let mut total_sites = 0_usize;
        let mut total_text = 0_usize;
        let mut body_frequencies = BTreeMap::<String, u64>::new();
        for owner in &self.owners {
            if owner.node_id.is_empty()
                || owner.node_id.len() > 4096
                || previous.is_some_and(|p| p >= owner.node_id.as_str())
            {
                return Err(fail(
                    "owners must have bounded unique IDs in canonical order",
                ));
            }
            previous = Some(&owner.node_id);
            if !safe_source(&owner.source)
                || !owner
                    .source_digest
                    .strip_prefix("sha256:")
                    .is_some_and(digest)
            {
                return Err(fail("invalid owner source or digest"));
            }
            if !files.get(owner.source.file.as_str()).is_some_and(|file| {
                file.source_digest == owner.source_digest && owner.source.end_byte <= file.byte_size
            }) {
                return Err(fail("owner does not match the source-file inventory"));
            }
            if owner.identifiers.len() > 1024
                || owner.comments.len() > 8
                || owner.code_windows.len() > 16
            {
                return Err(fail("per-owner site limit exceeded"));
            }
            if !source_query_uses_code_windows(&self.profile)
                && (!owner.code_windows.is_empty() || owner.omitted_code_windows > 0)
            {
                return Err(fail("legacy profile does not support code windows"));
            }
            if source_query_uses_parameter_bindings(&self.profile)
                != owner.parameter_bindings.is_some()
            {
                return Err(fail(
                    "parameter coverage must be explicit only under supported binding profiles",
                ));
            }
            let parameter_sites = if let Some(bindings) = &owner.parameter_bindings {
                if bindings.sites.len() > SOURCE_QUERY_MAX_PARAMETER_SITES
                    || bindings.omitted_sites > 200_000
                    || !bindings.has_coherent_coverage()
                {
                    return Err(fail("invalid parameter binding coverage or bounds"));
                }
                for binding in &bindings.sites {
                    if !files
                        .get(owner.source.file.as_str())
                        .is_some_and(|file| parameter_spelling(&binding.text, &file.language))
                    {
                        return Err(fail("parameter binding must be a whole identifier"));
                    }
                    let bytes = parameter_file_bytes.entry(&owner.source.file).or_default();
                    *bytes = bytes.saturating_add(binding.text.len());
                    if *bytes > SOURCE_QUERY_MAX_PARAMETER_FILE_BYTES {
                        return Err(fail("per-file parameter binding limit exceeded"));
                    }
                }
                bindings.sites.as_slice()
            } else {
                &[]
            };
            for (sites, cap, code) in [
                (owner.identifiers.as_slice(), 256_usize, false),
                (owner.comments.as_slice(), 4096, false),
                (owner.code_windows.as_slice(), 512, true),
                (parameter_sites, 256, false),
            ] {
                let mut previous = None;
                for site in sites {
                    let key = (site.source.start_byte, site.source.end_byte);
                    if previous.is_some_and(|p| p >= key) {
                        return Err(fail("sites must be distinct and in source order"));
                    }
                    previous = Some(key);
                    if !safe_source(&site.source)
                        || !contained(&owner.source, &site.source)
                        || site.text.is_empty()
                        || site.text.len() > cap
                        || site.source.end_byte - site.source.start_byte != site.text.len() as u64
                    {
                        return Err(fail("invalid source excerpt"));
                    }
                    let mut line = site.source.start_line;
                    let mut column = site.source.start_column;
                    for byte in site.text.bytes() {
                        if byte == b'\n' {
                            line = line
                                .checked_add(1)
                                .ok_or_else(|| fail("source line overflow"))?;
                            column = 0;
                        } else {
                            column = column
                                .checked_add(1)
                                .ok_or_else(|| fail("source column overflow"))?;
                        }
                    }
                    if (line, column) != (site.source.end_line, site.source.end_column) {
                        return Err(fail("excerpt coordinates disagree with UTF-8 bytes"));
                    }
                    if code {
                        total_code_sites = total_code_sites.saturating_add(1);
                        total_code_text = total_code_text.saturating_add(site.text.len());
                        if total_code_sites > SOURCE_QUERY_MAX_CODE_WINDOWS
                            || total_code_text > SOURCE_QUERY_MAX_CODE_BYTES
                        {
                            return Err(fail("aggregate code window limit exceeded"));
                        }
                        continue;
                    }
                    total_sites = total_sites.saturating_add(1);
                    total_text = total_text.saturating_add(site.text.len());
                    if source_query_uses_parameter_bindings(&self.profile) {
                        let bytes = file_text_bytes.entry(&owner.source.file).or_default();
                        *bytes = bytes.saturating_add(site.text.len());
                        if *bytes > SOURCE_QUERY_MAX_FILE_TEXT_BYTES {
                            return Err(fail("per-file lexical and parameter limit exceeded"));
                        }
                    }
                    if total_sites > SOURCE_QUERY_MAX_SITES
                        || total_text > SOURCE_QUERY_MAX_TEXT_BYTES
                    {
                        return Err(fail("aggregate source evidence limit exceeded"));
                    }
                }
            }
            if owner.nested_contexts.len() > SOURCE_QUERY_MAX_NESTED_CONTEXTS
                || (!source_query_uses_nested_context(&self.profile)
                    && (!owner.nested_contexts.is_empty() || owner.omitted_nested_contexts > 0))
            {
                return Err(fail(
                    "nested context is oversized or unsupported by this profile",
                ));
            }
            let mut previous_context = None;
            let mut owner_code_sites = owner.code_windows.len();
            for context in &owner.nested_contexts {
                let key = (context.source.start_byte, context.source.end_byte);
                if previous_context.is_some_and(|prior| prior >= key)
                    || !safe_source(&context.source)
                    || !contained(&owner.source, &context.source)
                    || (context.source.start_byte == owner.source.start_byte
                        && context.source.end_byte == owner.source.end_byte)
                    || context.code_windows.is_empty()
                {
                    return Err(fail("invalid or noncanonical nested callable context"));
                }
                let supported_syntax = files.get(owner.source.file.as_str()).is_some_and(|file| {
                    matches!(
                        (file.language.as_str(), context.syntax.as_str()),
                        ("python", "lambda")
                            | ("rust", "closure_expression")
                            | ("go", "func_literal")
                            | (
                                "typescript" | "tsx",
                                "arrow_function" | "function_expression" | "generator_function"
                            )
                    )
                });
                if !supported_syntax {
                    return Err(fail(
                        "nested callable syntax does not match the source language",
                    ));
                }
                previous_context = Some(key);
                let mut previous_window = None;
                for window in &context.code_windows {
                    let key = (window.source.start_byte, window.source.end_byte);
                    if previous_window.is_some_and(|prior| prior >= key)
                        || !safe_source(&window.source)
                        || !contained(&context.source, &window.source)
                        || window.text.is_empty()
                        || window.text.len() > 512
                        || window.source.end_byte - window.source.start_byte
                            != window.text.len() as u64
                    {
                        return Err(fail("invalid nested code quotation"));
                    }
                    previous_window = Some(key);
                    let mut line = window.source.start_line;
                    let mut column = window.source.start_column;
                    for byte in window.text.bytes() {
                        if byte == b'\n' {
                            line = line
                                .checked_add(1)
                                .ok_or_else(|| fail("source line overflow"))?;
                            column = 0;
                        } else {
                            column = column
                                .checked_add(1)
                                .ok_or_else(|| fail("source column overflow"))?;
                        }
                    }
                    if (line, column) != (window.source.end_line, window.source.end_column) {
                        return Err(fail("nested quotation coordinates disagree with bytes"));
                    }
                    owner_code_sites += 1;
                    total_code_sites = total_code_sites.saturating_add(1);
                    total_code_text = total_code_text.saturating_add(window.text.len());
                    if owner_code_sites > 16
                        || total_code_sites > SOURCE_QUERY_MAX_CODE_WINDOWS
                        || total_code_text > SOURCE_QUERY_MAX_CODE_BYTES
                    {
                        return Err(fail("nested code exceeds shared code-window limits"));
                    }
                }
            }
            for term in owner.document_for_profile(&self.profile)?.terms() {
                *body_frequencies.entry(term).or_default() += 1;
            }
        }
        if !self
            .additional_document_frequencies
            .keys()
            .eq(body_frequencies.keys())
            || self
                .additional_document_frequencies
                .iter()
                .any(|(term, df)| *df > body_frequencies.get(term).copied().unwrap_or(0))
        {
            return Err(fail("invalid additional document frequency"));
        }
        Ok(())
    }
}
