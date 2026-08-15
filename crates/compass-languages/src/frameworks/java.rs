//! Shared Java source-identity helpers retained by enterprise config packs.
//!
//! Spring source detection lives exclusively in the universal evidence pack.

use regex::Regex;

pub(super) fn java_package_name(body: &str) -> String {
    Regex::new(r"(?m)^\s*package\s+([A-Za-z_$][A-Za-z0-9_$.]*)\s*;")
        .ok()
        .and_then(|pattern| pattern.captures(body))
        .and_then(|capture| capture.get(1))
        .map(|value| value.as_str().to_owned())
        .unwrap_or_else(|| "<default>".to_owned())
}

pub(super) fn java_callable_target(
    package: &str,
    owner: &str,
    name: &str,
    parameters: &str,
) -> (String, String) {
    let owner = if package.is_empty() {
        owner.to_owned()
    } else {
        format!("{package}.{owner}")
    };
    let qualified = format!("{owner}.{name}");
    let parameters = split_java_parameters(parameters)
        .into_iter()
        .filter_map(|parameter| java_parameter_type(&parameter))
        .collect::<Vec<_>>()
        .join(",");
    let signature = format!("{qualified}({parameters})");
    (qualified, signature)
}

fn split_java_parameters(parameters: &str) -> Vec<String> {
    let mut output = Vec::new();
    let mut start = 0;
    let mut depth = 0_u32;
    for (offset, character) in parameters.char_indices() {
        match character {
            '<' | '(' | '[' => depth = depth.saturating_add(1),
            '>' | ')' | ']' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                output.push(parameters[start..offset].trim().to_owned());
                start = offset.saturating_add(1);
            }
            _ => {}
        }
    }
    let tail = parameters[start..].trim();
    if !tail.is_empty() {
        output.push(tail.to_owned());
    }
    output
}

fn java_parameter_type(parameter: &str) -> Option<String> {
    let stripped = strip_java_parameter_annotations(parameter);
    let stripped = stripped
        .split_whitespace()
        .filter(|token| !matches!(*token, "final" | "volatile" | "transient"))
        .collect::<Vec<_>>();
    if stripped.len() < 2 {
        return None;
    }
    let raw_type = stripped[..stripped.len().saturating_sub(1)].join("");
    let mut normalized = String::with_capacity(raw_type.len());
    let mut generic_depth = 0_u32;
    for character in raw_type.chars() {
        match character {
            '<' => generic_depth = generic_depth.saturating_add(1),
            '>' => generic_depth = generic_depth.saturating_sub(1),
            _ if generic_depth > 0 => {}
            character if character.is_whitespace() => {}
            character
                if character.is_alphanumeric()
                    || matches!(character, '_' | '$' | '.' | '[' | ']') =>
            {
                normalized.push(character);
            }
            _ => {}
        }
    }
    if parameter.contains("...") {
        normalized.push_str("...");
    }
    (!normalized.is_empty()).then_some(normalized)
}

fn strip_java_parameter_annotations(parameter: &str) -> String {
    let mut output = String::with_capacity(parameter.len());
    let chars = parameter.chars().collect::<Vec<_>>();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] != '@' {
            output.push(chars[index]);
            index += 1;
            continue;
        }
        index += 1;
        while index < chars.len()
            && (chars[index].is_alphanumeric() || matches!(chars[index], '_' | '$' | '.'))
        {
            index += 1;
        }
        if index < chars.len() && chars[index] == '(' {
            let mut depth = 1_u32;
            index += 1;
            while index < chars.len() && depth > 0 {
                match chars[index] {
                    '(' => depth = depth.saturating_add(1),
                    ')' => depth = depth.saturating_sub(1),
                    _ => {}
                }
                index += 1;
            }
        }
        output.push(' ');
    }
    output
}
