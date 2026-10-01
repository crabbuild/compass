//! Source-backed Python receiver bindings. Project-wide member selection stays
//! in compass-resolve; this producer records only the nominal root type.

use super::*;

impl DirectEvidenceState<'_> {
    pub(super) fn index_python_mutated_receiver_members(&mut self, root: Node<'_>) {
        let mut writes = BTreeMap::<(String, String), (usize, bool)>::new();
        let mut pending = vec![(root, String::new(), String::new())];
        while let Some((node, mut class, mut method)) = pending.pop() {
            if node.kind() == "class_definition" {
                class = self
                    .declarations
                    .get(&node.id())
                    .map(|context| context.qualified_name.clone())
                    .unwrap_or_default();
                method.clear();
            } else if node.kind() == "function_definition" {
                method = node
                    .child_by_field_name("name")
                    .map(|name| self.text(name))
                    .unwrap_or_default();
            }
            if matches!(node.kind(), "assignment" | "augmented_assignment")
                && !class.is_empty()
                && let Some(lhs) = node
                    .child_by_field_name("left")
                    .filter(|node| node.kind() == "attribute")
                && let (Some(object), Some(member)) = (
                    lhs.child_by_field_name("object"),
                    lhs.child_by_field_name("attribute"),
                )
                && self.text(object) == "self"
            {
                let entry = writes
                    .entry((class.clone(), self.text(member)))
                    .or_default();
                entry.0 += 1;
                entry.1 |= method != "__init__"
                    || node.kind() != "assignment"
                    || !python_direct_function_assignment(node);
            }
            let mut cursor = node.walk();
            pending.extend(
                node.named_children(&mut cursor)
                    .map(|child| (child, class.clone(), method.clone())),
            );
        }
        self.python_mutated_receiver_members.extend(
            writes
                .into_iter()
                .filter(|(_, (count, unsafe_write))| *count > 1 || *unsafe_write)
                .map(|(key, _)| key),
        );
    }

    pub(super) fn collect_python_receiver_members(
        &mut self,
        node: Node<'_>,
        owner: &DeclarationContext,
        class: Option<&DeclarationContext>,
    ) -> Result<(), EvidenceError> {
        let active = self
            .declarations
            .get(&node.id())
            .cloned()
            .unwrap_or_else(|| owner.clone());
        let class = if node.kind() == "class_definition" {
            Some(&active)
        } else {
            class
        };
        if node.kind() == "assignment" && active.name == "__init__" {
            let lhs = node.child_by_field_name("left");
            let direct = python_direct_function_assignment(node);
            if direct
                && let (Some(lhs), Some(class)) =
                    (lhs.filter(|lhs| lhs.kind() == "attribute"), class)
                && let (Some(object), Some(member)) = (
                    lhs.child_by_field_name("object"),
                    lhs.child_by_field_name("attribute"),
                )
                && self.text(object) == "self"
                && !self
                    .python_mutated_receiver_members
                    .contains(&(class.qualified_name.clone(), self.text(member)))
                && self
                    .python_bound_method_receiver(node, "self", node.start_byte())?
                    .as_deref()
                    == Some(class.qualified_name.as_str())
            {
                let nominal = if let Some(annotation) = node.child_by_field_name("type") {
                    self.python_canonical_annotation(&active, annotation)
                        .and_then(|annotation| match annotation.runtime_targets.as_slice() {
                            [target] => Some(target.clone()),
                            _ => None,
                        })
                } else if let Some(rhs) = node.child_by_field_name("right") {
                    self.python_nominal_value(&active, rhs, node)
                } else {
                    None
                };
                if let Some(nominal) = nominal {
                    self.builder.bind(
                        BindingKind::Member,
                        &self.text(member),
                        &nominal,
                        None,
                        Some(&class.scope_id),
                        range_for_node(self.source_file, node),
                    )?;
                }
            }
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.collect_python_receiver_members(child, &active, class)?;
        }
        Ok(())
    }

    /// Connect a chained invocation to the source callable's return contract.
    /// Unknown callable/return types remain deferred in project resolution.
    pub(super) fn python_chained_receiver_binding(
        &mut self,
        owner: &DeclarationContext,
        function: Node<'_>,
        call: Node<'_>,
        depth: usize,
    ) -> Result<Option<String>, EvidenceError> {
        if depth >= 16 || function.kind() != "attribute" {
            return Ok(None);
        }
        let Some(receiver) = function
            .child_by_field_name("object")
            .filter(|node| node.kind() == "call")
        else {
            return Ok(None);
        };
        self.python_call_result_binding(owner, receiver, call, depth + 1)
    }

    fn python_call_result_binding(
        &mut self,
        owner: &DeclarationContext,
        receiver: Node<'_>,
        site: Node<'_>,
        depth: usize,
    ) -> Result<Option<String>, EvidenceError> {
        if depth >= 16 {
            return Ok(None);
        }
        let Some(called) = receiver.child_by_field_name("function") else {
            return Ok(None);
        };
        let raw = self.text(called);
        let (qualifier, spelling) = split_qualified(&raw);
        let (target, receiver_binding) = if let Some(qualifier) = qualifier {
            if let Some(binding) =
                self.python_chained_receiver_binding(owner, called, site, depth + 1)?
            {
                (spelling.to_owned(), Some(binding))
            } else if let Some(binding) =
                self.python_typed_receiver_binding(owner, called, site, qualifier)?
            {
                // Preserve the field path; the resolver follows nominal member
                // bindings before selecting the factory method's return type.
                let suffix = qualifier
                    .split_once('.')
                    .map(|(_, tail)| format!("{tail}.{spelling}"))
                    .unwrap_or_else(|| spelling.to_owned());
                (suffix, Some(binding))
            } else if let Some(target) =
                self.imported_qualified_target_for(owner, qualifier, called.start_byte(), true)
            {
                (format!("{target}.{spelling}"), None)
            } else {
                return Ok(None);
            }
        } else {
            if self.python_name_is_statically_local(owner, spelling) {
                return Ok(None);
            }
            let target = self
                .python_unique_visible_declaration(
                    owner,
                    spelling,
                    called.start_byte(),
                    &["class", "function"],
                )
                .map(|declaration| declaration.qualified_name)
                .or_else(|| {
                    self.imported_target_for_occurrence(owner, spelling, called.start_byte(), true)
                        .cloned()
                });
            let Some(target) = target else {
                return Ok(None);
            };
            (target, None)
        };
        Ok(Some(self.builder.bind_chained_call_result(
            &raw,
            &target,
            None,
            receiver_binding.as_deref(),
            None,
            Some(&owner.scope_id),
            range_for_node(self.source_file, called),
        )?))
    }

    fn python_nominal_value(
        &self,
        owner: &DeclarationContext,
        value: Node<'_>,
        site: Node<'_>,
    ) -> Option<String> {
        if value.kind() == "call" {
            let function = value.child_by_field_name("function")?;
            return self.python_nominal_annotation(
                owner,
                &self.text(function),
                function.start_byte(),
            );
        }
        if value.kind() != "identifier" {
            return None;
        }
        let name = self.text(value);
        let mut ancestor = Some(site);
        while let Some(node) = ancestor {
            if node.kind() == "function_definition" {
                let parameter = python_parameter_nodes(node)
                    .into_iter()
                    .find(|parameter| self.text(parameter.name) == name)?;
                let body = node.child_by_field_name("body").unwrap_or(node);
                if self.python_name_rebound_between(
                    body,
                    parameter.name.end_byte(),
                    value.start_byte(),
                    &name,
                ) {
                    return None;
                }
                let annotation = self.python_canonical_annotation(owner, parameter.annotation?)?;
                return match annotation.runtime_targets.as_slice() {
                    [target] => Some(target.clone()),
                    _ => None,
                };
            }
            ancestor = node.parent();
        }
        None
    }

    pub(super) fn python_typed_receiver_binding(
        &mut self,
        owner: &DeclarationContext,
        function: Node<'_>,
        call: Node<'_>,
        qualifier: &str,
    ) -> Result<Option<String>, EvidenceError> {
        if !qualifier.split('.').all(valid_python_identifier) {
            return Ok(None);
        }
        let root = qualified_binding_head(qualifier);
        let mut nominal = if matches!(root, "self" | "cls") {
            self.python_bound_method_receiver(call, root, function.start_byte())?
        } else {
            None
        };
        let mut ancestor = Some(call);
        while nominal.is_none() {
            let Some(node) = ancestor else { break };
            if node.kind() == "function_definition" {
                let body = node.child_by_field_name("body").unwrap_or(node);
                if let Some(parameter) = python_parameter_nodes(node)
                    .into_iter()
                    .find(|parameter| self.text(parameter.name) == root)
                {
                    if !self.python_name_rebound_between(
                        body,
                        parameter.name.end_byte(),
                        function.start_byte(),
                        root,
                    ) {
                        nominal = parameter
                            .annotation
                            .and_then(|annotation| {
                                self.python_canonical_annotation(owner, annotation)
                            })
                            .and_then(|annotation| match annotation.runtime_targets.as_slice() {
                                [target] => Some(target.clone()),
                                _ => None,
                            });
                    }
                    break;
                }
                if crate::engine::python_bound_names(node, self.source, false).contains(root) {
                    // Only a single unconditional local declaration can
                    // establish a nominal type; branching/rebinding stays unknown.
                    let mut cursor = body.walk();
                    let mut assignments = body.named_children(&mut cursor).filter(|statement| {
                        statement.start_byte() < function.start_byte()
                            && crate::engine::python_bound_names(*statement, self.source, true)
                                .contains(root)
                    });
                    if let Some(assignment) = assignments.next()
                        && assignments.next().is_none()
                        && assignment.kind() == "assignment"
                        && let Some(annotation) = assignment.child_by_field_name("type")
                    {
                        nominal = self
                            .python_canonical_annotation(owner, annotation)
                            .and_then(|annotation| match annotation.runtime_targets.as_slice() {
                                [target] => Some(target.clone()),
                                _ => None,
                            });
                    }
                    if nominal.is_none() {
                        nominal = self.python_local_initializer_receiver(
                            owner,
                            root,
                            function.start_byte(),
                            call,
                        )?;
                    }
                    break;
                }
            }
            if node.kind() == "class_definition" {
                break;
            }
            ancestor = node.parent();
        }
        let Some(nominal) = nominal else {
            return Ok(None);
        };
        let binding = self.builder.bind(
            BindingKind::LocalAlias,
            root,
            &nominal,
            None,
            Some(&owner.scope_id),
            range_for_node(self.source_file, function),
        )?;
        Ok(Some(binding))
    }
}

fn python_direct_function_assignment(node: Node<'_>) -> bool {
    node.parent()
        .and_then(|parent| {
            if parent.kind() == "expression_statement" {
                parent.parent()
            } else {
                Some(parent)
            }
        })
        .filter(|body| body.kind() == "block")
        .and_then(|body| body.parent())
        .is_some_and(|function| function.kind() == "function_definition")
}
