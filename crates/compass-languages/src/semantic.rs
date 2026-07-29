use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde_json::{Map, Value, json};
use tree_sitter::Node;

use crate::{Extraction, RawEdgeRecord, RawNodeRecord, file_stem, make_id};

pub(crate) fn enrich(
    path: &Path,
    source: &[u8],
    root: Node<'_>,
    language: &'static str,
    extraction: &mut Extraction,
) {
    let mut state = State::new(path, source, language, extraction);
    match language {
        "rust" => state.rust(root),
        "javascript" | "typescript" | "tsx" => state.typescript(root),
        "csharp" => state.csharp(root),
        _ => {}
    }
}

struct State<'source, 'extraction> {
    source: &'source [u8],
    source_file: String,
    stem: String,
    file_id: String,
    language: &'static str,
    extraction: &'extraction mut Extraction,
    types: HashMap<String, String>,
    callables: HashMap<String, String>,
    seen_edges: HashSet<(String, String, String, usize, usize)>,
}

impl<'source, 'extraction> State<'source, 'extraction> {
    fn new(
        path: &Path,
        source: &'source [u8],
        language: &'static str,
        extraction: &'extraction mut Extraction,
    ) -> Self {
        let source_file = path.to_string_lossy().into_owned();
        let stem = file_stem(path);
        let file_id = make_id(&[&source_file]);
        let seen_edges = extraction
            .edges
            .iter()
            .map(|edge| {
                (
                    edge.source.clone(),
                    edge.target.clone(),
                    edge.string("relation"),
                    edge.attributes
                        .get("start_byte")
                        .and_then(Value::as_u64)
                        .and_then(|value| usize::try_from(value).ok())
                        .unwrap_or_default(),
                    edge.attributes
                        .get("end_byte")
                        .and_then(Value::as_u64)
                        .and_then(|value| usize::try_from(value).ok())
                        .unwrap_or_default(),
                )
            })
            .collect();
        Self {
            source,
            source_file,
            stem,
            file_id,
            language,
            extraction,
            types: HashMap::new(),
            callables: HashMap::new(),
            seen_edges,
        }
    }

    fn rust(&mut self, root: Node<'_>) {
        let declarations = descendants(
            root,
            &["trait_item", "struct_item", "enum_item", "type_item"],
        );
        for declaration in &declarations {
            let Some(name) = node_name(*declaration, self.source) else {
                continue;
            };
            let kind = match declaration.kind() {
                "trait_item" => "trait",
                "struct_item" => "struct",
                "enum_item" => "enum",
                "type_item" => "type_alias",
                _ => continue,
            };
            let id = make_id(&[&self.stem, &name]);
            self.upsert_node(&id, &name, &name, kind, *declaration, None);
            self.types.insert(name, id.clone());
            self.edge(&self.file_id.clone(), &id, "contains", *declaration, None);
        }

        for declaration in declarations {
            let Some(name) = node_name(declaration, self.source) else {
                continue;
            };
            let Some(owner) = self.types.get(&name).cloned() else {
                continue;
            };
            match declaration.kind() {
                "struct_item" => self.rust_fields(declaration, &owner, &name),
                "enum_item" => self.rust_variants(declaration, &owner, &name),
                "type_item" => {
                    if let Some(target) = declaration
                        .child_by_field_name("type")
                        .and_then(|node| type_name(node, self.source))
                        .and_then(|target| self.types.get(&target).cloned())
                    {
                        self.edge(&owner, &target, "aliases", declaration, Some("type-alias"));
                    }
                }
                "trait_item" => self.rust_trait_methods(declaration, &owner, &name),
                _ => {}
            }
        }

        for item in descendants(root, &["const_item"]) {
            let Some(name) = node_name(item, self.source) else {
                continue;
            };
            let id = make_id(&[&self.stem, "constant", &name]);
            self.upsert_node(&id, &name, &name, "constant", item, None);
            self.edge(&self.file_id.clone(), &id, "contains", item, None);
            self.typed_edge(&id, item.child_by_field_name("type"), item);
        }
        for item in descendants(root, &["macro_definition"]) {
            let Some(name) = node_name(item, self.source) else {
                continue;
            };
            let id = make_id(&[&self.stem, "macro", &name]);
            self.upsert_node(&id, &name, &name, "macro", item, None);
            self.edge(&self.file_id.clone(), &id, "contains", item, None);
        }

        for function in descendants(root, &["function_item", "function_signature_item"]) {
            if ancestor(function, &["trait_item", "impl_item"]).is_some() {
                continue;
            }
            let Some(name) = node_name(function, self.source) else {
                continue;
            };
            let id = make_id(&[&self.stem, &name]);
            self.upsert_node(&id, &format!("{name}()"), &name, "function", function, None);
            self.callables.insert(name.clone(), id.clone());
            self.rust_callable_semantics(function, &id, &name);
            if has_rust_test_attribute(function, self.source) {
                self.mark_test(&id);
            }
        }

        for implementation in descendants(root, &["impl_item"]) {
            self.rust_impl(implementation);
        }
        self.rust_test_edges(root);
    }

    fn rust_fields(&mut self, declaration: Node<'_>, owner: &str, owner_name: &str) {
        for field in descendants(declaration, &["field_declaration"]) {
            let Some(name) = node_name(field, self.source) else {
                continue;
            };
            let id = make_id(&[owner, "field", &name]);
            let qualified = format!("{owner_name}::{name}");
            self.upsert_node(&id, &name, &qualified, "field", field, None);
            self.edge(owner, &id, "contains", field, None);
            self.typed_edge(&id, field.child_by_field_name("type"), field);
        }
    }

    fn rust_variants(&mut self, declaration: Node<'_>, owner: &str, owner_name: &str) {
        for variant in descendants(declaration, &["enum_variant"]) {
            let Some(name) = node_name(variant, self.source) else {
                continue;
            };
            let id = make_id(&[owner, "variant", &name]);
            let qualified = format!("{owner_name}::{name}");
            let extra = Map::from_iter([(
                "declaring_type".to_owned(),
                Value::String(owner_name.to_owned()),
            )]);
            self.upsert_node(&id, &name, &qualified, "enum_member", variant, Some(extra));
        }
    }

    fn rust_trait_methods(&mut self, declaration: Node<'_>, owner: &str, owner_name: &str) {
        for function in descendants(declaration, &["function_item", "function_signature_item"]) {
            if ancestor_between(function, declaration, "impl_item") {
                continue;
            }
            let Some(name) = node_name(function, self.source) else {
                continue;
            };
            let id = make_id(&[owner, owner_name, &name]);
            let qualified = format!("{owner_name}::{name}");
            self.upsert_node(
                &id,
                &format!(".{name}()"),
                &qualified,
                "method",
                function,
                None,
            );
            self.edge(owner, &id, "contains", function, None);
            self.callables.insert(qualified.clone(), id.clone());
            self.rust_callable_semantics(function, &id, &qualified);
        }
    }

    fn rust_impl(&mut self, implementation: Node<'_>) {
        let Some(implemented_type) = implementation
            .child_by_field_name("type")
            .and_then(|node| type_name(node, self.source))
        else {
            return;
        };
        let Some(owner) = self.types.get(&implemented_type).cloned() else {
            return;
        };
        let trait_name = implementation
            .child_by_field_name("trait")
            .and_then(|node| type_name(node, self.source));
        let semantic_owner = trait_name.as_ref().map_or_else(
            || implemented_type.clone(),
            |trait_name| format!("{implemented_type} as {trait_name}"),
        );
        for function in descendants(implementation, &["function_item"]) {
            if ancestor_between(function, implementation, "impl_item") {
                continue;
            }
            let Some(name) = node_name(function, self.source) else {
                continue;
            };
            let id = make_id(&[&owner, &semantic_owner, &name]);
            let qualified = format!("{semantic_owner}::{name}");
            self.upsert_node(
                &id,
                &format!(".{name}()"),
                &qualified,
                "method",
                function,
                None,
            );
            self.edge(&owner, &id, "contains", function, None);
            self.callables.insert(qualified.clone(), id.clone());
            self.rust_callable_semantics(function, &id, &qualified);
            if let Some(trait_name) = &trait_name
                && let Some(trait_id) = self.types.get(trait_name)
            {
                let target = make_id(&[trait_id, trait_name, &name]);
                if self.has_node(&target) {
                    self.edge(
                        &id,
                        &target,
                        "overrides",
                        function,
                        Some("trait-implementation"),
                    );
                }
            }
        }
    }

    fn rust_callable_semantics(&mut self, function: Node<'_>, id: &str, qualified: &str) {
        if let Some(parameters) = function.child_by_field_name("parameters") {
            for (position, parameter) in parameters
                .named_children(&mut parameters.walk())
                .enumerate()
            {
                if !matches!(parameter.kind(), "parameter" | "self_parameter") {
                    continue;
                }
                let name = parameter
                    .child_by_field_name("pattern")
                    .and_then(|node| type_name(node, self.source))
                    .or_else(|| node_name(parameter, self.source))
                    .unwrap_or_else(|| format!("parameter_{position}"));
                let parameter_id = make_id(&[id, "parameter", &position.to_string(), &name]);
                self.upsert_node(
                    &parameter_id,
                    &name,
                    &format!("{qualified}::{name}"),
                    "parameter",
                    parameter,
                    None,
                );
                self.edge(id, &parameter_id, "contains", parameter, None);
                self.typed_edge(
                    &parameter_id,
                    parameter.child_by_field_name("type"),
                    parameter,
                );
            }
        }
        if let Some(return_type) = function.child_by_field_name("return_type")
            && let Some(target) =
                type_name(return_type, self.source).and_then(|name| self.types.get(&name).cloned())
        {
            self.edge(id, &target, "returns", return_type, Some("return-type"));
        }
    }

    fn rust_test_edges(&mut self, root: Node<'_>) {
        for function in descendants(root, &["function_item"]) {
            let Some(name) = node_name(function, self.source) else {
                continue;
            };
            let id = make_id(&[&self.stem, &name]);
            if !self.is_test(&id) {
                continue;
            }
            let Some(body) = function.child_by_field_name("body") else {
                continue;
            };
            for call in descendants(body, &["call_expression"]) {
                let Some(callee) = call
                    .child_by_field_name("function")
                    .and_then(|node| type_name(node, self.source))
                else {
                    continue;
                };
                let target = make_id(&[&self.stem, &callee]);
                if target != id && self.has_node(&target) {
                    self.edge(&id, &target, "tests", call, Some("direct-test-call"));
                }
            }
        }
    }

    fn typescript(&mut self, root: Node<'_>) {
        for declaration in descendants(
            root,
            &[
                "class_declaration",
                "abstract_class_declaration",
                "interface_declaration",
                "enum_declaration",
                "type_alias_declaration",
            ],
        ) {
            let Some(name) = node_name(declaration, self.source) else {
                continue;
            };
            let kind = match declaration.kind() {
                "interface_declaration" => "interface",
                "enum_declaration" => "enum",
                "type_alias_declaration" => "type_alias",
                _ => "class",
            };
            let id = make_id(&[&self.stem, &name]);
            self.upsert_node(&id, &name, &name, kind, declaration, None);
            self.types.insert(name, id);
        }
        for function in descendants(root, &["function_declaration"]) {
            if ancestor(function, &["class_declaration", "interface_declaration"]).is_some() {
                continue;
            }
            let Some(name) = node_name(function, self.source) else {
                continue;
            };
            let id = make_id(&[&self.stem, &name]);
            self.callables.insert(name.clone(), id.clone());
            self.ts_callable(function, &id, &name);
        }
        for declaration in descendants(
            root,
            &[
                "class_declaration",
                "abstract_class_declaration",
                "interface_declaration",
            ],
        ) {
            self.ts_type_members(declaration);
        }
        self.ts_exports(root);
        self.ts_decorators(root);
        self.ts_overrides(root);
    }

    fn ts_type_members(&mut self, declaration: Node<'_>) {
        let Some(owner_name) = node_name(declaration, self.source) else {
            return;
        };
        let Some(owner) = self.types.get(&owner_name).cloned() else {
            return;
        };
        let Some(body) = declaration.child_by_field_name("body") else {
            return;
        };
        for member in body.named_children(&mut body.walk()) {
            if matches!(member.kind(), "method_definition" | "method_signature") {
                let Some(name) = node_name(member, self.source) else {
                    continue;
                };
                let kind = if name == "constructor" {
                    "constructor"
                } else {
                    "method"
                };
                let id = make_id(&[&owner, &name]);
                let qualified = format!("{owner_name}::{name}");
                self.upsert_node(&id, &format!(".{name}()"), &qualified, kind, member, None);
                self.edge(&owner, &id, "contains", member, None);
                self.callables.insert(qualified.clone(), id.clone());
                self.ts_callable(member, &id, &qualified);
            } else if matches!(
                member.kind(),
                "public_field_definition" | "property_signature" | "abstract_property_signature"
            ) {
                let Some(name) = node_name(member, self.source) else {
                    continue;
                };
                let id = make_id(&[&owner, "property", &name]);
                self.upsert_node(
                    &id,
                    &name,
                    &format!("{owner_name}::{name}"),
                    "property",
                    member,
                    None,
                );
                self.edge(&owner, &id, "contains", member, None);
                self.typed_edge(&id, member.child_by_field_name("type"), member);
            }
        }
    }

    fn ts_callable(&mut self, function: Node<'_>, id: &str, qualified: &str) {
        if let Some(parameters) = function.child_by_field_name("parameters") {
            for (position, parameter) in parameters
                .named_children(&mut parameters.walk())
                .enumerate()
            {
                let Some(name) = node_name(parameter, self.source) else {
                    continue;
                };
                let parameter_id = make_id(&[id, "parameter", &position.to_string(), &name]);
                self.upsert_node(
                    &parameter_id,
                    &name,
                    &format!("{qualified}::{name}"),
                    "parameter",
                    parameter,
                    None,
                );
                self.edge(id, &parameter_id, "contains", parameter, None);
                self.typed_edge(
                    &parameter_id,
                    parameter.child_by_field_name("type"),
                    parameter,
                );
            }
        }
        if let Some(return_type) = function.child_by_field_name("return_type")
            && let Some(target) =
                type_name(return_type, self.source).and_then(|name| self.types.get(&name).cloned())
        {
            self.edge(id, &target, "returns", return_type, Some("return-type"));
        }
    }

    fn ts_exports(&mut self, root: Node<'_>) {
        for statement in descendants(root, &["export_statement"]) {
            let text = self.text(statement);
            if let Some((source_name, exported_name)) = parse_export_alias(text) {
                let Some(target) = self
                    .types
                    .get(source_name)
                    .or_else(|| self.callables.get(source_name))
                    .cloned()
                else {
                    continue;
                };
                let id = make_id(&[
                    &self.stem,
                    "export",
                    exported_name,
                    &statement.start_byte().to_string(),
                ]);
                let mut extra = Map::new();
                extra.insert(
                    "specifier".to_owned(),
                    Value::String(exported_name.to_owned()),
                );
                extra.insert(
                    "imported_name".to_owned(),
                    Value::String(source_name.to_owned()),
                );
                extra.insert(
                    "local_name".to_owned(),
                    Value::String(exported_name.to_owned()),
                );
                self.upsert_node(
                    &id,
                    exported_name,
                    exported_name,
                    "export",
                    statement,
                    Some(extra),
                );
                self.edge(&self.file_id.clone(), &id, "contains", statement, None);
                self.edge(&id, &target, "exports", statement, Some("named-export"));
                self.edge(&id, &target, "aliases", statement, Some("export-alias"));
                continue;
            }
            let Some(declaration) = first_descendant_of(
                statement,
                &[
                    "class_declaration",
                    "abstract_class_declaration",
                    "interface_declaration",
                    "enum_declaration",
                    "type_alias_declaration",
                    "function_declaration",
                ],
            ) else {
                continue;
            };
            let Some(name) = node_name(declaration, self.source) else {
                continue;
            };
            let Some(target) = self
                .types
                .get(&name)
                .or_else(|| self.callables.get(&name))
                .cloned()
            else {
                continue;
            };
            let id = make_id(&[
                &self.stem,
                "export",
                &name,
                &statement.start_byte().to_string(),
            ]);
            let extra = Map::from_iter([("specifier".to_owned(), Value::String(name.clone()))]);
            self.upsert_node(&id, &name, &name, "export", statement, Some(extra));
            self.edge(&self.file_id.clone(), &id, "contains", statement, None);
            self.edge(
                &id,
                &target,
                "exports",
                statement,
                Some("declaration-export"),
            );
        }
    }

    fn ts_decorators(&mut self, root: Node<'_>) {
        for decorator in descendants(root, &["decorator"]) {
            let declaration = ancestor(
                decorator,
                &[
                    "class_declaration",
                    "abstract_class_declaration",
                    "method_definition",
                ],
            )
            .or_else(|| {
                ancestor(decorator, &["export_statement"]).and_then(|statement| {
                    first_descendant_of(
                        statement,
                        &["class_declaration", "abstract_class_declaration"],
                    )
                })
            });
            let Some(declaration) = declaration else {
                continue;
            };
            let Some(declared_name) = node_name(declaration, self.source) else {
                continue;
            };
            let target = if matches!(
                declaration.kind(),
                "class_declaration" | "abstract_class_declaration"
            ) {
                self.types.get(&declared_name).cloned()
            } else {
                let owner = ancestor(
                    declaration,
                    &["class_declaration", "abstract_class_declaration"],
                )
                .and_then(|node| node_name(node, self.source));
                owner.map(|owner| make_id(&[&make_id(&[&self.stem, &owner]), &declared_name]))
            };
            let Some(target) = target.filter(|target| self.has_node(target)) else {
                continue;
            };
            let annotation_name = self
                .text(decorator)
                .trim()
                .trim_start_matches('@')
                .split(['(', '.'])
                .next()
                .unwrap_or_default()
                .trim();
            if annotation_name.is_empty() {
                continue;
            }
            let id = make_id(&[
                &self.stem,
                "annotation",
                annotation_name,
                &decorator.start_byte().to_string(),
            ]);
            self.upsert_node(
                &id,
                annotation_name,
                annotation_name,
                "annotation",
                decorator,
                None,
            );
            self.edge(&self.file_id.clone(), &id, "contains", decorator, None);
            self.edge(&id, &target, "decorates", decorator, Some("decorator"));
        }
    }

    fn ts_overrides(&mut self, root: Node<'_>) {
        for declaration in descendants(root, &["class_declaration", "abstract_class_declaration"]) {
            let Some(class_name) = node_name(declaration, self.source) else {
                continue;
            };
            let superclass = declaration
                .child_by_field_name("superclass")
                .and_then(|node| type_name(node, self.source))
                .or_else(|| parse_extends_name(self.text(declaration)));
            let Some(superclass) = superclass else {
                continue;
            };
            let Some(base) = self.types.get(&superclass).cloned() else {
                continue;
            };
            let owner = make_id(&[&self.stem, &class_name]);
            let Some(body) = declaration.child_by_field_name("body") else {
                continue;
            };
            for method in body
                .named_children(&mut body.walk())
                .filter(|node| node.kind() == "method_definition")
            {
                if !self
                    .text(method)
                    .split_whitespace()
                    .any(|word| word == "override")
                {
                    continue;
                }
                let Some(name) = node_name(method, self.source) else {
                    continue;
                };
                let source = make_id(&[&owner, &name]);
                let target = make_id(&[&base, &name]);
                if self.has_node(&source) && self.has_node(&target) {
                    self.edge(
                        &source,
                        &target,
                        "overrides",
                        method,
                        Some("override-modifier"),
                    );
                }
            }
        }
    }

    fn csharp(&mut self, root: Node<'_>) {
        let namespace = descendants(
            root,
            &["file_scoped_namespace_declaration", "namespace_declaration"],
        )
        .first()
        .and_then(|node| node_name(*node, self.source))
        .unwrap_or_default();
        for declaration in descendants(
            root,
            &[
                "class_declaration",
                "interface_declaration",
                "enum_declaration",
                "struct_declaration",
                "record_declaration",
            ],
        ) {
            let Some(name) = node_name(declaration, self.source) else {
                continue;
            };
            let id = make_id(&[&self.stem, &namespace, &name]);
            let kind = match declaration.kind() {
                "interface_declaration" => "interface",
                "enum_declaration" => "enum",
                "struct_declaration" | "record_declaration" => "struct",
                _ => "class",
            };
            self.upsert_node(&id, &name, &name, kind, declaration, None);
            self.types.insert(name, id);
        }
        for declaration in descendants(
            root,
            &[
                "class_declaration",
                "interface_declaration",
                "struct_declaration",
                "record_declaration",
            ],
        ) {
            self.csharp_members(declaration);
        }
        self.csharp_overrides(root);
    }

    fn csharp_members(&mut self, declaration: Node<'_>) {
        let Some(owner_name) = node_name(declaration, self.source) else {
            return;
        };
        let Some(owner) = self.types.get(&owner_name).cloned() else {
            return;
        };
        let Some(body) = declaration
            .child_by_field_name("body")
            .or_else(|| first_descendant_of(declaration, &["declaration_list"]))
        else {
            return;
        };
        for member in body.named_children(&mut body.walk()) {
            match member.kind() {
                "constructor_declaration" | "method_declaration" => {
                    let name = if member.kind() == "constructor_declaration" {
                        owner_name.clone()
                    } else if let Some(name) = node_name(member, self.source) {
                        name
                    } else {
                        continue;
                    };
                    let kind = if member.kind() == "constructor_declaration" {
                        "constructor"
                    } else {
                        "method"
                    };
                    let id = make_id(&[&owner, &name]);
                    let qualified = format!("{owner_name}::{name}");
                    self.upsert_node(&id, &format!(".{name}()"), &qualified, kind, member, None);
                    self.edge(&owner, &id, "contains", member, None);
                    self.callables.insert(qualified.clone(), id.clone());
                    self.csharp_callable(member, &id, &qualified);
                }
                "property_declaration" => {
                    let Some(name) = node_name(member, self.source) else {
                        continue;
                    };
                    let id = make_id(&[&owner, "property", &name]);
                    self.upsert_node(
                        &id,
                        &name,
                        &format!("{owner_name}::{name}"),
                        "property",
                        member,
                        None,
                    );
                    self.edge(&owner, &id, "contains", member, None);
                    self.typed_edge(&id, member.child_by_field_name("type"), member);
                }
                "field_declaration" => {
                    let kind = if self
                        .text(member)
                        .split_whitespace()
                        .any(|word| word == "const")
                    {
                        "constant"
                    } else {
                        "field"
                    };
                    let Some(declaration) = first_descendant_of(member, &["variable_declaration"])
                    else {
                        continue;
                    };
                    let declared_type = declaration.child_by_field_name("type");
                    for variable in descendants(declaration, &["variable_declarator"]) {
                        let Some(name) = node_name(variable, self.source) else {
                            continue;
                        };
                        let id = make_id(&[&owner, kind, &name]);
                        self.upsert_node(
                            &id,
                            &name,
                            &format!("{owner_name}::{name}"),
                            kind,
                            variable,
                            None,
                        );
                        self.edge(&owner, &id, "contains", variable, None);
                        self.typed_edge(&id, declared_type, variable);
                    }
                }
                _ => {}
            }
        }
    }

    fn csharp_callable(&mut self, function: Node<'_>, id: &str, qualified: &str) {
        if let Some(parameters) = function.child_by_field_name("parameters") {
            for (position, parameter) in parameters
                .named_children(&mut parameters.walk())
                .filter(|node| node.kind() == "parameter")
                .enumerate()
            {
                let Some(name) = node_name(parameter, self.source) else {
                    continue;
                };
                let parameter_id = make_id(&[id, "parameter", &position.to_string(), &name]);
                self.upsert_node(
                    &parameter_id,
                    &name,
                    &format!("{qualified}::{name}"),
                    "parameter",
                    parameter,
                    None,
                );
                self.edge(id, &parameter_id, "contains", parameter, None);
                self.typed_edge(
                    &parameter_id,
                    parameter.child_by_field_name("type"),
                    parameter,
                );
            }
        }
        if let Some(return_type) = function
            .child_by_field_name("returns")
            .or_else(|| function.child_by_field_name("type"))
            && let Some(target) =
                type_name(return_type, self.source).and_then(|name| self.types.get(&name).cloned())
        {
            self.edge(id, &target, "returns", return_type, Some("return-type"));
        }
    }

    fn csharp_overrides(&mut self, root: Node<'_>) {
        for declaration in descendants(root, &["class_declaration"]) {
            let Some(class_name) = node_name(declaration, self.source) else {
                continue;
            };
            let Some(base_list) = first_descendant_of(declaration, &["base_list"]) else {
                continue;
            };
            let Some(base_name) = base_list
                .named_children(&mut base_list.walk())
                .find_map(|node| type_name(node, self.source))
            else {
                continue;
            };
            let (Some(owner), Some(base)) = (
                self.types.get(&class_name).cloned(),
                self.types.get(&base_name).cloned(),
            ) else {
                continue;
            };
            let Some(body) = declaration
                .child_by_field_name("body")
                .or_else(|| first_descendant_of(declaration, &["declaration_list"]))
            else {
                continue;
            };
            for method in body
                .named_children(&mut body.walk())
                .filter(|node| node.kind() == "method_declaration")
            {
                if !self
                    .text(method)
                    .split_whitespace()
                    .any(|word| word == "override")
                {
                    continue;
                }
                let Some(name) = node_name(method, self.source) else {
                    continue;
                };
                let source = make_id(&[&owner, &name]);
                let target = make_id(&[&base, &name]);
                if self.has_node(&source) && self.has_node(&target) {
                    self.edge(
                        &source,
                        &target,
                        "overrides",
                        method,
                        Some("override-modifier"),
                    );
                }
            }
        }
    }

    fn typed_edge(&mut self, source: &str, annotation: Option<Node<'_>>, site: Node<'_>) {
        let Some(target) = annotation
            .and_then(|node| type_name(node, self.source))
            .and_then(|name| self.types.get(&name).cloned())
        else {
            return;
        };
        self.edge(source, &target, "type_of", site, Some("declared-type"));
    }

    fn upsert_node(
        &mut self,
        id: &str,
        label: &str,
        qualified_name: &str,
        kind: &str,
        node: Node<'_>,
        extra: Option<Map<String, Value>>,
    ) {
        if let Some(existing) = self
            .extraction
            .nodes
            .iter_mut()
            .find(|record| record.id == id)
        {
            existing
                .attributes
                .insert("symbol_kind".to_owned(), Value::String(kind.to_owned()));
            existing.attributes.insert(
                "qualified_name".to_owned(),
                Value::String(qualified_name.to_owned()),
            );
            crate::facts::stamp_node_range(&mut existing.attributes, node);
            if let Some(extra) = extra {
                existing.attributes.extend(extra);
            }
            return;
        }
        let mut attributes = Map::from_iter([
            ("label".to_owned(), Value::String(label.to_owned())),
            ("name".to_owned(), Value::String(label.to_owned())),
            (
                "qualified_name".to_owned(),
                Value::String(qualified_name.to_owned()),
            ),
            ("symbol_kind".to_owned(), Value::String(kind.to_owned())),
            ("file_type".to_owned(), Value::String("code".to_owned())),
            (
                "source_file".to_owned(),
                Value::String(self.source_file.clone()),
            ),
            (
                "source_location".to_owned(),
                Value::String(format!("L{}", node.start_position().row + 1)),
            ),
            (
                "language".to_owned(),
                Value::String(self.language.to_owned()),
            ),
        ]);
        crate::facts::stamp_node_range(&mut attributes, node);
        if let Some(extra) = extra {
            attributes.extend(extra);
        }
        self.extraction.nodes.push(RawNodeRecord {
            id: id.to_owned(),
            attributes,
        });
    }

    fn edge(
        &mut self,
        source: &str,
        target: &str,
        relation: &str,
        node: Node<'_>,
        context: Option<&str>,
    ) {
        if source.is_empty() || target.is_empty() || source == target {
            return;
        }
        let key = (
            source.to_owned(),
            target.to_owned(),
            relation.to_owned(),
            node.start_byte(),
            node.end_byte(),
        );
        if !self.seen_edges.insert(key) {
            return;
        }
        let mut attributes = Map::from_iter([
            ("relation".to_owned(), Value::String(relation.to_owned())),
            (
                "confidence".to_owned(),
                Value::String("EXTRACTED".to_owned()),
            ),
            (
                "source_file".to_owned(),
                Value::String(self.source_file.clone()),
            ),
            (
                "source_location".to_owned(),
                Value::String(format!("L{}", node.start_position().row + 1)),
            ),
            ("weight".to_owned(), json!(1.0)),
            ("_origin".to_owned(), Value::String("ast".to_owned())),
        ]);
        if let Some(context) = context {
            attributes.insert("context".to_owned(), Value::String(context.to_owned()));
        }
        crate::facts::stamp_node_range(&mut attributes, node);
        self.extraction.edges.push(RawEdgeRecord {
            source: source.to_owned(),
            target: target.to_owned(),
            attributes,
        });
    }

    fn mark_test(&mut self, id: &str) {
        if let Some(node) = self.extraction.nodes.iter_mut().find(|node| node.id == id) {
            node.attributes.insert("roles".to_owned(), json!(["test"]));
        }
    }

    fn is_test(&self, id: &str) -> bool {
        self.extraction
            .nodes
            .iter()
            .find(|node| node.id == id)
            .and_then(|node| node.attributes.get("roles"))
            .and_then(Value::as_array)
            .is_some_and(|roles| roles.iter().any(|role| role.as_str() == Some("test")))
    }

    fn has_node(&self, id: &str) -> bool {
        self.extraction.nodes.iter().any(|node| node.id == id)
    }

    fn text(&self, node: Node<'_>) -> &'source str {
        node.utf8_text(self.source).unwrap_or_default()
    }
}

fn descendants<'tree>(node: Node<'tree>, kinds: &[&str]) -> Vec<Node<'tree>> {
    let mut output = Vec::new();
    collect_descendants(node, kinds, &mut output);
    output
}

fn collect_descendants<'tree>(node: Node<'tree>, kinds: &[&str], output: &mut Vec<Node<'tree>>) {
    if kinds.contains(&node.kind()) {
        output.push(node);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_descendants(child, kinds, output);
    }
}

fn first_descendant_of<'tree>(node: Node<'tree>, kinds: &[&str]) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if kinds.contains(&child.kind()) {
            return Some(child);
        }
        if let Some(found) = first_descendant_of(child, kinds) {
            return Some(found);
        }
    }
    None
}

fn ancestor<'tree>(node: Node<'tree>, kinds: &[&str]) -> Option<Node<'tree>> {
    let mut parent = node.parent();
    while let Some(candidate) = parent {
        if kinds.contains(&candidate.kind()) {
            return Some(candidate);
        }
        parent = candidate.parent();
    }
    None
}

fn ancestor_between(node: Node<'_>, boundary: Node<'_>, kind: &str) -> bool {
    let mut parent = node.parent();
    while let Some(candidate) = parent {
        if candidate.id() == boundary.id() {
            return false;
        }
        if candidate.kind() == kind {
            return true;
        }
        parent = candidate.parent();
    }
    false
}

fn node_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    node.child_by_field_name("name")
        .or_else(|| node.child_by_field_name("declarator"))
        .or_else(|| node.child_by_field_name("pattern"))
        .and_then(|name| type_name(name, source))
}

fn type_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    if matches!(
        node.kind(),
        "identifier"
            | "type_identifier"
            | "field_identifier"
            | "property_identifier"
            | "shorthand_property_identifier_pattern"
            | "predefined_type"
            | "primitive_type"
            | "self"
    ) {
        let name = node.utf8_text(source).ok()?.trim();
        return (!name.is_empty()).then(|| name.to_owned());
    }
    if matches!(
        node.kind(),
        "qualified_name" | "scoped_type_identifier" | "nested_type_identifier"
    ) {
        let raw = node.utf8_text(source).ok()?.trim();
        let name = raw.rsplit(['.', ':']).find(|part| !part.is_empty())?;
        return Some(name.to_owned());
    }
    if let Some(name) = node.child_by_field_name("name") {
        return type_name(name, source);
    }
    if let Some(type_node) = node.child_by_field_name("type") {
        return type_name(type_node, source);
    }
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find_map(|child| type_name(child, source))
}

fn parse_export_alias(text: &str) -> Option<(&str, &str)> {
    let body = text
        .trim()
        .strip_prefix("export")?
        .trim()
        .strip_prefix('{')?
        .split_once('}')?
        .0
        .trim();
    let (source, alias) = body.split_once(" as ")?;
    let source = source.trim();
    let alias = alias.trim().split(',').next()?.trim();
    (!source.is_empty() && !alias.is_empty()).then_some((source, alias))
}

fn parse_extends_name(text: &str) -> Option<String> {
    let (_, suffix) = text.split_once(" extends ")?;
    let name = suffix
        .split(|character: char| character.is_whitespace() || matches!(character, '{' | '<' | ','))
        .next()?
        .trim();
    (!name.is_empty()).then(|| name.to_owned())
}

fn has_rust_test_attribute(node: Node<'_>, source: &[u8]) -> bool {
    node.prev_named_sibling().is_some_and(|attribute| {
        attribute.kind() == "attribute_item"
            && attribute
                .utf8_text(source)
                .is_ok_and(|text| text.trim() == "#[test]")
    })
}
