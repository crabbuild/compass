//! Definition-bound type expressions and exact generic parameter substitutions.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::*;

#[derive(Clone)]
pub(super) struct TypeOrigin {
    pub(super) spelling: String,
    pub(super) scope_id: String,
    substitutions: Option<Arc<TypeSubstitutions>>,
}

struct TypeSubstitutions {
    parameters: BTreeMap<String, TypeOrigin>,
    parent: Option<Arc<Self>>,
}

#[derive(Clone)]
pub(super) struct BaseTypeOrigins {
    pub(super) declaration_id: String,
    pub(super) arguments: Vec<TypeOrigin>,
}

impl TypeOrigin {
    pub(super) fn new(scope_id: &str, spelling: &str) -> Self {
        Self {
            spelling: spelling.to_owned(),
            scope_id: scope_id.to_owned(),
            substitutions: None,
        }
    }

    pub(super) fn with_spelling(&self, spelling: &str) -> Self {
        Self {
            spelling: spelling.to_owned(),
            scope_id: self.scope_id.clone(),
            substitutions: self.substitutions.clone(),
        }
    }

    pub(super) fn context_identity(&self) -> usize {
        self.substitutions
            .as_ref()
            .map_or(0, |frame| Arc::as_ptr(frame) as usize)
    }

    pub(super) fn substituted_parameter(&self, parameter_id: &str) -> Result<Option<Self>, ()> {
        let mut current = self.substitutions.as_ref();
        for _ in 0..=MAX_TYPE_SHAPE_DEPTH {
            let Some(frame) = current else {
                return Ok(None);
            };
            if let Some(origin) = frame.parameters.get(parameter_id) {
                return Ok(Some(origin.clone()));
            }
            current = frame.parent.as_ref();
        }
        Err(())
    }
}

impl CandidateState<'_, '_> {
    pub(super) fn type_argument_evidence_list(
        &self,
        origins: &[TypeOrigin],
        depth: usize,
        remaining: &mut usize,
        remaining_bytes: &mut usize,
    ) -> Option<Vec<TypeScriptTypeArgument>> {
        if origins.is_empty() {
            return Some(Vec::new());
        }
        if origins.len() > MAX_TYPESCRIPT_TYPE_ARGUMENT_ARITY
            || depth > MAX_TYPESCRIPT_TYPE_ARGUMENT_DEPTH
            || origins.len() > *remaining
        {
            self.type_origin_work_exhausted.set(true);
            return None;
        }
        *remaining -= origins.len();
        let mut arguments = Vec::with_capacity(origins.len());
        for origin in origins {
            if !self.charge_type_origin_work() {
                return None;
            }
            let Some(receiver) = self.resolve_type_origin_receiver(origin.clone()) else {
                arguments.push(TypeScriptTypeArgument::Unknown);
                continue;
            };
            let nested = self.type_argument_evidence_list(
                receiver.type_origins.as_deref().unwrap_or(&[]),
                depth + 1,
                remaining,
                remaining_bytes,
            )?;
            if let Some(import) = receiver.import {
                let mut original = None;
                for bindings in self.import_bindings.values() {
                    for binding in bindings.values() {
                        if !self.charge_type_origin_work() {
                            return None;
                        }
                        if binding.binding_id == import.binding_id {
                            original = Some(binding);
                        }
                    }
                }
                let Some(original) = original else {
                    arguments.push(TypeScriptTypeArgument::Unknown);
                    continue;
                };
                let target = import_target_without_namespace(&original.target);
                self.charge_type_argument_text(&import.binding_id, remaining_bytes)?;
                let member_path = if import.target == original.target {
                    Vec::new()
                } else if let Some(suffix) = import.target.strip_prefix(&format!("{target}::")) {
                    if suffix.split("::").count() > MAX_TYPESCRIPT_TYPE_ARGUMENT_DEPTH {
                        self.type_origin_work_exhausted.set(true);
                        return None;
                    }
                    for part in suffix.split("::") {
                        self.charge_type_argument_text(part, remaining_bytes)?;
                    }
                    suffix.split("::").map(ToOwned::to_owned).collect()
                } else {
                    arguments.push(TypeScriptTypeArgument::Unknown);
                    continue;
                };
                arguments.push(TypeScriptTypeArgument::Import {
                    binding_id: import.binding_id,
                    member_path,
                    arguments: nested,
                });
            } else if let Some(owner) = receiver
                .scope_id
                .as_deref()
                .and_then(|scope| self.scope_owners.get(scope))
                .and_then(|id| self.declarations.get(id))
                .filter(|owner| {
                    owner.qualified_name == receiver.qualified_name
                        && matches!(
                            owner.kind.as_str(),
                            "class" | "interface" | "enum" | "type_alias"
                        )
                })
            {
                self.charge_type_argument_text(&owner.id, remaining_bytes)?;
                arguments.push(TypeScriptTypeArgument::Source {
                    declaration_id: owner.id.clone(),
                    arguments: nested,
                });
            } else {
                arguments.push(TypeScriptTypeArgument::Unknown);
            }
        }
        Some(arguments)
    }

    pub(super) fn charge_type_argument_text(
        &self,
        text: &str,
        remaining: &mut usize,
    ) -> Option<()> {
        if text.len() > MAX_TYPESCRIPT_TYPE_ARGUMENT_REFERENCE_BYTES || text.len() > *remaining {
            self.type_origin_work_exhausted.set(true);
            return None;
        }
        *remaining -= text.len();
        Some(())
    }

    /// A literal key argument is not a nominal receiver, but its parameter
    /// substitution still has an exact definition context.
    pub(super) fn substituted_type_argument_origin(
        &self,
        mut origin: TypeOrigin,
    ) -> Option<TypeOrigin> {
        for _ in 0..=MAX_TYPE_SHAPE_DEPTH {
            if !self.charge_type_origin_work() {
                return None;
            }
            let Some(Resolution::Local(parameter)) =
                self.resolve_name(&origin.scope_id, &origin.spelling, Namespace::Type)
            else {
                return Some(origin);
            };
            if parameter.kind != "type_parameter" {
                return Some(origin);
            }
            let Some(argument) = origin.substituted_parameter(&parameter.id).ok()? else {
                return Some(origin);
            };
            origin = argument;
        }
        None
    }

    /// Build a persistent substitution frame keyed by exact parameter identity.
    /// Values keep the argument's original frame rather than inheriting the
    /// parameter definition context. Shared frames avoid expanding an alias
    /// tree when an argument occurs repeatedly.
    pub(super) fn instantiated_type_origin(
        &self,
        declaration: &DeclarationInfo,
        spelling: &str,
        arguments: &[TypeOrigin],
        parent: &TypeOrigin,
    ) -> Option<TypeOrigin> {
        let scope = self.scope_for_owner_declaration(&declaration.id)?;
        let mut origin = TypeOrigin {
            spelling: spelling.to_owned(),
            scope_id: scope.clone(),
            substitutions: parent.substitutions.clone(),
        };
        let Some(order) = self
            .generic_parameter_order_by_declaration
            .get(&declaration.id)
        else {
            return arguments.is_empty().then_some(origin);
        };
        if arguments.len() > order.len() || order.len() > MAX_INLINE_OBJECT_PROPERTIES {
            return None;
        }
        let mut parameters = BTreeMap::new();
        for (name, argument) in order.iter().zip(arguments) {
            if !self.charge_type_origin_work() {
                return None;
            }
            let Resolution::Local(parameter) = self.resolve_name(&scope, name, Namespace::Type)?
            else {
                return None;
            };
            if parameter.kind != "type_parameter" || parameter.scope_id != scope {
                return None;
            }
            parameters.insert(parameter.id, argument.clone());
        }
        origin.substitutions = Some(Arc::new(TypeSubstitutions {
            parameters,
            parent: parent.substitutions.clone(),
        }));
        Some(origin)
    }

    pub(super) fn member_type_origin(
        &self,
        receiver: &ReceiverTarget,
        declaration: &DeclarationInfo,
        receiver_is_member: bool,
    ) -> Option<TypeOrigin> {
        let spelling = declaration.declared_type_name.as_deref()?;
        let origin = TypeOrigin::new(&declaration.scope_id, spelling);
        let Some(owner_id) = self.scope_owners.get(&declaration.scope_id) else {
            return Some(origin);
        };
        let Some(owner) = self.declarations.get(owner_id) else {
            return Some(origin);
        };
        if !matches!(owner.kind.as_str(), "class" | "interface" | "type_alias") {
            return Some(origin);
        }
        let Some(order) = self.generic_parameter_order_by_declaration.get(owner_id) else {
            return Some(origin);
        };
        if order.is_empty() {
            return Some(origin);
        }
        let receiver_name = if receiver_is_member {
            receiver.qualified_name.rsplit_once('.')?.0
        } else {
            receiver.qualified_name.as_str()
        };
        if receiver_name == owner.qualified_name {
            let owner_scope = self.scope_for_owner_declaration(owner_id)?;
            if receiver.scope_id.as_deref() != Some(owner_scope.as_str()) {
                return None;
            }
            let arguments = receiver.type_origins.as_deref().unwrap_or(&[]);
            return self.instantiated_type_origin(owner, spelling, arguments, &origin);
        }
        let inherited = self.base_type_bindings.get(receiver_name)?;
        if inherited.declaration_id != owner.id {
            return None;
        }
        // Derived-class arguments belong to the extends clause, not to the
        // base field. The actual derived receiver can instantiate those
        // arguments before they are installed for the base's parameters.
        let ids = self.declarations_by_qualified.get(receiver_name)?;
        let [derived_id] = ids.as_slice() else {
            return None;
        };
        let derived = self.declarations.get(derived_id)?;
        let derived_origin = self.instantiated_type_origin(
            derived,
            "",
            receiver.type_origins.as_deref().unwrap_or(&[]),
            &origin,
        )?;
        let arguments = inherited
            .arguments
            .iter()
            .map(|argument| TypeOrigin {
                substitutions: derived_origin.substitutions.clone(),
                ..argument.clone()
            })
            .collect::<Vec<_>>();
        self.instantiated_type_origin(owner, spelling, &arguments, &origin)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_type_origin_work_exhaustion_returns_no_partial_batch()
    -> Result<(), Box<dyn std::error::Error>> {
        let source = b"class Table { lookup() {} } class Owner { table: Table; read() { this.table.lookup(); } }";
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&tree_sitter_language_pack::get_language("typescript")?)?;
        let tree = parser.parse(source, None).ok_or("missing parsed tree")?;
        let path = Path::new("src/main.ts");
        let result = emit_tree_evidence_with_type_origin_budget(
            path,
            "src/main.ts",
            source,
            tree.root_node(),
            "typescript",
            0,
        );
        assert!(matches!(
            result,
            Err(EvidenceError {
                code: EvidenceErrorCode::ResourceLimit,
                ..
            })
        ));
        let batch = emit_tree_evidence_with_type_origin_budget(
            path,
            "src/main.ts",
            source,
            tree.root_node(),
            "typescript",
            MAX_TYPE_ORIGIN_WORK,
        )?;
        assert!(
            batch
                .candidates
                .iter()
                .any(|candidate| candidate.target_spelling == "lookup"
                    && candidate.constraints.exact_target_declaration_id.is_some())
        );
        Ok(())
    }

    #[test]
    fn type_origin_budget_accepts_its_exact_work_boundary() -> Result<(), Box<dyn std::error::Error>>
    {
        let source = b"class Table { lookup() {} } type Selected<T> = T; class Owner { table: Selected<Table>; read() { this.table.lookup(); } }";
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&tree_sitter_language_pack::get_language("typescript")?)?;
        let tree = parser.parse(source, None).ok_or("missing parsed tree")?;
        let emit = |budget| {
            emit_tree_evidence_with_type_origin_budget(
                Path::new("src/main.ts"),
                "src/main.ts",
                source,
                tree.root_node(),
                "typescript",
                budget,
            )
        };
        let expected = serde_json::to_value(emit(MAX_TYPE_ORIGIN_WORK)?)?;
        let mut low = 0;
        let mut high = MAX_TYPE_ORIGIN_WORK;
        while high - low > 1 {
            let middle = low + (high - low) / 2;
            match emit(middle) {
                Ok(batch) => {
                    assert_eq!(serde_json::to_value(batch)?, expected);
                    high = middle;
                }
                Err(error) if error.code == EvidenceErrorCode::ResourceLimit => low = middle,
                Err(error) => return Err(Box::new(error)),
            }
        }
        assert!(matches!(
            emit(low),
            Err(EvidenceError {
                code: EvidenceErrorCode::ResourceLimit,
                ..
            })
        ));
        assert_eq!(serde_json::to_value(emit(high)?)?, expected);
        assert_eq!(serde_json::to_value(emit(high + 1)?)?, expected);
        Ok(())
    }
    #[test]
    fn producer_and_wire_accept_the_same_exact_depth_for_all_leaf_kinds()
    -> Result<(), Box<dyn std::error::Error>> {
        for leaf in ["Leaf", "Remote", "T", "Pair<Leaf, Remote>"] {
            let exact_depth = if leaf.starts_with("Pair") { 15 } else { 16 };
            for excess in [0, 1] {
                let mut argument = leaf.to_owned();
                for _ in 0..exact_depth + excess {
                    argument = format!("Wrap<{argument}>");
                }
                let source = format!(
                    "import type {{ Holder, Remote }} from './provider'; class Leaf {{}} class Wrap<V> {{}} class Pair<A, B> {{}} class Owner<T> {{ holder: Holder<{argument}>; read() {{ this.holder.field.lookup(); }} }}"
                );
                let mut parser = tree_sitter::Parser::new();
                parser.set_language(&tree_sitter_language_pack::get_language("typescript")?)?;
                let tree = parser
                    .parse(source.as_bytes(), None)
                    .ok_or("missing parsed tree")?;
                let result = emit_tree_evidence_with_type_origin_budget(
                    Path::new("main.ts"),
                    "main.ts",
                    source.as_bytes(),
                    tree.root_node(),
                    "typescript",
                    MAX_TYPE_ORIGIN_WORK,
                );

                if excess == 0 {
                    let batch = result?;
                    crate::validate_evidence(&batch, EvidenceLimits::default())?;
                    let proof = batch
                        .candidates
                        .iter()
                        .filter(|candidate| candidate.target_spelling == "lookup")
                        .find_map(|candidate| {
                            candidate.constraints.typescript_type_arguments.as_ref()
                        })
                        .ok_or("missing deep proof")?;
                    assert_eq!(proof.arguments.len(), 1);
                    let mut pending = proof
                        .arguments
                        .iter()
                        .map(|argument| (argument, 0))
                        .collect::<Vec<_>>();
                    let mut maximum = 0;
                    while let Some((argument, depth)) = pending.pop() {
                        maximum = maximum.max(depth);
                        match argument {
                            TypeScriptTypeArgument::Source { arguments, .. }
                            | TypeScriptTypeArgument::Import { arguments, .. } => {
                                pending
                                    .extend(arguments.iter().map(|argument| (argument, depth + 1)));
                            }
                            TypeScriptTypeArgument::Unknown => {}
                        }
                    }
                    assert_eq!(maximum, 16, "{leaf}");
                } else {
                    assert_eq!(
                        result
                            .err()
                            .ok_or("producer accepted excessive depth")?
                            .code,
                        EvidenceErrorCode::ResourceLimit,
                        "{leaf}"
                    );
                }
            }
        }
        Ok(())
    }
}
