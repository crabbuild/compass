//! Bounded certificates for directly retained TypeScript nominal members.

use super::*;
use std::collections::BTreeMap;

impl CandidateState<'_, '_> {
    pub(super) fn inventory_work(&self) -> Result<(), EvidenceError> {
        if self.charge_type_origin_work() {
            Ok(())
        } else {
            Err(EvidenceError::new(
                EvidenceErrorCode::ResourceLimit,
                "TypeScript class inventory exceeded the shared type-origin work limit",
            ))
        }
    }

    pub(super) fn merge_inventory(
        &self,
        root: Node<'_>,
    ) -> Result<(Option<bool>, Vec<super::super::TypeScriptMergeDependency>), EvidenceError> {
        use super::super::{TypeScriptMergeDependencyKind as Kind, TypeScriptMergeDependencyUsage};
        let mut dependencies = Vec::new();
        if self.language != "typescript" {
            return Ok((None, dependencies));
        }
        if self.parser_recovered {
            return Ok((Some(false), dependencies));
        }
        let mut module = false;
        let mut nonempty = false;
        let mut cursor = root.walk();
        for child in root.named_children(&mut cursor) {
            self.inventory_work()?;
            module |= matches!(child.kind(), "import_statement" | "export_statement");
            nonempty |= child.kind() != "comment";
        }
        if nonempty && !module {
            return Ok((Some(false), dependencies));
        }
        let mut usage = TypeScriptMergeDependencyUsage::default();
        // Walk the original AST rather than the declaration projection. A
        // known load omitted by ordinary import extraction still matters to
        // the absence of declaration augmentation.
        let mut stack = vec![(root, 0_usize)];
        while let Some((node, depth)) = stack.pop() {
            self.inventory_work()?;
            if depth > MAX_TRAVERSAL_DEPTH {
                return Err(EvidenceError::new(
                    EvidenceErrorCode::ResourceLimit,
                    "TypeScript merge inventory exceeded the syntax depth limit",
                ));
            }
            if matches!(
                node.kind(),
                "ambient_declaration" | "internal_module" | "module"
            ) {
                return Ok((Some(false), dependencies));
            }
            // Escaped identifiers can hide require-like loads. They are not
            // interpreted by this bounded source profile.
            if matches!(node.kind(), "identifier" | "property_identifier")
                && node_text(self.source, node).contains('\\')
            {
                return Ok((Some(false), dependencies));
            }
            let module_node = match node.kind() {
                "import_statement" => {
                    let module = first_named_child_kind(node, "string").or_else(|| {
                        first_named_child_kind(node, "import_require_clause")
                            .and_then(|clause| first_named_child_kind(clause, "string"))
                    });
                    if module.is_none() {
                        return Ok((Some(false), dependencies));
                    }
                    module
                }
                "export_statement" => first_named_child_kind(node, "string"),
                "call_expression" => {
                    let function = node.child_by_field_name("function");
                    let load = function.is_some_and(|function| {
                        let name = node_text(self.source, function);
                        matches!(
                            name.as_str(),
                            "import" | "require" | "require.resolve" | "module.require"
                        )
                    });
                    if load {
                        let argument = node
                            .child_by_field_name("arguments")
                            .and_then(first_named_child);
                        if argument.is_none_or(|argument| argument.kind() != "string") {
                            return Ok((Some(false), dependencies));
                        }
                        argument
                    } else {
                        None
                    }
                }
                "comment" => {
                    let text = std::str::from_utf8(
                        self.source
                            .get(node.start_byte()..node.end_byte())
                            .unwrap_or_default(),
                    )
                    .unwrap_or_default();
                    if let Some(directive) = text.strip_prefix("///").map(str::trim_start)
                        && directive.starts_with("<reference")
                    {
                        // Bound XML parsing before allocating its tree. Other
                        // compiler reference profiles (e.g. lib) are unknown.
                        if directive.len() > 4_096 {
                            return Err(EvidenceError::new(
                                EvidenceErrorCode::ResourceLimit,
                                "TypeScript reference directive exceeds byte limit",
                            ));
                        }
                        if directive.contains('&') {
                            // Do not assume XML entity decoding agrees with
                            // TypeScript's compiler-directive interpretation.
                            return Ok((Some(false), dependencies));
                        }
                        let Ok(document) = roxmltree::Document::parse(directive) else {
                            return Ok((Some(false), dependencies));
                        };
                        let element = document.root_element();
                        let mut attributes = element.attributes();
                        let Some(attribute) = attributes.next() else {
                            return Ok((Some(false), dependencies));
                        };
                        let kind = match attribute.name() {
                            "path" => Kind::ReferencePath,
                            "types" => Kind::ReferenceTypes,
                            _ => return Ok((Some(false), dependencies)),
                        };
                        if element.tag_name().name() != "reference"
                            || attributes.next().is_some()
                            || element.children().next().is_some()
                        {
                            return Ok((Some(false), dependencies));
                        }
                        if !self.retain_merge_dependency(
                            kind,
                            attribute.value(),
                            node,
                            &mut usage,
                            &mut dependencies,
                        )? {
                            return Ok((Some(false), dependencies));
                        }
                    }
                    None
                }
                _ => None,
            };
            if let Some(module_node) = module_node {
                if module_node
                    .end_byte()
                    .saturating_sub(module_node.start_byte())
                    > super::super::MAX_TYPESCRIPT_MERGE_DEPENDENCY_SPECIFIER_BYTES + 2
                {
                    return Err(EvidenceError::new(
                        EvidenceErrorCode::ResourceLimit,
                        "TypeScript module specifier exceeds byte limit",
                    ));
                }
                let raw = self
                    .source
                    .get(module_node.start_byte()..module_node.end_byte())
                    .unwrap_or_default();
                let Some(delimiter) = raw
                    .first()
                    .copied()
                    .filter(|byte| matches!(byte, b'\'' | b'"'))
                else {
                    return Ok((Some(false), dependencies));
                };
                if raw.len() < 2 || raw.last().copied() != Some(delimiter) {
                    return Ok((Some(false), dependencies));
                }
                let Ok(specifier) = std::str::from_utf8(&raw[1..raw.len() - 1]) else {
                    return Ok((Some(false), dependencies));
                };
                if !self.retain_merge_dependency(
                    Kind::Module,
                    specifier,
                    module_node,
                    &mut usage,
                    &mut dependencies,
                )? {
                    return Ok((Some(false), dependencies));
                }
            }
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                self.inventory_work()?;
                stack.push((child, depth + 1));
            }
        }
        dependencies.sort_unstable_by(|left, right| {
            left.range
                .start_byte
                .cmp(&right.range.start_byte)
                .then_with(|| left.range.end_byte.cmp(&right.range.end_byte))
                .then_with(|| left.kind.cmp(&right.kind))
                .then_with(|| left.specifier.cmp(&right.specifier))
        });
        Ok((Some(true), dependencies))
    }

    fn retain_merge_dependency(
        &self,
        kind: super::super::TypeScriptMergeDependencyKind,
        specifier: &str,
        node: Node<'_>,
        usage: &mut super::super::TypeScriptMergeDependencyUsage,
        dependencies: &mut Vec<super::super::TypeScriptMergeDependency>,
    ) -> Result<bool, EvidenceError> {
        if specifier.is_empty() || specifier.contains(['\\', '\0']) {
            return Ok(false);
        }
        self.inventory_work()?;
        *usage = usage.retaining(specifier, self.source_file)?;
        dependencies.push(super::super::TypeScriptMergeDependency {
            kind,
            specifier: specifier.to_owned(),
            range: range_for_node(self.source_file, node),
        });
        Ok(true)
    }

    pub(super) fn nominal_inventories(
        &self,
        root: Node<'_>,
        preorder: &[(Node<'_>, usize, String)],
    ) -> Result<BTreeMap<String, (bool, bool)>, EvidenceError> {
        let mut result = BTreeMap::new();
        if self.language != "typescript" {
            return Ok(result);
        }
        // Script declarations can merge with other global source files.
        // This first certificate covers explicit ECMAScript modules only.
        let mut module = false;
        let mut cursor = root.walk();
        for child in root.named_children(&mut cursor) {
            self.inventory_work()?;
            module |= matches!(child.kind(), "import_statement" | "export_statement");
        }
        for (node, _, _) in preorder {
            self.inventory_work()?;
            if !matches!(
                node.kind(),
                "class_declaration"
                    | "abstract_class_declaration"
                    | "class"
                    | "interface_declaration"
            ) {
                continue;
            }
            let Some(scope) = self.scope_by_node.get(&node.id()) else {
                continue;
            };
            let Some(owner) = self
                .scope_owners
                .get(scope)
                .and_then(|id| self.declarations.get(id))
            else {
                continue;
            };
            if !matches!(owner.kind.as_str(), "class" | "interface") {
                continue;
            }
            let complete = module
                && !self.parser_recovered
                && self.member_scope_for_declaration(owner).as_deref() == Some(scope)
                && self.nominal_inventory_complete(*node, scope)?;
            // The accepted shape excludes heritage entirely. Inheritance
            // is checked separately by consumers; no missing base is empty.
            result.insert(owner.id.clone(), (complete, complete));
        }
        Ok(result)
    }

    fn nominal_inventory_complete(
        &self,
        node: Node<'_>,
        scope: &str,
    ) -> Result<bool, EvidenceError> {
        let mut ancestor = node.parent();
        for _ in 0..=MAX_TRAVERSAL_DEPTH {
            let Some(parent) = ancestor else { break };
            self.inventory_work()?;
            if matches!(
                parent.kind(),
                "ambient_declaration" | "internal_module" | "module"
            ) {
                return Ok(false);
            }
            if parent.kind() == "export_statement" {
                let mut cursor = parent.walk();
                for child in parent.named_children(&mut cursor) {
                    self.inventory_work()?;
                    if child.kind() == "decorator" {
                        return Ok(false);
                    }
                }
            }
            ancestor = parent.parent();
        }
        if ancestor.is_some() {
            return Ok(false);
        }
        let mut body = None;
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.inventory_work()?;
            match child.kind() {
                "identifier" | "type_identifier" | "type_parameters" | "comment" => {}
                "class_body" | "interface_body" => body = Some(child),
                _ => return Ok(false),
            }
        }
        let Some(body) = body else { return Ok(false) };
        let mut cursor = body.walk();
        for member in body.named_children(&mut cursor) {
            self.inventory_work()?;
            if member.kind() == "comment" {
                continue;
            }
            let Some((kind, name, _, _, _)) = declaration_shape(member) else {
                return Ok(false);
            };
            if !matches!(kind, "method" | "property")
                || !matches!(
                    name.kind(),
                    "property_identifier" | "identifier" | "private_property_identifier"
                )
                || name.end_byte().saturating_sub(name.start_byte())
                    > MAX_TYPESCRIPT_TYPE_ARGUMENT_REFERENCE_BYTES
                || node_text(self.source, name).contains('\\')
                || node_text(self.source, name) == "constructor"
            {
                return Ok(false);
            }
            let mut member_cursor = member.walk();
            for child in member.named_children(&mut member_cursor) {
                self.inventory_work()?;
                if child.kind() == "decorator" {
                    return Ok(false);
                }
            }
            // Certify the inventory actually retained by declaration
            // traversal, not just a syntactically supported source body.
            let spelling = declaration_name_text(self.source, name);
            let mut retained = 0_usize;
            if let Some(ids) = self
                .declarations_by_scope_name
                .get(scope)
                .and_then(|names| names.get(&spelling))
            {
                for id in ids {
                    self.inventory_work()?;
                    if self.declarations.get(id).is_some_and(|fact| {
                        fact.kind == kind && fact.range_start_byte == name.start_byte()
                    }) {
                        retained += 1;
                    }
                }
            }
            if retained != 1 {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inventory_work_has_an_inclusive_shared_budget() -> Result<(), Box<dyn std::error::Error>> {
        let source = b"import type { X } from './missing'; class Good { lookup() {} }";
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&tree_sitter_language_pack::get_language("typescript")?)?;
        let tree = parser.parse(source, None).ok_or("missing parsed tree")?;
        let run = |limit| {
            emit_tree_evidence_with_type_origin_budget(
                Path::new("main.ts"),
                "main.ts",
                source,
                tree.root_node(),
                "typescript",
                limit,
            )
        };
        let expected = run(MAX_TYPE_ORIGIN_WORK)?;
        let good = expected
            .declarations
            .iter()
            .find(|fact| fact.name == "Good" && fact.kind == "class")
            .ok_or("missing class")?;
        assert_eq!(good.member_inventory_complete, Some(true));
        assert!(good.direct_bases_complete);
        let mut low = 0;
        let mut high = MAX_TYPE_ORIGIN_WORK;
        while high - low > 1 {
            let middle = low + (high - low) / 2;
            match run(middle) {
                Ok(batch) => {
                    assert_eq!(batch, expected);
                    high = middle;
                }
                Err(error) if error.code == EvidenceErrorCode::ResourceLimit => low = middle,
                Err(error) => return Err(Box::new(error)),
            }
        }
        assert!(matches!(
            run(low),
            Err(EvidenceError {
                code: EvidenceErrorCode::ResourceLimit,
                ..
            })
        ));
        assert_eq!(run(high)?, expected);
        assert_eq!(run(high + 1)?, expected);
        Ok(())
    }
}
