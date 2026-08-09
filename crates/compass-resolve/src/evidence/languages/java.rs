//! Java overload applicability and conversion resolution policy.

use super::super::*;

impl ResolutionDb<'_> {
    pub(in crate::evidence) fn unique_java_applicable_overload<'a>(
        &self,
        overloads: &[&'a DeclarationFact],
        argument_types: &[Option<String>],
    ) -> Option<&'a str> {
        let mut proven = Vec::new();
        for declaration in overloads {
            if declaration.parameter_types.len() != argument_types.len() {
                return None;
            }
            let mut applicability = JavaApplicability::Proven;
            for (parameter, argument) in declaration.parameter_types.iter().zip(argument_types) {
                let argument = argument.as_deref()?;
                match self.java_conversion(argument, parameter) {
                    JavaConversion::Proven => {}
                    JavaConversion::Disproven => {
                        applicability = JavaApplicability::Disproven;
                        break;
                    }
                    JavaConversion::Unknown => applicability = JavaApplicability::Unknown,
                }
            }
            match applicability {
                JavaApplicability::Proven => proven.push(*declaration),
                JavaApplicability::Unknown => return None,
                JavaApplicability::Disproven => {}
            }
        }
        if let [only] = proven.as_slice() {
            return Some(only.id.as_str());
        }
        let mut most_specific = proven.iter().copied().filter(|candidate| {
            proven.iter().copied().all(|other| {
                candidate.id == other.id
                    || self.java_parameter_vector_more_specific(candidate, other)
            })
        });
        let only = most_specific.next()?;
        most_specific.next().is_none().then_some(only.id.as_str())
    }

    pub(in crate::evidence) fn java_parameter_vector_more_specific(
        &self,
        candidate: &DeclarationFact,
        other: &DeclarationFact,
    ) -> bool {
        candidate.parameter_types.len() == other.parameter_types.len()
            && candidate
                .parameter_types
                .iter()
                .zip(&other.parameter_types)
                .all(|(candidate, other)| {
                    self.java_conversion(candidate, other) == JavaConversion::Proven
                })
            && candidate.parameter_types != other.parameter_types
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
