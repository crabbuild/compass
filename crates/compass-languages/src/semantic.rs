use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde_json::{Map, Value, json};
use tree_sitter::Node;

use crate::{Extraction, RawEdgeRecord, RawNodeRecord, make_id};

pub(crate) fn enrich<'tree>(
    path: &Path,
    source: &[u8],
    root: Node<'tree>,
    language: &'static str,
    extraction: &mut Extraction,
) {
    let mut inventory = Vec::new();
    let mut work = Work::default();
    collect_inventory(
        root,
        source,
        language,
        &mut Vec::new(),
        None,
        &mut inventory,
        &mut work,
    );
    let mut state = State::new(path, source, language, extraction, inventory, work);
    match language {
        "rust" => state.rust(),
        "typescript" | "tsx" => state.typescript(),
        "csharp" => state.csharp(),
        _ => {}
    }
    state.publish_work();
}

#[derive(Clone)]
struct Item<'tree> {
    node: Node<'tree>,
    scope: Vec<String>,
    owner_ast: Option<usize>,
    callable_ast: Option<usize>,
}

#[derive(Clone)]
struct TypeRecord {
    id: String,
}

#[derive(Clone)]
struct CallableRecord {
    id: String,
    name: String,
    scope: Vec<String>,
    signature: String,
    ast_id: usize,
    is_test: bool,
}

#[derive(Default)]
struct Work {
    ast_visits: usize,
    index_lookups: usize,
    declarations: usize,
}

struct State<'source, 'tree, 'extraction> {
    source: &'source [u8],
    source_file: String,
    file_id: String,
    language: &'static str,
    extraction: &'extraction mut Extraction,
    inventory: Vec<Item<'tree>>,
    inventory_index: HashMap<usize, usize>,
    node_index: HashMap<String, usize>,
    anchor_index: HashMap<(usize, usize), Vec<usize>>,
    label_line_index: HashMap<(String, usize), Vec<usize>>,
    id_anchors: HashMap<String, HashSet<(usize, usize)>>,
    seen_edges: HashSet<(String, String, String, usize, usize)>,
    owner_ids: HashMap<usize, String>,
    owner_ast_by_id: HashMap<String, usize>,
    types: HashMap<String, Vec<TypeRecord>>,
    callables: HashMap<String, Vec<CallableRecord>>,
    callable_ast_index: HashMap<usize, CallableRecord>,
    callables_by_owner: HashMap<usize, Vec<CallableRecord>>,
    methods_by_owner: HashMap<(usize, String, String), Vec<String>>,
    work: Work,
}

impl<'source, 'tree, 'extraction> State<'source, 'tree, 'extraction> {
    fn new(
        path: &Path,
        source: &'source [u8],
        language: &'static str,
        extraction: &'extraction mut Extraction,
        inventory: Vec<Item<'tree>>,
        work: Work,
    ) -> Self {
        let mut node_index = HashMap::new();
        let inventory_index = inventory
            .iter()
            .enumerate()
            .map(|(index, item)| (item.node.id(), index))
            .collect();
        let mut anchor_index = HashMap::<(usize, usize), Vec<usize>>::new();
        let mut label_line_index = HashMap::<(String, usize), Vec<usize>>::new();
        let mut id_anchors = HashMap::<String, HashSet<(usize, usize)>>::new();
        for (index, node) in extraction.nodes.iter().enumerate() {
            node_index.entry(node.id.clone()).or_insert(index);
            let line = node
                .attributes
                .get("source_location")
                .and_then(Value::as_str)
                .and_then(|value| value.strip_prefix('L'))
                .and_then(|value| value.parse().ok())
                .unwrap_or_default();
            label_line_index
                .entry((normalized_label(&node.string("label")), line))
                .or_default()
                .push(index);
            if let Some(anchor) = record_anchor(node) {
                anchor_index.entry(anchor).or_default().push(index);
                id_anchors
                    .entry(node.id.clone())
                    .or_default()
                    .insert(anchor);
            }
        }
        let seen_edges = extraction
            .edges
            .iter()
            .map(|edge| {
                (
                    edge.source.clone(),
                    edge.target.clone(),
                    edge.string("relation"),
                    edge_usize(edge, "start_byte"),
                    edge_usize(edge, "end_byte"),
                )
            })
            .collect();
        Self {
            source,
            source_file: path.to_string_lossy().into_owned(),
            file_id: make_id(&[&path.to_string_lossy()]),
            language,
            extraction,
            inventory,
            inventory_index,
            node_index,
            anchor_index,
            label_line_index,
            id_anchors,
            seen_edges,
            owner_ids: HashMap::new(),
            owner_ast_by_id: HashMap::new(),
            types: HashMap::new(),
            callables: HashMap::new(),
            callable_ast_index: HashMap::new(),
            callables_by_owner: HashMap::new(),
            methods_by_owner: HashMap::new(),
            work,
        }
    }

    fn publish_work(&mut self) {
        self.extraction.extensions.insert(
            "_semantic_work".to_owned(),
            json!({
                "ast_visits": self.work.ast_visits,
                "index_lookups": self.work.index_lookups,
                "declarations": self.work.declarations,
            }),
        );
    }

    fn rust(&mut self) {
        self.ensure_scopes(&["mod_item"], "module");
        self.ensure_types(&[
            ("trait_item", "trait"),
            ("struct_item", "struct"),
            ("enum_item", "enum"),
            ("type_item", "type_alias"),
        ]);
        self.bind_rust_impl_owners();

        for item in self.items_of(&["field_declaration"]) {
            let Some(owner) = self.closest_owner(&item) else {
                continue;
            };
            let Some(name) = node_name(item.node, self.source) else {
                continue;
            };
            let id = self.ensure_member(&item, &owner, &name, "field", None);
            self.typed_edge(
                &id,
                item.node.child_by_field_name("type"),
                item.node,
                &item.scope,
            );
        }
        for item in self.items_of(&["enum_variant"]) {
            let Some(owner) = self.closest_owner(&item) else {
                continue;
            };
            let Some(name) = node_name(item.node, self.source) else {
                continue;
            };
            let declaring_type = self.node_qualified(&owner);
            let extra =
                Map::from_iter([("declaring_type".to_owned(), Value::String(declaring_type))]);
            let id = self.ensure_member(&item, &owner, &name, "enum_member", Some(extra));
            self.edge(&owner, &id, "contains", item.node, None);
        }
        for item in self.items_of(&["const_item"]) {
            let Some(name) = node_name(item.node, self.source) else {
                continue;
            };
            let owner = self
                .closest_owner(&item)
                .unwrap_or_else(|| self.file_id.clone());
            let id = self.ensure_member(&item, &owner, &name, "constant", None);
            self.typed_edge(
                &id,
                item.node.child_by_field_name("type"),
                item.node,
                &item.scope,
            );
        }
        for item in self.items_of(&["macro_definition"]) {
            let Some(name) = node_name(item.node, self.source) else {
                continue;
            };
            let owner = self
                .closest_owner(&item)
                .unwrap_or_else(|| self.file_id.clone());
            self.ensure_member(&item, &owner, &name, "macro", None);
        }

        self.ensure_rust_callables();
        self.rust_aliases();
        self.callable_semantics("rust");
        self.rust_overrides();
        self.rust_tests();
        self.prune_coalesced_base_shadows();
    }

    fn typescript(&mut self) {
        self.ensure_scopes(
            &[
                "internal_module",
                "module_declaration",
                "namespace_declaration",
            ],
            "namespace",
        );
        self.ensure_types(&[
            ("class_declaration", "class"),
            ("abstract_class_declaration", "class"),
            ("interface_declaration", "interface"),
            ("enum_declaration", "enum"),
            ("type_alias_declaration", "type_alias"),
        ]);
        self.ensure_ts_callables();

        for item in self.items_of(&[
            "public_field_definition",
            "property_signature",
            "abstract_property_signature",
        ]) {
            let Some(owner) = self.closest_owner(&item) else {
                continue;
            };
            let Some(name) = node_name(item.node, self.source) else {
                continue;
            };
            let id = self.ensure_member(&item, &owner, &name, "property", None);
            self.typed_edge(
                &id,
                item.node.child_by_field_name("type"),
                item.node,
                &item.scope,
            );
        }
        self.callable_semantics("typescript");
        self.ts_exports();
        self.ts_decorators();
        self.ts_overrides();
        self.prune_coalesced_base_shadows();
    }

    fn csharp(&mut self) {
        self.ensure_scopes(
            &["file_scoped_namespace_declaration", "namespace_declaration"],
            "namespace",
        );
        self.ensure_types(&[
            ("class_declaration", "class"),
            ("interface_declaration", "interface"),
            ("enum_declaration", "enum"),
            ("struct_declaration", "struct"),
            ("record_declaration", "struct"),
        ]);
        self.ensure_csharp_callables();

        for item in self.items_of(&["property_declaration"]) {
            let Some(owner) = self.closest_owner(&item) else {
                continue;
            };
            let Some(name) = node_name(item.node, self.source) else {
                continue;
            };
            let id = self.ensure_member(&item, &owner, &name, "property", None);
            self.typed_edge(
                &id,
                item.node.child_by_field_name("type"),
                item.node,
                &item.scope,
            );
        }
        for item in self.items_of(&["variable_declarator"]) {
            let Some(field) = ancestor_of_kind(item.node, &["field_declaration"]) else {
                continue;
            };
            let Some(owner) = self.closest_owner(&item) else {
                continue;
            };
            let Some(name) = node_name(item.node, self.source) else {
                continue;
            };
            let kind = if has_modifier(field, "const", self.source) {
                "constant"
            } else {
                "field"
            };
            let id = self.ensure_member(&item, &owner, &name, kind, None);
            let declared_type = ancestor_of_kind(item.node, &["variable_declaration"])
                .and_then(|node| node.child_by_field_name("type"));
            self.typed_edge(&id, declared_type, item.node, &item.scope);
        }
        self.callable_semantics("csharp");
        self.csharp_overrides();
        self.prune_coalesced_base_shadows();
    }

    fn ensure_scopes(&mut self, kinds: &[&str], semantic_kind: &str) {
        for item in self.items_of(kinds) {
            let Some(name) = scope_node_name(item.node, self.source, self.language) else {
                continue;
            };
            let qualified = qualify(&item.scope, &name);
            let id = self.ensure_node(item.node, &name, &qualified, semantic_kind, None, None);
            self.register_owner(item.node.id(), id.clone());
            let owner = item
                .owner_ast
                .and_then(|ast| self.owner_ids.get(&ast).cloned())
                .unwrap_or_else(|| self.file_id.clone());
            self.edge(&owner, &id, "contains", item.node, None);
        }
    }

    fn ensure_types(&mut self, kinds: &[(&str, &str)]) {
        for item in self
            .inventory
            .clone()
            .into_iter()
            .filter(|item| kinds.iter().any(|(kind, _)| item.node.kind() == *kind))
        {
            let Some(name) = node_name(item.node, self.source) else {
                continue;
            };
            let kind = kinds
                .iter()
                .find_map(|(node_kind, semantic)| {
                    (item.node.kind() == *node_kind).then_some(*semantic)
                })
                .unwrap_or("class");
            let logical_name = qualify(&item.scope, &name);
            let qualified = format!("{logical_name}@{}", item.node.start_byte());
            let id = self.ensure_node(item.node, &name, &qualified, kind, None, None);
            self.register_owner(item.node.id(), id.clone());
            self.types
                .entry(logical_name.clone())
                .or_default()
                .push(TypeRecord { id: id.clone() });
            let owner = item
                .owner_ast
                .and_then(|ast| self.owner_ids.get(&ast).cloned())
                .unwrap_or_else(|| self.file_id.clone());
            self.edge(&owner, &id, "contains", item.node, None);
        }
    }

    fn bind_rust_impl_owners(&mut self) {
        for item in self.items_of(&["impl_item"]) {
            let Some(type_node) = item.node.child_by_field_name("type") else {
                continue;
            };
            let Some(owner) = self.resolve_type(type_node, &item.scope) else {
                continue;
            };
            self.register_owner(item.node.id(), owner);
        }
    }

    fn ensure_rust_callables(&mut self) {
        for item in self.items_of(&["function_item", "function_signature_item"]) {
            let Some(name) = node_name(item.node, self.source) else {
                continue;
            };
            let owner = self
                .closest_owner(&item)
                .unwrap_or_else(|| self.file_id.clone());
            let is_method = item.owner_ast.is_some_and(|ast| {
                self.item_by_ast(ast)
                    .is_some_and(|owner| matches!(owner.node.kind(), "trait_item" | "impl_item"))
            });
            let kind = if is_method { "method" } else { "function" };
            let record = self.ensure_callable(&item, &owner, &name, kind);
            self.register_callable(record);
        }
    }

    fn ensure_ts_callables(&mut self) {
        for item in self.items_of(&[
            "function_declaration",
            "method_definition",
            "method_signature",
        ]) {
            let Some(name) = node_name(item.node, self.source) else {
                continue;
            };
            let owner = self
                .closest_owner(&item)
                .unwrap_or_else(|| self.file_id.clone());
            let kind = if item.node.kind() == "function_declaration" {
                "function"
            } else if name == "constructor" {
                "constructor"
            } else {
                "method"
            };
            let record = self.ensure_callable(&item, &owner, &name, kind);
            self.register_callable(record);
        }
    }

    fn ensure_csharp_callables(&mut self) {
        for item in self.items_of(&["constructor_declaration", "method_declaration"]) {
            let owner = self
                .closest_owner(&item)
                .unwrap_or_else(|| self.file_id.clone());
            let is_constructor = item.node.kind() == "constructor_declaration";
            let name = if is_constructor {
                self.node_qualified(&owner)
                    .rsplit("::")
                    .next()
                    .unwrap_or("constructor")
                    .split('@')
                    .next()
                    .unwrap_or("constructor")
                    .to_owned()
            } else if let Some(name) = node_name(item.node, self.source) {
                name
            } else {
                continue;
            };
            let record = self.ensure_callable(
                &item,
                &owner,
                &name,
                if is_constructor {
                    "constructor"
                } else {
                    "method"
                },
            );
            self.register_callable(record);
        }
    }

    fn ensure_callable(
        &mut self,
        item: &Item<'tree>,
        owner: &str,
        name: &str,
        kind: &str,
    ) -> CallableRecord {
        let signature = callable_signature(item.node, self.source);
        let logical = qualify(&item.scope, name);
        let qualified = format!("{logical}{signature}@{}", item.node.start_byte());
        let extra = Map::from_iter([
            ("signature".to_owned(), Value::String(signature.clone())),
            (
                "overload_discriminator".to_owned(),
                Value::String(format!("{signature}@{}", item.node.start_byte())),
            ),
        ]);
        let id = self.ensure_node(
            item.node,
            &format!(".{name}{signature}"),
            &qualified,
            kind,
            Some(&signature),
            Some(extra),
        );
        self.edge(owner, &id, "contains", item.node, None);
        let is_test = self.language == "rust" && has_rust_test_attribute(item.node, self.source);
        if is_test {
            self.mark_test(&id, name, item.node);
        }
        CallableRecord {
            id,
            name: name.to_owned(),
            scope: item.scope.clone(),
            signature,
            ast_id: item.node.id(),
            is_test,
        }
    }

    fn register_callable(&mut self, record: CallableRecord) {
        let key = qualify(&record.scope, &record.name);
        if let Some(owner_ast) = self
            .item_by_ast(record.ast_id)
            .and_then(|item| item.owner_ast)
        {
            self.callables_by_owner
                .entry(owner_ast)
                .or_default()
                .push(record.clone());
            self.methods_by_owner
                .entry((owner_ast, record.name.clone(), record.signature.clone()))
                .or_default()
                .push(record.id.clone());
        }
        self.callable_ast_index
            .insert(record.ast_id, record.clone());
        self.callables.entry(key).or_default().push(record);
    }

    fn callable_semantics(&mut self, family: &str) {
        let callables = self
            .callables
            .values()
            .flatten()
            .cloned()
            .collect::<Vec<_>>();
        for callable in callables {
            let Some(item) = self.item_by_ast(callable.ast_id).cloned() else {
                continue;
            };
            if let Some(parameters) = item.node.child_by_field_name("parameters") {
                let parameters = direct_named_children(parameters)
                    .into_iter()
                    .filter(|node| {
                        matches!(
                            node.kind(),
                            "parameter"
                                | "required_parameter"
                                | "optional_parameter"
                                | "rest_pattern"
                                | "self_parameter"
                        )
                    })
                    .collect::<Vec<_>>();
                for (position, parameter) in parameters.into_iter().enumerate() {
                    let name = parameter_name(parameter, self.source)
                        .unwrap_or_else(|| format!("parameter_{position}"));
                    let extra = Map::from_iter([("position".to_owned(), json!(position))]);
                    let id = self.ensure_node(
                        parameter,
                        &name,
                        &format!(
                            "{}::{name}@{}",
                            self.node_qualified(&callable.id),
                            parameter.start_byte()
                        ),
                        "parameter",
                        None,
                        Some(extra),
                    );
                    self.edge(&callable.id, &id, "contains", parameter, None);
                    self.typed_edge(
                        &id,
                        parameter.child_by_field_name("type"),
                        parameter,
                        &item.scope,
                    );
                }
            }
            let return_type = match family {
                "rust" | "typescript" => item.node.child_by_field_name("return_type"),
                "csharp" => item
                    .node
                    .child_by_field_name("returns")
                    .or_else(|| item.node.child_by_field_name("type")),
                _ => None,
            };
            if let Some(return_type) = return_type
                && let Some(target) = self.resolve_type(return_type, &item.scope)
            {
                self.edge(
                    &callable.id,
                    &target,
                    "returns",
                    return_type,
                    Some("return-type"),
                );
            }
        }
    }

    fn rust_aliases(&mut self) {
        for item in self.items_of(&["type_item"]) {
            let Some(source) = self.owner_ids.get(&item.node.id()).cloned() else {
                continue;
            };
            let Some(type_node) = item.node.child_by_field_name("type") else {
                continue;
            };
            if let Some(target) = self.resolve_type(type_node, &item.scope) {
                self.edge(&source, &target, "aliases", type_node, Some("type-alias"));
            }
        }
    }

    fn rust_overrides(&mut self) {
        for item in self.items_of(&["impl_item"]) {
            let Some(trait_node) = item.node.child_by_field_name("trait") else {
                continue;
            };
            let Some(trait_id) = self.resolve_type(trait_node, &item.scope) else {
                continue;
            };
            let methods = self
                .callables_by_owner
                .get(&item.node.id())
                .cloned()
                .unwrap_or_default();
            for method in methods {
                if let Some(target) =
                    self.method_in_owner(&trait_id, &method.name, &method.signature)
                    && let Some(site) = self.item_by_ast(method.ast_id).map(|item| item.node)
                {
                    self.edge(
                        &method.id,
                        &target,
                        "overrides",
                        site,
                        Some("trait-implementation"),
                    );
                }
            }
        }
    }

    fn rust_tests(&mut self) {
        let tests = self
            .callables
            .values()
            .flatten()
            .filter(|callable| callable.is_test)
            .cloned()
            .collect::<Vec<_>>();
        for test in tests {
            for item in self.inventory.clone().into_iter().filter(|item| {
                item.node.kind() == "call_expression" && item.callable_ast == Some(test.ast_id)
            }) {
                let Some(function) = item.node.child_by_field_name("function") else {
                    continue;
                };
                let Some(name) = expression_name(function, self.source) else {
                    continue;
                };
                if let Some(target) = self.resolve_callable(&name, &test.scope, None)
                    && target != test.id
                {
                    self.edge(
                        &test.id,
                        &target,
                        "tests",
                        item.node,
                        Some("direct-test-call"),
                    );
                }
            }
        }
    }

    fn ts_exports(&mut self) {
        for statement in self.items_of(&["export_statement"]) {
            let specifiers = descendants_bounded(statement.node, &["export_specifier"]);
            if !specifiers.is_empty() {
                for specifier in specifiers {
                    let local_node = specifier
                        .child_by_field_name("name")
                        .or_else(|| direct_named_children(specifier).first().copied());
                    let alias_node = specifier
                        .child_by_field_name("alias")
                        .or_else(|| specifier.child_by_field_name("value"));
                    let Some(local_node) = local_node else {
                        continue;
                    };
                    let local = clean_identifier(self.text(local_node));
                    let exported = alias_node
                        .map(|node| clean_identifier(self.text(node)))
                        .unwrap_or_else(|| local.clone());
                    if local.is_empty() || exported.is_empty() {
                        continue;
                    }
                    let target = self
                        .resolve_type_name(&local, &statement.scope)
                        .or_else(|| self.resolve_callable(&local, &statement.scope, None));
                    let Some(target) = target else {
                        continue;
                    };
                    self.emit_export(
                        &statement,
                        specifier,
                        &local,
                        &exported,
                        &target,
                        local != exported,
                    );
                }
                continue;
            }
            let declarations = descendants_bounded(
                statement.node,
                &[
                    "class_declaration",
                    "abstract_class_declaration",
                    "interface_declaration",
                    "enum_declaration",
                    "type_alias_declaration",
                    "function_declaration",
                ],
            );
            for declaration in declarations {
                let Some(name) = node_name(declaration, self.source) else {
                    continue;
                };
                let target = self
                    .owner_ids
                    .get(&declaration.id())
                    .cloned()
                    .or_else(|| self.resolve_callable(&name, &statement.scope, None));
                if let Some(target) = target {
                    self.emit_export(&statement, declaration, &name, &name, &target, false);
                }
            }
        }
    }

    fn emit_export(
        &mut self,
        statement: &Item<'tree>,
        site: Node<'tree>,
        local: &str,
        exported: &str,
        target: &str,
        aliased: bool,
    ) {
        let qualified = format!(
            "{}@{}",
            qualify(&statement.scope, exported),
            site.start_byte()
        );
        let extra = Map::from_iter([
            ("specifier".to_owned(), Value::String(exported.to_owned())),
            ("imported_name".to_owned(), Value::String(local.to_owned())),
            ("local_name".to_owned(), Value::String(exported.to_owned())),
        ]);
        let id = self.ensure_node(site, exported, &qualified, "export", None, Some(extra));
        let owner = self
            .closest_owner(statement)
            .unwrap_or_else(|| self.file_id.clone());
        self.edge(&owner, &id, "contains", site, None);
        self.edge(&id, target, "exports", site, Some("named-export-target"));
        if aliased {
            self.edge(&id, target, "aliases", site, Some("export-alias"));
        }
    }

    fn ts_decorators(&mut self) {
        for item in self.items_of(&["decorator"]) {
            let expression = direct_named_children(item.node).first().copied();
            let Some(expression) = expression else {
                continue;
            };
            let callee = if expression.kind() == "call_expression" {
                expression
                    .child_by_field_name("function")
                    .unwrap_or(expression)
            } else {
                expression
            };
            let name = self.text(callee).trim().trim_start_matches('@').to_owned();
            if name.is_empty() {
                continue;
            }
            let target_ast = item
                .owner_ast
                .or_else(|| item.node.next_named_sibling().map(|node| node.id()));
            let Some(target) = target_ast.and_then(|ast| {
                self.owner_ids.get(&ast).cloned().or_else(|| {
                    self.callable_ast_index
                        .get(&ast)
                        .map(|callable| callable.id.clone())
                })
            }) else {
                continue;
            };
            let qualified = format!("{}@{}", qualify(&item.scope, &name), item.node.start_byte());
            let id = self.ensure_node(item.node, &name, &qualified, "annotation", None, None);
            let owner = self
                .closest_owner(&item)
                .unwrap_or_else(|| self.file_id.clone());
            self.edge(&owner, &id, "contains", item.node, None);
            self.edge(&id, &target, "decorates", item.node, Some("decorator"));
        }
    }

    fn ts_overrides(&mut self) {
        for item in self.items_of(&["method_definition"]) {
            if !has_modifier(item.node, "override", self.source) {
                continue;
            }
            let Some(owner_ast) = item.owner_ast else {
                continue;
            };
            let Some(class_item) = self.item_by_ast(owner_ast).cloned() else {
                continue;
            };
            let base = class_item
                .node
                .child_by_field_name("superclass")
                .and_then(|node| self.resolve_type(node, &class_item.scope))
                .or_else(|| {
                    descendants_bounded(class_item.node, &["class_heritage", "extends_clause"])
                        .into_iter()
                        .find_map(|heritage| {
                            type_reference(heritage, self.source)
                                .and_then(|name| self.resolve_type_name(&name, &class_item.scope))
                        })
                });
            let Some(base) = base else {
                continue;
            };
            let Some(method) = self.callable_by_ast(item.node.id()) else {
                continue;
            };
            if let Some(target) = self.method_in_owner(&base, &method.name, &method.signature) {
                self.edge(
                    &method.id,
                    &target,
                    "overrides",
                    item.node,
                    Some("override-modifier"),
                );
            }
        }
    }

    fn csharp_overrides(&mut self) {
        for item in self.items_of(&["method_declaration"]) {
            if !has_modifier(item.node, "override", self.source) {
                continue;
            }
            let Some(owner_ast) = item.owner_ast else {
                continue;
            };
            let Some(class_item) = self.item_by_ast(owner_ast).cloned() else {
                continue;
            };
            let Some(base_list) = direct_named_children(class_item.node)
                .into_iter()
                .find(|node| node.kind() == "base_list")
            else {
                continue;
            };
            let Some(base_node) = direct_named_children(base_list).first().copied() else {
                continue;
            };
            let Some(base) = self.resolve_type(base_node, &class_item.scope) else {
                continue;
            };
            let Some(method) = self.callable_by_ast(item.node.id()) else {
                continue;
            };
            if let Some(target) = self.method_in_owner(&base, &method.name, &method.signature) {
                self.edge(
                    &method.id,
                    &target,
                    "overrides",
                    item.node,
                    Some("override-modifier"),
                );
            }
        }
    }

    fn ensure_member(
        &mut self,
        item: &Item<'tree>,
        owner: &str,
        name: &str,
        kind: &str,
        extra: Option<Map<String, Value>>,
    ) -> String {
        let qualified = format!(
            "{}::{name}@{}",
            self.node_qualified(owner),
            item.node.start_byte()
        );
        let id = self.ensure_node(item.node, name, &qualified, kind, None, extra);
        self.edge(owner, &id, "contains", item.node, None);
        id
    }

    fn ensure_node(
        &mut self,
        node: Node<'tree>,
        label: &str,
        qualified_name: &str,
        kind: &str,
        signature: Option<&str>,
        extra: Option<Map<String, Value>>,
    ) -> String {
        self.work.declarations += 1;
        let anchor = (node.start_byte(), node.end_byte());
        let desired = make_id(&[
            &self.source_file,
            qualified_name,
            kind,
            signature.unwrap_or_default(),
            &node.start_byte().to_string(),
        ]);
        self.work.index_lookups += 1;
        let compatible_index = |index: &usize| {
            self.extraction.nodes.get(*index).is_some_and(|record| {
                let existing_label = record.string("label");
                compatible_label(&existing_label, label)
                    && compatible_kind(&record.string("symbol_kind"), kind)
            })
        };
        let existing = self
            .anchor_index
            .get(&anchor)
            .and_then(|indexes| {
                indexes
                    .iter()
                    .find(|index| compatible_index(index))
                    .copied()
            })
            .or_else(|| {
                self.label_line_index
                    .get(&(
                        normalized_label(label),
                        node.start_position().row.saturating_add(1),
                    ))
                    .and_then(|indexes| {
                        indexes
                            .iter()
                            .find(|index| compatible_index(index))
                            .copied()
                    })
            });
        let index = if let Some(index) = existing {
            let old_id = self.extraction.nodes[index].id.clone();
            let collision = self
                .id_anchors
                .get(&old_id)
                .is_some_and(|anchors| anchors.len() > 1);
            if collision {
                self.extraction.nodes[index].id.clone_from(&desired);
                self.node_index.insert(desired.clone(), index);
            }
            index
        } else {
            let id = self.unique_id(&desired, anchor);
            let attributes = Map::from_iter([
                ("label".to_owned(), Value::String(label.to_owned())),
                ("name".to_owned(), Value::String(label.to_owned())),
                (
                    "source_file".to_owned(),
                    Value::String(self.source_file.clone()),
                ),
                ("file_type".to_owned(), Value::String("code".to_owned())),
                (
                    "language".to_owned(),
                    Value::String(self.language.to_owned()),
                ),
                ("_origin".to_owned(), Value::String("ast".to_owned())),
            ]);
            let index = self.extraction.nodes.len();
            self.extraction.nodes.push(RawNodeRecord {
                id: id.clone(),
                attributes,
            });
            self.node_index.insert(id, index);
            self.anchor_index.entry(anchor).or_default().push(index);
            index
        };
        let record = &mut self.extraction.nodes[index];
        record
            .attributes
            .insert("symbol_kind".to_owned(), Value::String(kind.to_owned()));
        record.attributes.insert(
            "qualified_name".to_owned(),
            Value::String(qualified_name.to_owned()),
        );
        record
            .attributes
            .entry("_origin".to_owned())
            .or_insert_with(|| Value::String("ast".to_owned()));
        if let Some(signature) = signature {
            record
                .attributes
                .insert("signature".to_owned(), Value::String(signature.to_owned()));
        }
        if let Some(extra) = extra {
            record.attributes.extend(extra);
        }
        crate::facts::stamp_node_range(&mut record.attributes, node);
        record.id.clone()
    }

    fn unique_id(&mut self, desired: &str, anchor: (usize, usize)) -> String {
        self.work.index_lookups += 1;
        if !self.node_index.contains_key(desired) {
            return desired.to_owned();
        }
        make_id(&[
            desired,
            "occurrence",
            &anchor.0.to_string(),
            &anchor.1.to_string(),
        ])
    }

    fn edge(
        &mut self,
        source: &str,
        target: &str,
        relation: &str,
        node: Node<'tree>,
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

    fn typed_edge(
        &mut self,
        source: &str,
        annotation: Option<Node<'tree>>,
        site: Node<'tree>,
        scope: &[String],
    ) {
        if let Some(annotation) = annotation
            && let Some(target) = self.resolve_type(annotation, scope)
        {
            self.edge(source, &target, "type_of", site, Some("declared-type"));
        }
    }

    fn resolve_type(&mut self, node: Node<'tree>, scope: &[String]) -> Option<String> {
        let reference = type_reference(node, self.source)?;
        self.resolve_type_name(&reference, scope)
    }

    fn resolve_type_name(&mut self, reference: &str, scope: &[String]) -> Option<String> {
        self.work.index_lookups += 1;
        let reference = normalize_path(reference);
        if reference.contains("::") {
            for depth in (0..=scope.len()).rev() {
                let prefix = &scope[..depth];
                let candidate = qualify(prefix, &reference);
                if let Some(records) = self.types.get(&candidate)
                    && let [record] = records.as_slice()
                {
                    return Some(record.id.clone());
                }
            }
            return None;
        }
        for depth in (0..=scope.len()).rev() {
            let candidate = qualify(&scope[..depth], &reference);
            if let Some(records) = self.types.get(&candidate) {
                return match records.as_slice() {
                    [record] => Some(record.id.clone()),
                    _ => None,
                };
            }
        }
        None
    }

    fn resolve_callable(
        &mut self,
        name: &str,
        scope: &[String],
        signature: Option<&str>,
    ) -> Option<String> {
        self.work.index_lookups += 1;
        let name = normalize_path(name);
        for depth in (0..=scope.len()).rev() {
            let key = qualify(&scope[..depth], &name);
            if let Some(records) = self.callables.get(&key) {
                let matches = records
                    .iter()
                    .filter(|record| signature.is_none_or(|value| record.signature == value))
                    .collect::<Vec<_>>();
                return match matches.as_slice() {
                    [record] => Some(record.id.clone()),
                    _ => None,
                };
            }
        }
        None
    }

    fn method_in_owner(&self, owner: &str, name: &str, signature: &str) -> Option<String> {
        let owner_ast = self.owner_ast_by_id.get(owner)?;
        match self
            .methods_by_owner
            .get(&(*owner_ast, name.to_owned(), signature.to_owned()))
            .map(Vec::as_slice)
        {
            Some([id]) => Some(id.clone()),
            _ => None,
        }
    }

    fn closest_owner(&self, item: &Item<'tree>) -> Option<String> {
        let mut ast = item.owner_ast;
        while let Some(owner) = ast {
            if let Some(id) = self.owner_ids.get(&owner) {
                return Some(id.clone());
            }
            ast = self.item_by_ast(owner).and_then(|item| item.owner_ast);
        }
        None
    }

    fn mark_test(&mut self, id: &str, name: &str, site: Node<'tree>) {
        self.work.index_lookups += 1;
        let mut indexes = self
            .label_line_index
            .get(&(
                normalized_label(name),
                site.start_position().row.saturating_add(1),
            ))
            .cloned()
            .unwrap_or_default();
        if let Some(index) = self.node_index.get(id).copied() {
            indexes.push(index);
        }
        indexes.sort_unstable();
        indexes.dedup();
        for index in indexes {
            if let Some(node) = self.extraction.nodes.get_mut(index) {
                node.attributes.insert("roles".to_owned(), json!(["test"]));
            }
        }
    }

    fn node_qualified(&self, id: &str) -> String {
        self.node_index
            .get(id)
            .and_then(|index| self.extraction.nodes.get(*index))
            .map(|node| node.string("qualified_name"))
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| id.to_owned())
    }

    fn callable_by_ast(&self, ast: usize) -> Option<CallableRecord> {
        self.callable_ast_index.get(&ast).cloned()
    }

    fn item_by_ast(&self, ast: usize) -> Option<&Item<'tree>> {
        self.inventory_index
            .get(&ast)
            .and_then(|index| self.inventory.get(*index))
    }

    fn register_owner(&mut self, ast: usize, id: String) {
        self.owner_ast_by_id.insert(id.clone(), ast);
        self.owner_ids.insert(ast, id);
    }

    fn items_of(&self, kinds: &[&str]) -> Vec<Item<'tree>> {
        self.inventory
            .iter()
            .filter(|item| kinds.contains(&item.node.kind()))
            .cloned()
            .collect()
    }

    fn text(&self, node: Node<'tree>) -> &'source str {
        node.utf8_text(self.source).unwrap_or_default()
    }

    fn prune_coalesced_base_shadows(&mut self) {
        let mut anchored = HashMap::<(String, String), usize>::new();
        for node in &self.extraction.nodes {
            if record_anchor(node).is_some() {
                *anchored
                    .entry((
                        node.string("symbol_kind"),
                        normalized_label(&node.string("label")),
                    ))
                    .or_default() += 1;
            }
        }
        let removed = self
            .extraction
            .nodes
            .iter()
            .filter(|node| {
                record_anchor(node).is_none()
                    && node.string("qualified_name").is_empty()
                    && anchored
                        .get(&(
                            node.string("symbol_kind"),
                            normalized_label(&node.string("label")),
                        ))
                        .is_some_and(|count| *count > 1)
            })
            .map(|node| node.id.clone())
            .collect::<HashSet<_>>();
        if removed.is_empty() {
            return;
        }
        self.extraction
            .nodes
            .retain(|node| !removed.contains(&node.id));
        self.extraction
            .edges
            .retain(|edge| !removed.contains(&edge.source) && !removed.contains(&edge.target));
    }
}

fn collect_inventory<'tree>(
    node: Node<'tree>,
    source: &[u8],
    language: &str,
    scope: &mut Vec<String>,
    callable_ast: Option<usize>,
    output: &mut Vec<Item<'tree>>,
    work: &mut Work,
) {
    work.ast_visits += 1;
    let owner_ast = closest_scope_ancestor(node);
    output.push(Item {
        node,
        scope: scope.clone(),
        owner_ast,
        callable_ast,
    });
    let mut next_callable = callable_ast;
    if is_callable_kind(node.kind(), language) {
        next_callable = Some(node.id());
    }
    let pushed = if is_scope_kind(node.kind(), language) {
        scope_node_name(node, source, language).map(|name| {
            scope.push(name);
        })
    } else {
        None
    };
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_inventory(child, source, language, scope, next_callable, output, work);
    }
    if pushed.is_some() {
        scope.pop();
    }
}

fn closest_scope_ancestor(node: Node<'_>) -> Option<usize> {
    let mut parent = node.parent();
    while let Some(candidate) = parent {
        if matches!(
            candidate.kind(),
            "mod_item"
                | "trait_item"
                | "struct_item"
                | "enum_item"
                | "impl_item"
                | "internal_module"
                | "module_declaration"
                | "namespace_declaration"
                | "file_scoped_namespace_declaration"
                | "class_declaration"
                | "abstract_class_declaration"
                | "interface_declaration"
                | "struct_declaration"
                | "record_declaration"
        ) {
            return Some(candidate.id());
        }
        parent = candidate.parent();
    }
    None
}

fn is_scope_kind(kind: &str, language: &str) -> bool {
    match language {
        "rust" => matches!(
            kind,
            "mod_item" | "trait_item" | "struct_item" | "enum_item" | "impl_item"
        ),
        "javascript" | "typescript" | "tsx" => matches!(
            kind,
            "internal_module"
                | "module_declaration"
                | "namespace_declaration"
                | "class_declaration"
                | "abstract_class_declaration"
                | "interface_declaration"
                | "enum_declaration"
        ),
        "csharp" => matches!(
            kind,
            "file_scoped_namespace_declaration"
                | "namespace_declaration"
                | "class_declaration"
                | "interface_declaration"
                | "enum_declaration"
                | "struct_declaration"
                | "record_declaration"
        ),
        _ => false,
    }
}

fn is_callable_kind(kind: &str, language: &str) -> bool {
    match language {
        "rust" => matches!(kind, "function_item" | "function_signature_item"),
        "javascript" | "typescript" | "tsx" => matches!(
            kind,
            "function_declaration" | "method_definition" | "method_signature"
        ),
        "csharp" => matches!(kind, "constructor_declaration" | "method_declaration"),
        _ => false,
    }
}

fn scope_node_name(node: Node<'_>, source: &[u8], language: &str) -> Option<String> {
    if node.kind() == "impl_item" && language == "rust" {
        let implemented = node
            .child_by_field_name("type")
            .and_then(|node| type_reference(node, source))?;
        return Some(
            node.child_by_field_name("trait")
                .and_then(|node| type_reference(node, source))
                .map_or_else(
                    || format!("impl {implemented}"),
                    |trait_name| format!("{implemented} as {trait_name}"),
                ),
        );
    }
    node_name(node, source)
}

fn node_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    node.child_by_field_name("name")
        .or_else(|| node.child_by_field_name("declarator"))
        .or_else(|| node.child_by_field_name("pattern"))
        .and_then(|name| type_reference(name, source))
        .map(|name| clean_identifier(&name))
        .filter(|name| !name.is_empty())
}

fn parameter_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    node.child_by_field_name("name")
        .or_else(|| node.child_by_field_name("pattern"))
        .and_then(|name| name.utf8_text(source).ok())
        .map(clean_identifier)
        .filter(|name| !name.is_empty())
}

fn type_reference(node: Node<'_>, source: &[u8]) -> Option<String> {
    if matches!(
        node.kind(),
        "identifier"
            | "type_identifier"
            | "field_identifier"
            | "property_identifier"
            | "predefined_type"
            | "primitive_type"
            | "qualified_name"
            | "scoped_type_identifier"
            | "nested_type_identifier"
    ) {
        return node
            .utf8_text(source)
            .ok()
            .map(normalize_path)
            .filter(|name| !name.is_empty());
    }
    if let Some(name) = node.child_by_field_name("name") {
        return type_reference(name, source);
    }
    if let Some(inner) = node.child_by_field_name("type") {
        return type_reference(inner, source);
    }
    direct_named_children(node)
        .into_iter()
        .find_map(|child| type_reference(child, source))
}

fn expression_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    if matches!(
        node.kind(),
        "identifier" | "type_identifier" | "member_expression" | "scoped_identifier"
    ) {
        return node
            .utf8_text(source)
            .ok()
            .map(normalize_path)
            .filter(|name| !name.is_empty());
    }
    node.child_by_field_name("function")
        .and_then(|function| expression_name(function, source))
}

fn callable_signature(node: Node<'_>, source: &[u8]) -> String {
    let parameters = node
        .child_by_field_name("parameters")
        .map(direct_named_children)
        .unwrap_or_default()
        .into_iter()
        .filter(|parameter| {
            matches!(
                parameter.kind(),
                "parameter"
                    | "required_parameter"
                    | "optional_parameter"
                    | "rest_pattern"
                    | "self_parameter"
            )
        })
        .map(|parameter| {
            parameter
                .child_by_field_name("type")
                .and_then(|node| node.utf8_text(source).ok())
                .map(normalize_signature_part)
                .unwrap_or_else(|| "_".to_owned())
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("({parameters})")
}

fn has_modifier(node: Node<'_>, modifier: &str, source: &[u8]) -> bool {
    let mut cursor = node.walk();
    node.children(&mut cursor).any(|child| {
        child.kind() == modifier
            || (child.kind().contains("modifier")
                && child
                    .utf8_text(source)
                    .is_ok_and(|text| text.trim() == modifier))
    })
}

fn has_rust_test_attribute(node: Node<'_>, source: &[u8]) -> bool {
    let mut sibling = node.prev_named_sibling();
    while let Some(attribute) = sibling {
        if attribute.kind() != "attribute_item" {
            break;
        }
        if attribute_identifiers(attribute, source)
            .into_iter()
            .any(|identifier| identifier == "test")
        {
            return true;
        }
        sibling = attribute.prev_named_sibling();
    }
    direct_named_children(node)
        .into_iter()
        .filter(|child| child.kind() == "attribute_item")
        .flat_map(|attribute| attribute_identifiers(attribute, source))
        .any(|identifier| identifier == "test")
}

fn attribute_identifiers(node: Node<'_>, source: &[u8]) -> Vec<String> {
    let mut identifiers = descendants_bounded(node, &["identifier"])
        .into_iter()
        .filter_map(|identifier| identifier.utf8_text(source).ok().map(ToOwned::to_owned))
        .collect::<Vec<_>>();
    identifiers.extend(
        direct_named_children(node)
            .into_iter()
            .filter_map(|attribute| attribute.utf8_text(source).ok())
            .map(|text| {
                text.trim()
                    .trim_start_matches("#[")
                    .trim_end_matches(']')
                    .split(['(', ':'])
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_owned()
            })
            .filter(|name| !name.is_empty()),
    );
    identifiers
}

fn ancestor_of_kind<'tree>(node: Node<'tree>, kinds: &[&str]) -> Option<Node<'tree>> {
    let mut parent = node.parent();
    while let Some(candidate) = parent {
        if kinds.contains(&candidate.kind()) {
            return Some(candidate);
        }
        parent = candidate.parent();
    }
    None
}

fn direct_named_children(node: Node<'_>) -> Vec<Node<'_>> {
    node.named_children(&mut node.walk()).collect()
}

fn descendants_bounded<'tree>(node: Node<'tree>, kinds: &[&str]) -> Vec<Node<'tree>> {
    let mut output = Vec::new();
    let mut stack = direct_named_children(node);
    while let Some(candidate) = stack.pop() {
        if kinds.contains(&candidate.kind()) {
            output.push(candidate);
        } else {
            stack.extend(direct_named_children(candidate));
        }
    }
    output.sort_by_key(Node::start_byte);
    output
}

fn qualify(scope: &[String], name: &str) -> String {
    if scope.is_empty() {
        name.to_owned()
    } else {
        format!("{}::{name}", scope.join("::"))
    }
}

fn normalize_path(raw: &str) -> String {
    let raw = raw
        .trim()
        .trim_start_matches('&')
        .trim_start_matches("mut ")
        .trim_end_matches('?')
        .trim_end_matches("[]");
    let raw = raw.split('<').next().unwrap_or(raw);
    raw.replace('.', "::")
        .split("::")
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("::")
}

fn normalize_signature_part(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn clean_identifier(raw: &str) -> String {
    raw.trim()
        .trim_matches(['"', '\'', '`'])
        .trim_start_matches('#')
        .to_owned()
}

fn compatible_label(existing: &str, requested: &str) -> bool {
    normalized_label(existing) == normalized_label(requested)
}

fn compatible_kind(existing: &str, requested: &str) -> bool {
    existing.is_empty()
        || existing == requested
        || matches!(
            (existing, requested),
            ("method", "constructor")
                | ("variable", "property" | "field" | "constant")
                | ("class", "trait")
                | ("module", "namespace")
        )
}

fn normalized_label(value: &str) -> String {
    value
        .trim()
        .trim_start_matches('.')
        .trim_end_matches("()")
        .split('(')
        .next()
        .unwrap_or_default()
        .to_owned()
}

fn record_anchor(node: &RawNodeRecord) -> Option<(usize, usize)> {
    Some((
        node.attributes
            .get("start_byte")?
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())?,
        node.attributes
            .get("end_byte")?
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())?,
    ))
}

fn edge_usize(edge: &RawEdgeRecord, key: &str) -> usize {
    edge.attributes
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or_default()
}
