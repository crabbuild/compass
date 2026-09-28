//! Java nominal receivers, overload applicability and conversion policy.

use super::super::*;

impl ResolutionDb<'_> {
    pub(in crate::evidence) fn resolve_java_field_receiver(
        &self,
        candidate: &RelationshipCandidate,
    ) -> Option<ResolutionDecision> {
        if candidate.relation != CandidateRelation::AccessesMember {
            return None;
        }
        let (owner, member) = candidate
            .constraints
            .qualified_name
            .as_deref()?
            .rsplit_once("::")?;
        let declarations = if owner.contains("::") {
            self.indexes
                .names
                .by_qualified
                .get(&("java".to_owned(), owner.to_owned()))
        } else {
            self.indexes.java.receivers_by_source_name.get(owner)
        };
        let Some(declarations) = declarations else {
            return Some(ResolutionDecision::Unresolved);
        };
        if declarations.len() > self.budget.candidates_per_lookup() {
            return Some(ResolutionDecision::Ambiguous {
                candidate_count: declarations.len(),
            });
        }
        // Member availability cannot choose among duplicate nominal types.
        // Count receiver declarations before looking for the requested field.
        let mut owners = declarations
            .iter()
            .filter_map(|slot| self.declaration(*slot))
            .filter(|declaration| {
                matches!(
                    declaration.kind.as_str(),
                    "class" | "interface" | "enum" | "record" | "annotation_type" | "enum_member"
                )
            });
        let Some(receiver) = owners.next() else {
            return Some(ResolutionDecision::Unresolved);
        };
        let count = 1 + owners.count();
        if count > 1 {
            return Some(ResolutionDecision::Ambiguous {
                candidate_count: count,
            });
        }
        // A duplicate enclosing type cannot be selected by the availability
        // of a nested type in only one declaration. Every nominal prefix must
        // be unique before using the canonical owner, within the lookup budget.
        let mut prefix = receiver.qualified_name.as_str();
        let mut remaining = self.budget.candidates_per_lookup() - declarations.len();
        while let Some((parent, _)) = prefix.rsplit_once("::") {
            if remaining == 0 {
                return Some(ResolutionDecision::Unresolved);
            }
            let Some(parents) = self
                .indexes
                .names
                .by_qualified
                .get(&("java".to_owned(), parent.to_owned()))
            else {
                return Some(ResolutionDecision::Unresolved);
            };
            if parents.len() > self.budget.candidates_per_lookup() {
                return Some(ResolutionDecision::Ambiguous {
                    candidate_count: parents.len(),
                });
            }
            if parents.len() > remaining {
                return Some(ResolutionDecision::Unresolved);
            }
            remaining -= parents.len();
            let count = parents
                .iter()
                .filter_map(|slot| self.declaration(*slot))
                .filter(|declaration| {
                    matches!(
                        declaration.kind.as_str(),
                        "class"
                            | "interface"
                            | "enum"
                            | "record"
                            | "annotation_type"
                            | "enum_member"
                    )
                })
                .count();
            if count != 1 {
                return Some(if count == 0 {
                    ResolutionDecision::Unresolved
                } else {
                    ResolutionDecision::Ambiguous {
                        candidate_count: count,
                    }
                });
            }
            prefix = parent;
        }
        // A constant-specific body can access its own fields. A value whose
        // declared type is the enum cannot name that anonymous subclass.
        // Require lexical ownership, not just a matching qualified prefix.
        if receiver.kind == "enum_member" {
            let mut scope = candidate.constraints.scope_id.as_deref();
            let mut owned = false;
            for _ in 0..self.budget.candidates_per_lookup() {
                let Some(current) = scope.and_then(|id| self.facts.scopes.get(id)) else {
                    return Some(ResolutionDecision::Unresolved);
                };
                if current.owner_declaration_id.as_deref() == Some(receiver.id.as_str()) {
                    owned = true;
                    break;
                }
                scope = current.parent_scope_id.as_deref();
            }
            if !owned {
                return Some(ResolutionDecision::Unresolved);
            }
        }
        if receiver.qualified_name == owner {
            return None;
        }
        // This is nominal name canonicalization, not expression-chain typing.
        // The producer must already have established the receiver's type.
        let qualified = format!("{}::{member}", receiver.qualified_name);
        Some(
            self.unique_decision(
                self.indexes
                    .names
                    .by_qualified
                    .get(&("java".to_owned(), qualified)),
                candidate,
                ResolutionRule::MemberBinding,
            )
            .unwrap_or(ResolutionDecision::Unresolved),
        )
    }

    pub(in crate::evidence) fn resolve_java_same_package_builtin_collision(
        &self,
        candidate: &RelationshipCandidate,
    ) -> Option<ResolutionDecision> {
        let module = candidate.constraints.module_or_package.as_deref()?;
        if module == "java.lang" {
            return None;
        }
        let builtin = candidate
            .constraints
            .qualified_name
            .as_deref()?
            .strip_prefix("java.lang.")?;
        let builtin_type = builtin.split("::").next()?;
        if !compass_languages::is_language_builtin_global("java", builtin_type) {
            return None;
        }
        let same_package = format!("{module}.{builtin}");
        self.unique_decision(
            self.indexes
                .names
                .by_qualified
                .get(&(candidate.language.clone(), same_package)),
            candidate,
            ResolutionRule::UniqueModuleOrPackage,
        )
    }

    pub(in crate::evidence) fn unique_java_applicable_overload<'a>(
        &self,
        overloads: &[&'a DeclarationFact],
        argument_types: &[Option<String>],
    ) -> Option<&'a str> {
        // JLS 15.12.2: strict fixed arity, loose fixed arity, then variable
        // arity. A spread declaration participates in fixed phases as an array.
        for phase in [
            JavaInvocationPhase::Strict,
            JavaInvocationPhase::Loose,
            JavaInvocationPhase::Variable,
        ] {
            let mut proven = Vec::new();
            let mut unknown = false;
            for declaration in overloads {
                match self.java_applicability(declaration, argument_types, phase) {
                    JavaApplicability::Proven => proven.push(*declaration),
                    JavaApplicability::Unknown => unknown = true,
                    JavaApplicability::Disproven => {}
                }
            }
            // Missing type/hierarchy evidence in an earlier phase can change
            // the selected overload. Never skip it to prefer a later phase.
            if unknown {
                return None;
            }
            if proven.is_empty() {
                continue;
            }
            if let [only] = proven.as_slice() {
                return Some(only.id.as_str());
            }
            let mut most_specific = proven.iter().copied().filter(|candidate| {
                proven.iter().copied().all(|other| {
                    candidate.id == other.id
                        || self.java_parameters_more_specific(candidate, other, phase)
                })
            });
            let only = most_specific.next()?;
            return most_specific.next().is_none().then_some(only.id.as_str());
        }
        None
    }

    fn java_applicability(
        &self,
        declaration: &DeclarationFact,
        arguments: &[Option<String>],
        phase: JavaInvocationPhase,
    ) -> JavaApplicability {
        let parameters = &declaration.parameter_types;
        if declaration.parameter_count != u32::try_from(parameters.len()).ok() {
            return JavaApplicability::Unknown;
        }
        if phase == JavaInvocationPhase::Variable {
            if !declaration.variadic
                || parameters.is_empty()
                || arguments.len() < parameters.len() - 1
            {
                return JavaApplicability::Disproven;
            }
            if parameters.last().is_none_or(|p| !p.ends_with("[]")) {
                return JavaApplicability::Unknown;
            }
        } else if parameters.len() != arguments.len() {
            return JavaApplicability::Disproven;
        }
        let mut applicability = JavaApplicability::Proven;
        for (index, argument) in arguments.iter().enumerate() {
            let Some(parameter) = java_invocation_parameter(declaration, index, phase) else {
                return JavaApplicability::Unknown;
            };
            let Some(argument) = argument.as_deref() else {
                applicability = JavaApplicability::Unknown;
                continue;
            };
            match self.java_phase_conversion(argument, parameter, phase) {
                JavaConversion::Proven => {}
                JavaConversion::Disproven => return JavaApplicability::Disproven,
                JavaConversion::Unknown => applicability = JavaApplicability::Unknown,
            }
        }
        applicability
    }

    fn java_parameters_more_specific(
        &self,
        candidate: &DeclarationFact,
        other: &DeclarationFact,
        phase: JavaInvocationPhase,
    ) -> bool {
        // Comparing unequal fixed prefixes needs additional JLS specificity
        // evidence. Retain ambiguity instead of selecting by declaration order.
        candidate.parameter_types.len() == other.parameter_types.len()
            && candidate.parameter_types != other.parameter_types
            && (0..candidate.parameter_types.len()).all(|index| {
                let Some(candidate) = java_invocation_parameter(candidate, index, phase) else {
                    return false;
                };
                let Some(other) = java_invocation_parameter(other, index, phase) else {
                    return false;
                };
                self.java_phase_conversion(candidate, other, JavaInvocationPhase::Strict)
                    == JavaConversion::Proven
            })
    }

    fn java_phase_conversion(
        &self,
        argument: &str,
        parameter: &str,
        phase: JavaInvocationPhase,
    ) -> JavaConversion {
        if phase == JavaInvocationPhase::Strict
            && java_primitive_type(argument) != java_primitive_type(parameter)
        {
            return JavaConversion::Disproven;
        }
        self.java_conversion(argument, parameter)
    }

    fn java_conversion(&self, argument: &str, parameter: &str) -> JavaConversion {
        if argument == parameter {
            return JavaConversion::Proven;
        }
        if argument == "null" {
            return if java_primitive_type(parameter) {
                JavaConversion::Disproven
            } else {
                JavaConversion::Proven
            };
        }
        if java_primitive_type(argument) {
            if java_primitive_type(parameter) {
                return if java_primitive_widens_to(argument, parameter) {
                    JavaConversion::Proven
                } else {
                    JavaConversion::Disproven
                };
            }
            let Some(boxed) = java_boxed_type(argument) else {
                return JavaConversion::Disproven;
            };
            return self.java_reference_conversion(boxed, parameter);
        }
        if java_primitive_type(parameter) {
            let Some(unboxed) = java_unboxed_type(argument) else {
                return JavaConversion::Disproven;
            };
            return if java_primitive_widens_to(unboxed, parameter) {
                JavaConversion::Proven
            } else {
                JavaConversion::Disproven
            };
        }
        self.java_reference_conversion(argument, parameter)
    }

    fn java_reference_conversion(&self, argument: &str, parameter: &str) -> JavaConversion {
        if argument == parameter || parameter == "java.lang.Object" {
            return JavaConversion::Proven;
        }
        if let Some(argument_component) = argument.strip_suffix("[]") {
            if let Some(parameter_component) = parameter.strip_suffix("[]") {
                return if java_primitive_type(argument_component)
                    || java_primitive_type(parameter_component)
                {
                    if argument_component == parameter_component {
                        JavaConversion::Proven
                    } else {
                        JavaConversion::Disproven
                    }
                } else {
                    self.java_reference_conversion(argument_component, parameter_component)
                };
            }
            return if matches!(parameter, "java.lang.Cloneable" | "java.io.Serializable") {
                JavaConversion::Proven
            } else {
                JavaConversion::Disproven
            };
        }
        if parameter.ends_with("[]") {
            return JavaConversion::Disproven;
        }

        let mut pending = vec![argument.to_owned()];
        let mut visited = BTreeSet::new();
        let mut complete = true;
        while let Some(current) = pending.pop() {
            if !visited.insert(current.clone()) {
                continue;
            }
            if visited.len() > self.budget.candidates_per_lookup() {
                return JavaConversion::Unknown;
            }
            for base in java_known_direct_bases(&current) {
                if *base == parameter {
                    return JavaConversion::Proven;
                }
                pending.push((*base).to_owned());
            }
            if java_known_direct_bases(&current).is_empty() && current != "java.lang.Object" {
                let Some(declaration) = self.exact_java_type_declaration(&current) else {
                    complete = false;
                    continue;
                };
                if !declaration.direct_bases_complete {
                    complete = false;
                    continue;
                }
                if let Some(bases) = self
                    .indexes
                    .hierarchy
                    .direct_bases
                    .get(&("java".to_owned(), current.clone()))
                {
                    if !bases.complete {
                        complete = false;
                        continue;
                    }
                    for link in &bases.links {
                        let Some(base) = link.qualified_name.as_ref() else {
                            complete = false;
                            continue;
                        };
                        if base == parameter {
                            return JavaConversion::Proven;
                        }
                        pending.push(base.clone());
                    }
                }
                let implicit = match declaration.kind.as_str() {
                    "enum" => "java.lang.Enum",
                    "record" => "java.lang.Record",
                    "class" | "interface" | "annotation_type" => "java.lang.Object",
                    _ => {
                        complete = false;
                        continue;
                    }
                };
                if implicit == parameter {
                    return JavaConversion::Proven;
                }
                pending.push(implicit.to_owned());
            }
        }
        if complete {
            JavaConversion::Disproven
        } else {
            JavaConversion::Unknown
        }
    }

    pub(in crate::evidence) fn exact_java_type_declaration(
        &self,
        qualified_name: &str,
    ) -> Option<&DeclarationFact> {
        let declarations = self
            .indexes
            .names
            .by_qualified
            .get(&("java".to_owned(), qualified_name.to_owned()))?;
        let mut eligible = declarations.iter().filter_map(|id| {
            self.declaration(*id).filter(|declaration| {
                matches!(
                    declaration.kind.as_str(),
                    "class" | "interface" | "enum" | "record" | "annotation_type"
                )
            })
        });
        let only = eligible.next()?;
        eligible.next().is_none().then_some(only)
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum JavaInvocationPhase {
    Strict,
    Loose,
    Variable,
}

fn java_invocation_parameter(
    declaration: &DeclarationFact,
    index: usize,
    phase: JavaInvocationPhase,
) -> Option<&str> {
    let parameters = &declaration.parameter_types;
    if phase == JavaInvocationPhase::Variable && index >= parameters.len().checked_sub(1)? {
        parameters.last()?.strip_suffix("[]")
    } else {
        parameters.get(index).map(String::as_str)
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum JavaApplicability {
    Proven,
    Disproven,
    Unknown,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum JavaConversion {
    Proven,
    Disproven,
    Unknown,
}

pub(in crate::evidence) fn java_primitive_type(kind: &str) -> bool {
    matches!(
        kind,
        "byte" | "short" | "int" | "long" | "float" | "double" | "boolean" | "char"
    )
}

pub(in crate::evidence) fn java_primitive_widens_to(argument: &str, parameter: &str) -> bool {
    argument == parameter
        || matches!(
            (argument, parameter),
            ("byte", "short" | "int" | "long" | "float" | "double")
                | ("short" | "char", "int" | "long" | "float" | "double")
                | ("int", "long" | "float" | "double")
                | ("long", "float" | "double")
                | ("float", "double")
        )
}

pub(in crate::evidence) fn java_boxed_type(primitive: &str) -> Option<&'static str> {
    match primitive {
        "byte" => Some("java.lang.Byte"),
        "short" => Some("java.lang.Short"),
        "int" => Some("java.lang.Integer"),
        "long" => Some("java.lang.Long"),
        "float" => Some("java.lang.Float"),
        "double" => Some("java.lang.Double"),
        "boolean" => Some("java.lang.Boolean"),
        "char" => Some("java.lang.Character"),
        _ => None,
    }
}

pub(in crate::evidence) fn java_unboxed_type(reference: &str) -> Option<&'static str> {
    match reference {
        "java.lang.Byte" => Some("byte"),
        "java.lang.Short" => Some("short"),
        "java.lang.Integer" => Some("int"),
        "java.lang.Long" => Some("long"),
        "java.lang.Float" => Some("float"),
        "java.lang.Double" => Some("double"),
        "java.lang.Boolean" => Some("boolean"),
        "java.lang.Character" => Some("char"),
        _ => None,
    }
}

pub(in crate::evidence) fn java_known_direct_bases(reference: &str) -> &'static [&'static str] {
    match reference {
        "java.lang.Byte" | "java.lang.Short" | "java.lang.Integer" | "java.lang.Long"
        | "java.lang.Float" | "java.lang.Double" => &["java.lang.Number"],
        "java.lang.Number" => &["java.lang.Object"],
        "java.lang.Boolean" | "java.lang.Character" | "java.lang.String" => &["java.lang.Object"],
        "java.lang.Class" => &["java.lang.Object", "java.lang.reflect.Type"],
        "java.lang.Enum" | "java.lang.Record" => &["java.lang.Object"],
        "java.lang.StringBuilder" | "java.lang.StringBuffer" => &[
            "java.lang.Object",
            "java.lang.Appendable",
            "java.lang.CharSequence",
        ],
        "java.lang.Object" => &[],
        _ => &[],
    }
}
