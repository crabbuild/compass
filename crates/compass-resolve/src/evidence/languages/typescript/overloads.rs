//! Pure callable overload matching and type-argument inference.

use super::*;

pub(in crate::evidence) fn typescript_generic_parameter_names(signature: &str) -> Vec<String> {
    let signature = signature
        .trim()
        .split(['=', '|'])
        .next()
        .unwrap_or_default()
        .trim();
    if !signature.starts_with('<') || !signature.ends_with('>') {
        return Vec::new();
    }
    let body = &signature[1..signature.len().saturating_sub(1)];
    let mut names = Vec::new();
    for name in body.split(',').map(str::trim) {
        if name.is_empty()
            || name.len() > 128
            || !name.chars().all(|character| {
                character == '_' || character == '$' || character.is_ascii_alphanumeric()
            })
        {
            return Vec::new();
        }
        names.push(name.to_owned());
        if names.len() > 64 {
            return Vec::new();
        }
    }
    names
}

pub(in crate::evidence) fn typescript_callable_return_type(signature: &str) -> Option<&str> {
    let (_, return_type) = signature.rsplit_once("|return:")?;
    let return_type = return_type.trim();
    (!return_type.is_empty() && return_type.len() <= 1024).then_some(return_type)
}

pub(in crate::evidence) fn typescript_value_type(signature: &str) -> Option<&str> {
    let type_name = signature.trim().strip_prefix("|type:")?.trim();
    (!type_name.is_empty() && type_name.len() <= 1024).then_some(type_name)
}

pub(in crate::evidence) fn typescript_callable_parameter_types(
    signature: &str,
) -> Option<Vec<String>> {
    let (_, parameters) = signature.split_once("|params:")?;
    let parameters = parameters
        .split_once("|return:")
        .map_or(parameters, |(parameters, _)| parameters)
        .trim();
    if parameters.is_empty() {
        return Some(Vec::new());
    }
    let wrapped = format!("Parameters<{parameters}>");
    typescript_generic_type_parts(&wrapped).map(|(_, arguments)| arguments)
}

pub(in crate::evidence) fn typescript_candidate_argument_types(
    candidate: &RelationshipCandidate,
) -> Vec<String> {
    candidate
        .constraints
        .argument_types
        .iter()
        .map(|argument| argument.clone().unwrap_or_else(|| "__unknown".to_owned()))
        .collect()
}

pub(in crate::evidence) fn typescript_callable_overload_matches(
    declaration: &DeclarationFact,
    call_argument_types: &[String],
    call_type_arguments: &[String],
    parameter_aliases: &AHashMap<String, String>,
) -> bool {
    let Some(signature) = declaration.signature.as_deref() else {
        return false;
    };
    let generic_parameters = typescript_generic_parameter_names(signature);
    if !call_type_arguments.is_empty() && call_type_arguments.len() != generic_parameters.len() {
        return false;
    }
    let mut inferred_arguments = if call_type_arguments.is_empty() {
        generic_parameters.clone()
    } else {
        call_type_arguments.to_vec()
    };
    if let Some(parameter_types) = typescript_callable_parameter_types(signature) {
        if parameter_types.len() != call_argument_types.len() {
            return false;
        }
        for (parameter, argument) in parameter_types.iter().zip(call_argument_types) {
            if argument == "__unknown" {
                if call_type_arguments.is_empty()
                    || generic_parameters.iter().all(|name| {
                        !typescript_type_mentions_parameter(parameter, std::slice::from_ref(name))
                    })
                {
                    return false;
                }
                continue;
            }
            if !typescript_callable_parameter_matches(
                parameter,
                argument,
                &generic_parameters,
                &mut inferred_arguments,
                parameter_aliases,
            ) {
                return false;
            }
        }
        return true;
    }
    declaration
        .parameter_count
        .and_then(|count| usize::try_from(count).ok())
        .is_some_and(|count| {
            count == call_argument_types.len()
                && call_argument_types
                    .iter()
                    .all(|argument| argument != "__unknown")
        })
}

pub(in crate::evidence) fn typescript_callable_parameter_matches(
    parameter: &str,
    argument: &str,
    generic_parameters: &[String],
    inferred_arguments: &mut [String],
    parameter_aliases: &AHashMap<String, String>,
) -> bool {
    typescript_callable_parameter_matches_at_depth(
        parameter,
        argument,
        generic_parameters,
        inferred_arguments,
        parameter_aliases,
        0,
    )
}

pub(in crate::evidence) fn typescript_callable_parameter_matches_at_depth(
    parameter: &str,
    argument: &str,
    generic_parameters: &[String],
    inferred_arguments: &mut [String],
    parameter_aliases: &AHashMap<String, String>,
    depth: usize,
) -> bool {
    if depth > 32 {
        return false;
    }
    let parameter = parameter.trim();
    let argument = argument.trim();
    if parameter.is_empty() || argument.is_empty() {
        return false;
    }
    if let Some(index) = generic_parameters.iter().position(|name| name == parameter) {
        let Some(inferred) = inferred_arguments.get_mut(index) else {
            return false;
        };
        if inferred == parameter {
            inferred.clone_from(&argument.to_owned());
            return true;
        }
        return inferred == argument;
    }
    if let Some(alias) = parameter_aliases.get(parameter) {
        return alias == argument;
    }
    if parameter == argument {
        return true;
    }
    if let (Some(parameter_element), Some(argument_element)) =
        (parameter.strip_suffix("[]"), argument.strip_suffix("[]"))
    {
        return typescript_callable_parameter_matches_at_depth(
            parameter_element,
            argument_element,
            generic_parameters,
            inferred_arguments,
            parameter_aliases,
            depth.saturating_add(1),
        );
    }
    let Some((parameter_base, parameter_arguments)) = typescript_generic_type_parts(parameter)
    else {
        return false;
    };
    let Some((argument_base, argument_arguments)) = typescript_generic_type_parts(argument) else {
        return false;
    };
    parameter_base == argument_base
        && parameter_arguments.len() == argument_arguments.len()
        && parameter_arguments
            .iter()
            .zip(argument_arguments.iter())
            .all(|(parameter_argument, argument_argument)| {
                typescript_callable_parameter_matches_at_depth(
                    parameter_argument,
                    argument_argument,
                    generic_parameters,
                    inferred_arguments,
                    parameter_aliases,
                    depth.saturating_add(1),
                )
            })
}

pub(in crate::evidence) fn typescript_infer_type_arguments(
    parameter: &str,
    argument: &str,
    parameters: &[String],
    arguments: &mut [String],
) -> bool {
    let parameter = parameter.trim();
    let argument = argument.trim();
    if parameter.is_empty() || argument.is_empty() {
        return true;
    }
    if let Some(index) = parameters.iter().position(|name| name == parameter) {
        let Some(slot) = arguments.get_mut(index) else {
            return true;
        };
        if slot
            == parameters
                .get(index)
                .map(String::as_str)
                .unwrap_or_default()
        {
            slot.clone_from(&argument.to_owned());
            return true;
        }
        return slot == argument;
    }
    if let (Some(parameter_element), Some(argument_element)) =
        (parameter.strip_suffix("[]"), argument.strip_suffix("[]"))
    {
        return typescript_infer_type_arguments(
            parameter_element,
            argument_element,
            parameters,
            arguments,
        );
    }
    let Some((parameter_base, parameter_arguments)) = typescript_generic_type_parts(parameter)
    else {
        return true;
    };
    let Some((argument_base, argument_arguments)) = typescript_generic_type_parts(argument) else {
        return true;
    };
    if parameter_base != argument_base || parameter_arguments.len() != argument_arguments.len() {
        return true;
    }
    for (parameter_argument, argument_argument) in
        parameter_arguments.iter().zip(argument_arguments.iter())
    {
        if !typescript_infer_type_arguments(
            parameter_argument,
            argument_argument,
            parameters,
            arguments,
        ) {
            return false;
        }
    }
    true
}

pub(in crate::evidence) fn typescript_type_mentions_parameter(
    type_name: &str,
    parameters: &[String],
) -> bool {
    type_name
        .split(|character: char| {
            !(character == '_' || character == '$' || character.is_ascii_alphanumeric())
        })
        .filter(|token| !token.is_empty())
        .any(|token| parameters.iter().any(|parameter| parameter == token))
}
