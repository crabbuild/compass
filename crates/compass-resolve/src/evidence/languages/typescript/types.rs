//! Pure TypeScript type-expression parsing and substitution helpers.

use super::*;

pub(in crate::evidence) fn typescript_language_family(language: &str) -> &'static [&'static str] {
    match language {
        "typescript" => &["typescript", "javascript"],
        "javascript" => &["javascript", "typescript"],
        _ => &[],
    }
}

pub(in crate::evidence) fn typescript_member_path(
    qualified: &str,
    target_spelling: &str,
) -> Option<TypeScriptMemberPath> {
    let (_, path) = split_typescript_module_qualified(qualified);
    let segments = split_typescript_member_segments(path);
    if segments.len() < 2 || segments.last()?.as_str() != target_spelling {
        return None;
    }
    let root_segment = segments.first()?;
    let indexed = root_segment.ends_with("[]");
    let root_segment = root_segment
        .strip_suffix("[]")
        .unwrap_or(root_segment)
        .trim();
    let (root_base, mut call_result, mut call_argument_types, mut call_type_arguments) =
        if let Some((base, arguments, type_arguments)) = typescript_call_result_marker(root_segment)
        {
            (base, true, arguments, type_arguments)
        } else {
            (root_segment.to_owned(), false, Vec::new(), Vec::new())
        };
    let (root_export, type_arguments) = typescript_generic_type_parts(&root_base).map_or_else(
        || (root_base.clone(), Vec::new()),
        |(base, arguments)| (base.to_owned(), arguments),
    );
    let mut members = segments.into_iter().skip(1).collect::<Vec<_>>();
    let member_count = members.len();
    let mut call_member_index = None;
    for (index, member) in members.iter_mut().enumerate() {
        let Some((base, arguments, type_arguments)) = typescript_call_result_marker(member) else {
            continue;
        };
        if call_result || index.saturating_add(1) >= member_count {
            return None;
        }
        *member = base;
        call_result = true;
        call_argument_types = arguments;
        call_type_arguments = type_arguments;
        call_member_index = Some(index);
    }
    if root_export.is_empty() {
        return None;
    }
    Some(TypeScriptMemberPath {
        root_export,
        type_arguments,
        call_result,
        call_argument_types,
        call_type_arguments,
        call_member_index,
        indexed,
        members,
    })
}

pub(in crate::evidence) fn typescript_call_result_marker(
    value: &str,
) -> Option<(String, Vec<String>, Vec<String>)> {
    let value = value.trim();
    let mut call_value = value;
    let type_arguments = if let Some(marker) = call_value.find("#types<") {
        let payload_start = marker.saturating_add("#types<".len());
        let payload = call_value.get(payload_start..)?.strip_suffix('>')?;
        let marker_end = payload_start
            .saturating_add(payload.len())
            .saturating_add(1);
        if marker_end != call_value.len() || payload.is_empty() {
            return None;
        }
        call_value = call_value.get(..marker)?.trim();
        let wrapped = format!("Types<{payload}>");
        let (_, arguments) = typescript_generic_type_parts(&wrapped)?;
        arguments
    } else {
        Vec::new()
    };
    let marker = call_value.find("#call<")?;
    let payload_start = marker.saturating_add("#call<".len());
    let payload = call_value.get(payload_start..)?.strip_suffix('>')?;
    let marker_end = payload_start
        .saturating_add(payload.len())
        .saturating_add(1);
    if marker_end != call_value.len() || payload.is_empty() {
        return None;
    }
    let base = call_value.get(..marker)?.trim();
    if base.is_empty() {
        return None;
    }
    let arguments = if payload == "__none" {
        Vec::new()
    } else {
        let wrapped = format!("Call<{payload}>");
        let (_, arguments) = typescript_generic_type_parts(&wrapped)?;
        arguments
    };
    Some((base.to_owned(), arguments, type_arguments))
}

pub(in crate::evidence) fn split_typescript_module_qualified(value: &str) -> (Option<&str>, &str) {
    let bytes = value.as_bytes();
    let mut angle_depth = 0_u32;
    let mut split = None;
    for index in 0..bytes.len().saturating_sub(1) {
        match bytes[index] {
            b'<' => angle_depth = angle_depth.saturating_add(1),
            b'>' => angle_depth = angle_depth.saturating_sub(1),
            b':' if bytes[index + 1] == b':' && angle_depth == 0 => split = Some(index),
            _ => {}
        }
    }
    split.map_or((None, value), |index| {
        (
            value.get(..index).filter(|module| !module.is_empty()),
            value.get(index.saturating_add(2)..).unwrap_or_default(),
        )
    })
}

pub(in crate::evidence) fn split_typescript_member_segments(value: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut start = 0_usize;
    let mut angle_depth = 0_u32;
    let bytes = value.as_bytes();
    let mut index = 0_usize;
    while index < bytes.len() {
        match bytes[index] {
            b'<' => angle_depth = angle_depth.saturating_add(1),
            b'>' => angle_depth = angle_depth.saturating_sub(1),
            b'.' if angle_depth == 0 => {
                if let Some(segment) = value.get(start..index).map(str::trim)
                    && !segment.is_empty()
                {
                    segments.push(segment.to_owned());
                }
                start = index.saturating_add(1);
            }
            b':' if angle_depth == 0 && bytes.get(index.saturating_add(1)) == Some(&b':') => {
                if let Some(segment) = value.get(start..index).map(str::trim)
                    && !segment.is_empty()
                {
                    segments.push(segment.to_owned());
                }
                start = index.saturating_add(2);
                index = index.saturating_add(1);
            }
            _ => {}
        }
        index = index.saturating_add(1);
    }
    if let Some(segment) = value.get(start..).map(str::trim)
        && !segment.is_empty()
    {
        segments.push(segment.to_owned());
    }
    segments
}

pub(in crate::evidence) fn typescript_generic_type_parts(
    value: &str,
) -> Option<(&str, Vec<String>)> {
    let value = value.trim();
    let bytes = value.as_bytes();
    let open = bytes.iter().position(|byte| *byte == b'<')?;
    if open == 0 || !value.ends_with('>') {
        return None;
    }
    let base = value.get(..open)?.trim();
    let arguments = value.get(open.saturating_add(1)..value.len().saturating_sub(1))?;
    if base.is_empty() || arguments.is_empty() || arguments.len() > 1024 {
        return None;
    }
    let mut parts = Vec::new();
    let mut start = 0_usize;
    let mut depth = 0_u32;
    for (index, character) in arguments.char_indices() {
        match character {
            '<' | '[' | '(' | '{' => depth = depth.saturating_add(1),
            '>' | ']' | ')' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                let part = arguments.get(start..index)?.trim();
                if part.is_empty() || part.len() > 1024 {
                    return None;
                }
                parts.push(part.to_owned());
                start = index.saturating_add(1);
            }
            _ => {}
        }
    }
    let part = arguments.get(start..)?.trim();
    if part.is_empty() || part.len() > 1024 {
        return None;
    }
    parts.push(part.to_owned());
    (parts.len() <= 64).then_some((base, parts))
}

pub(in crate::evidence) fn typescript_array_element_type(value: &str) -> Option<String> {
    let value = value.trim();
    if let Some(element) = value.strip_suffix("[]") {
        let element = element.trim();
        return (!element.is_empty() && element.len() <= 1024).then(|| element.to_owned());
    }
    let (base, arguments) = typescript_generic_type_parts(value)?;
    if !matches!(base, "Array" | "ReadonlyArray") || arguments.len() != 1 {
        return None;
    }
    arguments.first().cloned()
}

pub(in crate::evidence) fn typescript_utility_receiver_type(value: &str) -> Option<String> {
    typescript_utility_receiver_type_at_depth(value, 0)
}

pub(in crate::evidence) fn typescript_utility_receiver_type_at_depth(
    value: &str,
    depth: u32,
) -> Option<String> {
    if depth > 32 {
        return None;
    }
    let (base, arguments) = typescript_generic_type_parts(value)?;
    if arguments.len() != 1 {
        return None;
    }
    let argument = arguments[0].trim();
    match base {
        "NonNullable" => {
            let members = typescript_split_top_level_union(argument)?;
            let nominal = members
                .into_iter()
                .map(str::trim)
                .filter(|member| !typescript_non_nominal_union_member(member))
                .collect::<Vec<_>>();
            let [nominal] = nominal.as_slice() else {
                return None;
            };
            Some((*nominal).to_owned())
        }
        "Awaited" => {
            if let Some((promise, nested)) = typescript_generic_type_parts(argument)
                && matches!(promise, "Promise" | "PromiseLike")
                && nested.len() == 1
            {
                return typescript_utility_receiver_type_at_depth(
                    &format!("Awaited<{}>", nested[0]),
                    depth.saturating_add(1),
                )
                .or_else(|| Some(nested[0].clone()));
            }
            Some(argument.to_owned())
        }
        "Partial" | "Required" | "Readonly" => Some(argument.to_owned()),
        _ => None,
    }
}

pub(in crate::evidence) fn typescript_non_nominal_union_member(value: &str) -> bool {
    matches!(
        value,
        "any"
            | "unknown"
            | "never"
            | "void"
            | "undefined"
            | "null"
            | "string"
            | "number"
            | "boolean"
            | "bigint"
            | "symbol"
            | "object"
            | "true"
            | "false"
    )
}

pub(in crate::evidence) fn typescript_split_top_level_union(value: &str) -> Option<Vec<&str>> {
    let mut members = Vec::new();
    let mut start = 0_usize;
    let mut depth = 0_u32;
    for (index, character) in value.char_indices() {
        match character {
            '<' | '{' | '(' | '[' => depth = depth.checked_add(1)?,
            '>' | '}' | ')' | ']' => depth = depth.checked_sub(1)?,
            '|' if depth == 0 => {
                let member = value.get(start..index)?.trim();
                if member.is_empty() {
                    return None;
                }
                members.push(member);
                start = index.saturating_add(character.len_utf8());
            }
            _ => {}
        }
        if members.len() >= 64 {
            return None;
        }
    }
    let member = value.get(start..)?.trim();
    if member.is_empty() {
        return None;
    }
    members.push(member);
    Some(members)
}

pub(in crate::evidence) fn typescript_tuple_elements(value: &str) -> Option<Vec<&str>> {
    let value = value.trim();
    if !value.starts_with('[') || !value.ends_with(']') {
        return None;
    }
    let inner = value.get(1..value.len().saturating_sub(1))?.trim();
    if inner.is_empty() || inner.len() > 1024 {
        return None;
    }
    let mut elements = Vec::new();
    let mut start = 0_usize;
    let mut depth = 0_u32;
    for (index, character) in inner.char_indices() {
        match character {
            '<' | '[' | '(' | '{' => depth = depth.saturating_add(1),
            '>' | ']' | ')' | '}' => depth = depth.checked_sub(1)?,
            ',' if depth == 0 => {
                let element = inner.get(start..index)?.trim();
                if element.is_empty() || element.len() > 1024 {
                    return None;
                }
                elements.push(element);
                start = index.saturating_add(1);
            }
            _ => {}
        }
        if elements.len() >= 64 {
            return None;
        }
    }
    if depth != 0 {
        return None;
    }
    let element = inner.get(start..)?.trim();
    if element.is_empty() || element.len() > 1024 {
        return None;
    }
    elements.push(element);
    Some(elements)
}

pub(in crate::evidence) fn typescript_tuple_element_type(
    value: &str,
    index: &str,
) -> Option<String> {
    let index = index.parse::<usize>().ok()?;
    let element = typescript_tuple_elements(value)?.get(index)?.trim();
    if element.is_empty() || element.starts_with("...") || element.ends_with('?') {
        return None;
    }
    Some(element.to_owned())
}

pub(in crate::evidence) fn typescript_indexed_member_segment(
    value: &str,
) -> Option<(String, Option<String>)> {
    let value = value.trim();
    if let Some(base) = value.strip_suffix("[]") {
        let base = base.trim();
        return (!base.is_empty()).then(|| (base.to_owned(), Some(String::new())));
    }
    if let Some(open) = value.rfind('[')
        && value.ends_with(']')
    {
        let base = value.get(..open)?.trim();
        let index = value.get(open.saturating_add(1)..value.len().saturating_sub(1))?;
        if base.is_empty()
            || index.is_empty()
            || !index.chars().all(|character| character.is_ascii_digit())
        {
            return None;
        }
        return Some((base.to_owned(), Some(index.to_owned())));
    }
    (!value.is_empty()).then(|| (value.to_owned(), None))
}

pub(in crate::evidence) fn typescript_literal_indexed_type(value: &str) -> Option<(&str, String)> {
    let value = value.trim();
    if !value.ends_with(']') {
        return None;
    }
    let open = value.rfind('[')?;
    let base = value.get(..open)?.trim();
    let raw_property = value
        .get(open.saturating_add(1)..value.len().saturating_sub(1))?
        .trim();
    let property = raw_property
        .strip_prefix('"')
        .and_then(|property| property.strip_suffix('"'))
        .or_else(|| {
            raw_property
                .strip_prefix('\'')
                .and_then(|property| property.strip_suffix('\''))
        })?;
    if base.is_empty() || property.is_empty() || property.len() > 1024 {
        return None;
    }
    Some((base, property.to_owned()))
}

pub(in crate::evidence) fn typescript_keyof_type_base(value: &str) -> Option<&str> {
    let rest = value.trim().strip_prefix("keyof")?;
    rest.chars()
        .next()
        .filter(|character| character.is_whitespace())?;
    let base = rest.trim();
    (!base.is_empty()).then_some(base)
}

pub(in crate::evidence) fn typescript_literal_key_names(value: &str) -> Option<BTreeSet<String>> {
    let mut names = BTreeSet::new();
    for key in value.split('|').map(str::trim) {
        let key = key
            .strip_prefix('"')
            .and_then(|key| key.strip_suffix('"'))
            .or_else(|| {
                key.strip_prefix('\'')
                    .and_then(|key| key.strip_suffix('\''))
            })
            .or_else(|| key.strip_prefix('`').and_then(|key| key.strip_suffix('`')))?;
        if key.is_empty() || key.len() > 1024 {
            return None;
        }
        names.insert(key.to_owned());
        if names.len() > 256 {
            return None;
        }
    }
    (!names.is_empty()).then_some(names)
}

pub(in crate::evidence) fn typescript_type_alias_target(signature: &str) -> Option<&str> {
    let alias_separator = signature.find('=');
    if let Some(index_separator) = signature.find("|index=")
        && alias_separator.is_none_or(|separator| separator >= index_separator)
    {
        // `index=` is declaration-shape metadata, not an alias target. A
        // generic interface such as `<T>|index=T` must not be expanded as if
        // its whole receiver were the indexed value.
        return None;
    }
    let (parameters, target) = signature.trim().split_once('=')?;
    let parameters = parameters.trim();
    let target = target
        .split_once("|index=")
        .map_or(target, |(target, _)| target);
    if parameters.contains("|index=")
        || (!parameters.is_empty() && !parameters.starts_with('<'))
        || target.trim().is_empty()
        || target.len() > 1024
    {
        return None;
    }
    Some(target.trim())
}

pub(in crate::evidence) fn typescript_index_value_type(signature: &str) -> Option<&str> {
    let value = signature
        .trim()
        .split_once("|index=")
        .map(|(_, value)| value)
        .or_else(|| signature.trim().strip_prefix("index="))?;
    (!value.is_empty() && value.len() <= 1024).then_some(value.trim())
}

pub(in crate::evidence) fn typescript_substitute_type_parameters(
    type_name: &str,
    parameters: &[String],
    arguments: &[String],
) -> String {
    let type_name = type_name.trim();
    if type_name.is_empty() || type_name.len() > 1024 {
        return type_name.to_owned();
    }
    if let Some(index) = parameters
        .iter()
        .position(|parameter| parameter == type_name)
        && let Some(argument) = arguments.get(index)
    {
        return argument.clone();
    }
    if let Some(key_base) = typescript_keyof_type_base(type_name) {
        let substituted = typescript_substitute_type_parameters(key_base, parameters, arguments);
        let substituted = format!("keyof {substituted}");
        return if substituted.len() <= 1024 {
            substituted
        } else {
            type_name.to_owned()
        };
    }
    if let Some(element) = type_name.strip_suffix("[]") {
        let substituted = typescript_substitute_type_parameters(element, parameters, arguments);
        let substituted = format!("{substituted}[]");
        return if substituted.len() <= 1024 {
            substituted
        } else {
            type_name.to_owned()
        };
    }
    if let Some((base, _)) = typescript_literal_indexed_type(type_name)
        && let Some(open) = type_name.rfind('[')
    {
        let substituted_base = typescript_substitute_type_parameters(base, parameters, arguments);
        let suffix = type_name.get(open..).unwrap_or_default();
        let substituted = format!("{substituted_base}{suffix}");
        return if substituted.len() <= 1024 {
            substituted
        } else {
            type_name.to_owned()
        };
    }
    if let Some(elements) = typescript_tuple_elements(type_name) {
        let substituted = elements
            .iter()
            .map(|element| typescript_substitute_type_parameters(element, parameters, arguments))
            .collect::<Vec<_>>();
        let substituted = format!("[{}]", substituted.join(","));
        return if substituted.len() <= 1024 {
            substituted
        } else {
            type_name.to_owned()
        };
    }
    if let Some(members) = typescript_split_top_level_union(type_name)
        && members.len() > 1
    {
        let substituted = members
            .iter()
            .map(|member| typescript_substitute_type_parameters(member, parameters, arguments))
            .collect::<Vec<_>>()
            .join("|");
        return if substituted.len() <= 1024 {
            substituted
        } else {
            type_name.to_owned()
        };
    }
    let Some((base, nested)) = typescript_generic_type_parts(type_name) else {
        return type_name.to_owned();
    };
    let nested = nested
        .iter()
        .map(|argument| typescript_substitute_type_parameters(argument, parameters, arguments))
        .collect::<Vec<_>>();
    let substituted = format!("{base}<{}>", nested.join(","));
    if substituted.len() <= 1024 {
        substituted
    } else {
        type_name.to_owned()
    }
}

pub(in crate::evidence) fn typescript_declaration_basic_allowed(
    target: &DeclarationFact,
    candidate: &RelationshipCandidate,
) -> bool {
    typescript_declaration_basic_allowed_for(target, candidate, false)
}

pub(in crate::evidence) fn typescript_declaration_basic_allowed_with_type_owner(
    target: &DeclarationFact,
    candidate: &RelationshipCandidate,
) -> bool {
    typescript_declaration_basic_allowed_for(target, candidate, true)
}

pub(in crate::evidence) fn typescript_declaration_basic_allowed_for(
    target: &DeclarationFact,
    candidate: &RelationshipCandidate,
    allow_type_owner: bool,
) -> bool {
    typescript_language_family(&candidate.language).contains(&target.language.as_str())
        && candidate
            .constraints
            .argument_count
            .is_none_or(|arguments| {
                target.parameter_count.is_none_or(|parameters| {
                    arguments == parameters
                        || (target.variadic && arguments >= parameters.saturating_sub(1))
                })
            })
        && (candidate.constraints.allowed_target_kinds.is_empty()
            || candidate
                .constraints
                .allowed_target_kinds
                .contains(&target.kind)
            || (allow_type_owner
                && matches!(
                    target.kind.as_str(),
                    "class" | "enum" | "interface" | "namespace" | "type_alias"
                )))
}
