//! Source export census, independent of successful target resolution.

use super::super::{ECMASCRIPT_EXPORT_INVENTORY_VERSION, EcmaScriptExportInventory};
use super::*;
use std::collections::BTreeMap;

type RetainedExports<'a> = BTreeMap<u64, Vec<&'a super::super::BindingFact>>;

impl CandidateState<'_, '_> {
    pub(super) fn export_inventory(
        &self,
        root: Node<'_>,
    ) -> Result<EcmaScriptExportInventory, EvidenceError> {
        let mut complete = !self.parser_recovered;
        let mut bindings = RetainedExports::new();
        for binding in self.builder.binding_facts() {
            self.inventory_work()?;
            if binding.kind == BindingKind::Reexport
                && binding.scope_id.as_deref() == Some(self.root_scope.as_str())
            {
                bindings
                    .entry(binding.range.start_byte)
                    .or_default()
                    .push(binding);
            }
        }
        let mut module = false;
        let mut nonempty = false;
        let mut cursor = root.walk();
        for statement in root.named_children(&mut cursor) {
            self.inventory_work()?;
            nonempty |= statement.kind() != "comment";
            module |= matches!(statement.kind(), "import_statement" | "export_statement");
            match statement.kind() {
                "export_statement" => {
                    complete &= self.export_statement_retained(statement, &bindings)?;
                }
                // A script, ambient external module, or recovered source is
                // not a closed ECMAScript source-module export surface.
                "ambient_declaration" | "ERROR" => complete = false,
                _ => {}
            }
        }
        complete &= module || !nonempty;
        Ok(EcmaScriptExportInventory {
            version: ECMASCRIPT_EXPORT_INVENTORY_VERSION,
            complete,
        })
    }

    fn export_statement_retained(
        &self,
        statement: Node<'_>,
        bindings: &RetainedExports<'_>,
    ) -> Result<bool, EvidenceError> {
        if statement.has_error() {
            return Ok(false);
        }
        // Inspect syntax tokens rather than source prefixes: comments may
        // occur between export and default/type/=.
        let mut cursor = statement.walk();
        let mut default = false;
        for child in statement.children(&mut cursor) {
            self.inventory_work()?;
            if child.kind() == "=" {
                // TypeScript export-assignment interop needs its own model.
                return Ok(false);
            }
            default |= child.kind() == "default";
        }
        if let Some(module) = first_named_child_kind(statement, "string") {
            let raw = self.export_text(module);
            if raw.len() < 2 || raw.len() > 4_098 || raw.contains(['\\', '\0']) {
                return Ok(false);
            }
            let bytes = raw.as_bytes();
            if !matches!(bytes[0], b'\'' | b'"') || bytes.last() != Some(&bytes[0]) {
                return Ok(false);
            }
            let literal = &raw[1..raw.len() - 1];
            // The ordinary emitter's legacy literal normalization cannot
            // certify a target if it changed an interior quote character.
            if literal.is_empty() || string_literal(self.source, module) != literal {
                return Ok(false);
            }
        }
        if default {
            return self.has_retained_export(bindings, statement, "default", false);
        }
        if let Some(clause) = first_named_child_kind(statement, "export_clause") {
            let mut cursor = clause.walk();
            for specifier in clause.named_children(&mut cursor) {
                self.inventory_work()?;
                if specifier.kind() == "comment" {
                    continue;
                }
                if specifier.kind() != "export_specifier" || specifier.named_child_count() > 2 {
                    return Ok(false);
                }
                let Some(imported) = specifier.child_by_field_name("name") else {
                    return Ok(false);
                };
                let exported = specifier.child_by_field_name("alias").unwrap_or(imported);
                for name in [imported, exported] {
                    self.inventory_work()?;
                    if !matches!(
                        name.kind(),
                        "identifier" | "type_identifier" | "property_identifier" | "default"
                    ) {
                        return Ok(false);
                    }
                }
                if !self.has_retained_export(
                    bindings,
                    exported,
                    self.export_text(exported),
                    true,
                )? {
                    return Ok(false);
                }
                // Module reexports must retain the imported name as well as
                // the alias. A same-spelled target in that module cannot
                // replace `default` or another renamed export.
                if first_named_child_kind(statement, "string").is_some() {
                    self.inventory_work()?;
                    let imported = self.export_text(imported);
                    let start = exported.start_byte() as u64;
                    let end = exported.end_byte() as u64;
                    let mut matched = false;
                    if let Some(values) = bindings.get(&start) {
                        for binding in values {
                            self.inventory_work()?;
                            matched |= binding.range.end_byte == end
                                && binding.spelling == self.export_text(exported)
                                && binding
                                    .qualified_target
                                    .rsplit_once("::")
                                    .is_some_and(|(_, name)| name == imported);
                        }
                    }
                    if !matched {
                        return Ok(false);
                    }
                }
            }
            return Ok(true);
        }
        if first_named_child_kind(statement, "string").is_some() {
            if let Some(namespace) = first_named_child_kind(statement, "namespace_export") {
                let Some(name) = first_identifier_node(namespace) else {
                    return Ok(false);
                };
                return self.has_retained_export(bindings, namespace, self.export_text(name), true);
            }
            return self.has_retained_export(bindings, statement, "*", true);
        }
        let Some(declaration) = first_named_child(statement) else {
            return Ok(false);
        };
        match declaration.kind() {
            "function_declaration"
            | "generator_function_declaration"
            | "class_declaration"
            | "abstract_class_declaration"
            | "interface_declaration"
            | "type_alias_declaration"
            | "enum_declaration" => {
                let Some(name) = declaration.child_by_field_name("name") else {
                    return Ok(false);
                };
                self.has_retained_export(bindings, name, self.export_text(name), true)
            }
            "lexical_declaration" | "variable_declaration" => {
                let mut cursor = declaration.walk();
                for declarator in declaration.named_children(&mut cursor) {
                    self.inventory_work()?;
                    if declarator.kind() == "comment" {
                        continue;
                    }
                    let Some(name) = declarator.child_by_field_name("name") else {
                        return Ok(false);
                    };
                    // Destructured export projection also needs key/default
                    // expression distinctions; it cannot certify absence yet.
                    if name.kind() != "identifier"
                        || !self.has_retained_export(
                            bindings,
                            name,
                            self.export_text(name),
                            true,
                        )?
                    {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn has_retained_export(
        &self,
        bindings: &RetainedExports<'_>,
        anchor: Node<'_>,
        name: &str,
        exact: bool,
    ) -> Result<bool, EvidenceError> {
        self.inventory_work()?;
        if name.is_empty() || name.len() > MAX_TYPE_SHAPE_BYTES || name.contains(['\\', '\0']) {
            return Ok(false);
        }
        let start = anchor.start_byte() as u64;
        let end = anchor.end_byte() as u64;
        for (_, values) in bindings.range(start..=end) {
            self.inventory_work()?;
            for binding in values {
                self.inventory_work()?;
                if binding.spelling == name
                    && binding.range.end_byte <= end
                    && (!exact
                        || (binding.range.start_byte == start && binding.range.end_byte == end))
                {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn export_text(&self, node: Node<'_>) -> &str {
        std::str::from_utf8(
            self.source
                .get(node.start_byte()..node.end_byte())
                .unwrap_or_default(),
        )
        .unwrap_or_default()
    }
}
