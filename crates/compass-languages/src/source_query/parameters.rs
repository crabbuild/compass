//! Direct parameter binding positions in the already parsed native tree.
//! This channel has independent retention after generic lexical evidence.
use super::{Site, site};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use tree_sitter::Node;

pub const MAX_PARAMETER_SITES: usize = 64;
pub const MAX_PARAMETER_FILE_BYTES: usize = 256 * 1024;

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
    pub sites: Vec<Site>,
    pub status: ParameterBindingStatus,
    /// Known omitted bindings only; unsupported/truncated suffixes are unknown.
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
    fn partial(&mut self, reason: ParameterBindingOmission) {
        self.status = ParameterBindingStatus::Partial;
        self.reasons.insert(reason);
    }
    fn unsupported(reason: ParameterBindingOmission) -> Self {
        Self {
            status: ParameterBindingStatus::Unsupported,
            reasons: BTreeSet::from([reason]),
            ..Self::default()
        }
    }
}

fn charge(visits: &mut usize) -> Result<(), ParameterBindingOmission> {
    if *visits == 0 {
        return Err(ParameterBindingOmission::TraversalLimit);
    }
    *visits -= 1;
    Ok(())
}

/// Follow only binding fields and audited one-child binding wrappers. In
/// particular, never walk defaults, type subtrees, or arbitrary descendants.
fn binding<'tree>(
    mut node: Node<'tree>,
    language: &str,
    visits: &mut usize,
) -> Result<Option<Node<'tree>>, ParameterBindingOmission> {
    for _ in 0..4 {
        charge(visits)?;
        if node.has_error() || node.is_missing() {
            return Err(ParameterBindingOmission::SyntaxRecovery);
        }
        if node.kind() == "identifier" {
            return Ok(Some(node));
        }
        let child = match (language, node.kind()) {
            ("python", "default_parameter" | "typed_default_parameter") => {
                node.child_by_field_name("name")
            }
            ("python", "typed_parameter") => node.named_child(0).filter(|child| {
                node.child_by_field_name("type")
                    .is_none_or(|ty| ty.id() != child.id())
            }),
            ("python", "list_splat_pattern" | "dictionary_splat_pattern")
            | ("typescript" | "tsx", "rest_pattern") => (node.named_child_count() == 1)
                .then(|| node.named_child(0))
                .flatten(),
            ("rust", "parameter" | "reference_pattern" | "mut_pattern") => {
                node.child_by_field_name("pattern")
            }
            ("rust", "self_parameter") => {
                return Err(ParameterBindingOmission::UnsupportedReceiver);
            }
            ("rust", "_") | ("typescript" | "tsx", "this") => return Ok(None),
            ("typescript" | "tsx", "required_parameter" | "optional_parameter") => {
                node.child_by_field_name("pattern")
            }
            _ => return Err(ParameterBindingOmission::UnsupportedPattern),
        };
        let Some(child) = child else {
            return Err(ParameterBindingOmission::UnsupportedPattern);
        };
        node = child;
    }
    Err(ParameterBindingOmission::UnsupportedPattern)
}

fn retain(
    result: &mut ParameterBindings,
    node: Node<'_>,
    source: &[u8],
    language: &str,
    remaining_text: &mut usize,
) {
    if language == "go" && source.get(node.byte_range()) == Some(b"_".as_slice()) {
        return;
    }
    if node.end_byte().saturating_sub(node.start_byte()) > 256 {
        result.omitted_sites = result.omitted_sites.saturating_add(1);
        result.partial(ParameterBindingOmission::TextLimit);
        return;
    }
    if !source
        .get(node.byte_range())
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .is_some_and(|text| supported_spelling(text, language))
    {
        result.omitted_sites = result.omitted_sites.saturating_add(1);
        result.partial(ParameterBindingOmission::UnsupportedSpelling);
        return;
    }
    if result.sites.len() >= MAX_PARAMETER_SITES {
        result.omitted_sites = result.omitted_sites.saturating_add(1);
        result.partial(ParameterBindingOmission::SiteLimit);
        return;
    }
    let Some(value) = site(node, source, 256) else {
        result.omitted_sites = result.omitted_sites.saturating_add(1);
        result.partial(ParameterBindingOmission::UnsupportedSpelling);
        return;
    };
    if value.end != node.end_byte() || value.text.len() > *remaining_text {
        result.omitted_sites = result.omitted_sites.saturating_add(1);
        result.partial(ParameterBindingOmission::TextLimit);
        return;
    }
    *remaining_text -= value.text.len();
    result.sites.push(value);
}

/// Bounded spelling shape only, not independent proof of AST binding role.
/// Escaped JS spellings are deliberately unavailable in this profile.
fn supported_spelling(text: &str, language: &str) -> bool {
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

pub(super) fn collect(
    callable: Node<'_>,
    source: &[u8],
    language: &str,
    remaining_visits: &mut usize,
    remaining_text: &mut usize,
) -> ParameterBindings {
    if let Err(reason) = charge(remaining_visits) {
        return ParameterBindings::unsupported(reason);
    }
    let mut result = ParameterBindings::default();
    let Some(parameters) = callable.child_by_field_name("parameters") else {
        if matches!(language, "typescript" | "tsx")
            && callable.kind() == "arrow_function"
            && let Some(parameter) = callable.child_by_field_name("parameter")
        {
            match binding(parameter, language, remaining_visits) {
                Ok(Some(node)) => retain(&mut result, node, source, language, remaining_text),
                Ok(None) => {}
                Err(reason) => result.partial(reason),
            }
            return result;
        }
        if language == "python" && callable.kind() == "lambda" {
            return result;
        }
        return ParameterBindings::unsupported(ParameterBindingOmission::UnsupportedPattern);
    };
    if let Err(reason) = charge(remaining_visits) {
        return ParameterBindings::unsupported(reason);
    }
    let mut cursor = parameters.walk();
    for parameter in parameters.named_children(&mut cursor) {
        if language == "python"
            && matches!(
                parameter.kind(),
                "positional_separator" | "keyword_separator"
            )
        {
            if let Err(reason) = charge(remaining_visits) {
                result.partial(reason);
                break;
            }
            continue;
        }
        if language == "go" {
            if let Err(reason) = charge(remaining_visits) {
                result.partial(reason);
                break;
            }
            if !matches!(
                parameter.kind(),
                "parameter_declaration" | "variadic_parameter_declaration"
            ) {
                result.partial(ParameterBindingOmission::UnsupportedPattern);
                continue;
            }
            let mut names = parameter.walk();
            for name in parameter.children_by_field_name("name", &mut names) {
                match binding(name, language, remaining_visits) {
                    Ok(Some(node)) => retain(&mut result, node, source, language, remaining_text),
                    Ok(None) => {}
                    Err(reason) => {
                        result.partial(reason);
                        if *remaining_visits == 0 {
                            break;
                        }
                    }
                }
            }
        } else {
            match binding(parameter, language, remaining_visits) {
                Ok(Some(node)) => retain(&mut result, node, source, language, remaining_text),
                Ok(None) => {}
                Err(reason) => result.partial(reason),
            }
        }
        if *remaining_visits == 0 {
            result.partial(ParameterBindingOmission::TraversalLimit);
            break;
        }
    }
    result.sites.sort_by_key(|site| (site.start, site.end));
    result
}
