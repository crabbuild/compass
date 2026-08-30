//! Kotlin overload, named/default argument, and extension-call policy.
//!
//! This module never searches Java declarations. Cross-language JVM edges are
//! admitted only through exact compiler/SCIP evidence outside structural
//! `SemanticEvidenceBatch` resolution.

use super::super::*;

#[derive(Clone, Debug)]
struct ParameterShape {
    name: String,
    kind: String,
    defaulted: bool,
    variadic: bool,
}

#[derive(Clone, Debug)]
struct CallableShape {
    receiver: Option<String>,
    parameters: Vec<ParameterShape>,
}

impl ResolutionDb<'_> {
    pub(in crate::evidence) fn resolve_kotlin_candidate(
        &self,
        candidate: &RelationshipCandidate,
    ) -> Option<ResolutionDecision> {
        if candidate.language != "kotlin" || !matches!(candidate.relation, CandidateRelation::Calls)
        {
            return None;
        }
        let argument_names = self
            .occurrence(candidate)
            .and_then(OccurrenceRef::context)
            .and_then(parse_argument_names)?;
        let receiver = candidate
            .constraints
            .hierarchy
            .as_ref()
            .and_then(|hierarchy| {
                if let HierarchyConstraint::ReceiverDispatch {
                    receiver_qualified_name,
                    ..
                } = hierarchy
                {
                    Some(receiver_qualified_name.as_str())
                } else {
                    None
                }
            });

        // Kotlin member declarations always shadow extension functions.
        let member_qualified = candidate.constraints.qualified_name.clone().or_else(|| {
            receiver.map(|receiver| format!("{receiver}::{}", candidate.target_spelling))
        });
        if let Some(qualified) = member_qualified.as_deref()
            && let Some(decision) = self.kotlin_unique_applicable(
                qualified,
                receiver,
                &argument_names,
                candidate,
                false,
            )
        {
            return Some(decision);
        }

        let binding = candidate
            .binding_id
            .as_deref()
            .and_then(|id| self.facts.bindings.get(id))?;
        let qualified = imported_callable_name(&binding.qualified_target);
        self.kotlin_unique_applicable(
            &qualified,
            receiver,
            &argument_names,
            candidate,
            receiver.is_some(),
        )
    }

    fn kotlin_unique_applicable(
        &self,
        qualified: &str,
        receiver: Option<&str>,
        argument_names: &[Option<String>],
        candidate: &RelationshipCandidate,
        require_extension: bool,
    ) -> Option<ResolutionDecision> {
        let slots = self
            .indexes
            .names
            .by_qualified
            .get(&("kotlin".to_owned(), qualified.to_owned()))?;
        let mut eligible = slots
            .iter()
            .filter_map(|slot| self.declaration(*slot))
            .filter(|declaration| {
                declaration.language == "kotlin"
                    && matches!(declaration.kind.as_str(), "function" | "method")
                    && (candidate.constraints.allowed_target_kinds.is_empty()
                        || candidate
                            .constraints
                            .allowed_target_kinds
                            .contains(&declaration.kind))
            })
            .filter_map(|declaration| {
                let shape = parse_callable_shape(declaration.signature.as_deref()?)?;
                if require_extension != shape.receiver.is_some() {
                    return None;
                }
                if let (Some(expected), Some(actual)) = (shape.receiver.as_deref(), receiver)
                    && !kotlin_types_compatible(expected, actual)
                {
                    return None;
                }
                kotlin_arguments_apply(
                    &shape.parameters,
                    argument_names,
                    &candidate.constraints.argument_types,
                )
                .then_some(declaration)
            })
            .take(self.budget.candidates_per_lookup().saturating_add(1));
        let only = eligible.next()?;
        if eligible.next().is_some() {
            return Some(ResolutionDecision::Ambiguous { candidate_count: 2 });
        }
        Some(ResolutionDecision::Resolved {
            declaration_id: only.id.clone(),
            evidence: ResolutionEvidence {
                rule: ResolutionRule::KotlinNamedDefaultArguments,
                candidate_count: 1,
            },
        })
    }
}

fn parse_argument_names(context: &str) -> Option<Vec<Option<String>>> {
    let names = context.strip_prefix("kotlin_args:")?;
    if names.is_empty() {
        return Some(Vec::new());
    }
    Some(
        names
            .split(',')
            .map(|name| (name != "_").then(|| name.to_owned()))
            .collect(),
    )
}

fn parse_callable_shape(signature: &str) -> Option<CallableShape> {
    let (_, body) = signature.split_once('(')?;
    let body = body.strip_suffix(')')?;
    let (receiver, parameters) = body
        .strip_prefix("receiver=")
        .and_then(|body| body.split_once(';'))
        .map_or((None, body), |(receiver, rest)| {
            (Some(receiver.to_owned()), rest)
        });
    let parameters = if parameters.is_empty() {
        Vec::new()
    } else {
        parameters
            .split(',')
            .map(|parameter| {
                let (name, kind) = parameter.split_once(':')?;
                let variadic = kind.ends_with("...");
                let kind = kind.strip_suffix("...").unwrap_or(kind);
                let defaulted = kind.ends_with('=');
                let kind = kind.strip_suffix('=').unwrap_or(kind);
                Some(ParameterShape {
                    name: name.to_owned(),
                    kind: kind.to_owned(),
                    defaulted,
                    variadic,
                })
            })
            .collect::<Option<Vec<_>>>()?
    };
    Some(CallableShape {
        receiver,
        parameters,
    })
}

fn kotlin_arguments_apply(
    parameters: &[ParameterShape],
    argument_names: &[Option<String>],
    argument_types: &[Option<String>],
) -> bool {
    if argument_names.len() != argument_types.len() {
        return false;
    }
    let mut assigned = vec![false; parameters.len()];
    let mut positional = 0_usize;
    for (index, name) in argument_names.iter().enumerate() {
        let parameter = if let Some(name) = name {
            let Some(parameter) = parameters
                .iter()
                .position(|parameter| &parameter.name == name)
            else {
                return false;
            };
            parameter
        } else {
            while assigned.get(positional).copied().unwrap_or(false) {
                positional = positional.saturating_add(1);
            }
            if positional >= parameters.len() {
                let Some(last) = parameters.last() else {
                    return false;
                };
                if !last.variadic {
                    return false;
                }
                parameters.len().saturating_sub(1)
            } else {
                let selected = positional;
                if !parameters[selected].variadic {
                    positional = positional.saturating_add(1);
                }
                selected
            }
        };
        if assigned[parameter] && !parameters[parameter].variadic {
            return false;
        }
        if let Some(argument) = argument_types.get(index).and_then(Option::as_deref)
            && !kotlin_types_compatible(&parameters[parameter].kind, argument)
        {
            return false;
        }
        assigned[parameter] = true;
    }
    parameters
        .iter()
        .zip(assigned)
        .all(|(parameter, assigned)| assigned || parameter.defaulted || parameter.variadic)
}

fn kotlin_types_compatible(expected: &str, actual: &str) -> bool {
    let expected = canonical_type(expected);
    let actual = canonical_type(actual);
    expected == actual
        || expected
            .strip_prefix("kotlin.")
            .is_some_and(|expected| actual.rsplit('.').next() == Some(expected))
        || actual
            .strip_prefix("kotlin.")
            .is_some_and(|actual| expected.rsplit('.').next() == Some(actual))
}

fn canonical_type(value: &str) -> String {
    let mut depth = 0_u32;
    value
        .chars()
        .filter(|character| match character {
            '<' => {
                depth = depth.saturating_add(1);
                false
            }
            '>' => {
                depth = depth.saturating_sub(1);
                false
            }
            '?' if depth == 0 => false,
            _ => depth == 0 && !character.is_whitespace(),
        })
        .collect()
}

fn imported_callable_name(target: &str) -> String {
    target.rsplit_once('.').map_or_else(
        || target.to_owned(),
        |(owner, name)| format!("{owner}::{name}"),
    )
}

#[cfg(test)]
mod tests {
    use super::{kotlin_arguments_apply, parse_argument_names, parse_callable_shape};

    #[test]
    fn named_default_and_variadic_arguments_fail_closed() {
        let shape = parse_callable_shape("render(receiver=String;prefix:String=,ids:Long?...)")
            .expect("valid test signature");
        assert_eq!(shape.receiver.as_deref(), Some("String"));
        let names = parse_argument_names("kotlin_args:ids").expect("valid context");
        assert!(kotlin_arguments_apply(
            &shape.parameters,
            &names,
            &[Some("Long".to_owned())]
        ));
        let unknown = parse_argument_names("kotlin_args:missing").expect("valid context");
        assert!(!kotlin_arguments_apply(
            &shape.parameters,
            &unknown,
            &[Some("Long".to_owned())]
        ));
    }
}
