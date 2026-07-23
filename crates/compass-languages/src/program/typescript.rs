use std::collections::{BTreeMap, HashMap};

use compass_ir::{
    BasicBlock, Capability, Coverage, CoverageState, FunctionIr, ModuleIr, Operation,
    OperationKind, ParameterIr, ProviderDescriptor, SourceAnchor, Terminator, TypeRef, hex_sha256,
};
use compass_program::{EvidenceBatch, FileInput, evidence_record};
use tree_sitter::Node;

pub(super) fn extract(
    descriptor: ProviderDescriptor,
    input: &FileInput<'_>,
    root: Node<'_>,
) -> EvidenceBatch {
    let mut collector = Collector::new(descriptor, input);
    collector.scan_reasons(root);
    collector.precollect(root, None);
    collector.walk(root, None);
    collector.finish()
}

struct Collector<'a> {
    descriptor: ProviderDescriptor,
    input: &'a FileInput<'a>,
    definitions: HashMap<String, Vec<String>>,
    definition_occurrences: HashMap<String, u32>,
    function_occurrences: HashMap<String, u32>,
    functions: Vec<FunctionIr>,
    evidence: Vec<compass_ir::EvidenceRecord>,
    reasons: BTreeMap<Capability, Vec<String>>,
}

impl<'a> Collector<'a> {
    fn new(descriptor: ProviderDescriptor, input: &'a FileInput<'a>) -> Self {
        Self {
            descriptor,
            input,
            definitions: HashMap::new(),
            definition_occurrences: HashMap::new(),
            function_occurrences: HashMap::new(),
            functions: Vec::new(),
            evidence: Vec::new(),
            reasons: BTreeMap::new(),
        }
    }

    fn scan_reasons(&mut self, node: Node<'_>) {
        match node.kind() {
            "import_statement" => add_reason(
                &mut self.reasons,
                Capability::CallResolution,
                "import_resolution_unavailable",
            ),
            "decorator" => add_reason(
                &mut self.reasons,
                Capability::Effects,
                "decorator_semantics",
            ),
            "subscript_expression" => add_reason(
                &mut self.reasons,
                Capability::CallResolution,
                "dynamic_property_access",
            ),
            "if_statement" | "switch_statement" | "for_statement" | "while_statement"
            | "try_statement" => add_reason(
                &mut self.reasons,
                Capability::ControlFlow,
                "branch_sensitive_cfg",
            ),
            "jsx_element" | "jsx_self_closing_element" => add_reason(
                &mut self.reasons,
                Capability::CallResolution,
                "jsx_framework_dispatch",
            ),
            _ => {}
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.scan_reasons(child);
        }
    }

    fn precollect(&mut self, node: Node<'_>, owner: Option<&str>) {
        let next_owner = if matches!(node.kind(), "class_declaration" | "class") {
            node.child_by_field_name("name")
                .map(|name| text(self.input.source, name).to_owned())
        } else {
            owner.map(str::to_owned)
        };
        if let Some((name, signature_node)) =
            function_name(self.input.source, node, next_owner.as_deref())
        {
            let base = symbol_id(
                self.input.source_file,
                &name,
                signature_bytes(self.input.source, signature_node),
            );
            let symbol = unique_symbol_id(base, &mut self.definition_occurrences);
            let short = name.rsplit('.').next().unwrap_or(&name);
            self.definitions
                .entry(short.to_owned())
                .or_default()
                .push(symbol);
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.precollect(child, next_owner.as_deref());
        }
    }

    fn walk(&mut self, node: Node<'_>, owner: Option<&str>) {
        let next_owner = if matches!(node.kind(), "class_declaration" | "class") {
            node.child_by_field_name("name")
                .map(|name| text(self.input.source, name).to_owned())
        } else {
            owner.map(str::to_owned)
        };
        if let Some((name, container, function)) =
            function_parts(self.input.source, node, next_owner.as_deref())
        {
            self.add_function(name, container, function);
            return;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk(child, next_owner.as_deref());
        }
    }

    fn add_function(&mut self, name: String, container: Node<'_>, function: Node<'_>) {
        let Some(body) = function.child_by_field_name("body") else {
            return;
        };
        let signature = signature_bytes(self.input.source, function);
        let base = symbol_id(self.input.source_file, &name, signature);
        let symbol = unique_symbol_id(base, &mut self.function_occurrences);
        let name_anchor = function
            .child_by_field_name("name")
            .or_else(|| container.child_by_field_name("name"))
            .unwrap_or(function);
        let definition = evidence_record(
            &self.descriptor.id,
            Some(self.input.source_file),
            Capability::Definitions,
            format!("{} function definition {name}", self.input.language),
            Some(&anchor(self.input.source_file, name_anchor)),
            "definition",
            &symbol,
        );
        self.evidence.push(definition.clone());
        let mut operations = Vec::new();
        collect_operations(
            self.input,
            body,
            &self.descriptor.id,
            &self.definitions,
            &mut self.evidence,
            &mut operations,
            &mut self.reasons,
        );
        operations.sort_by_key(|operation| operation.anchor.start_byte);
        for (ordinal, operation) in operations.iter_mut().enumerate() {
            operation.ordinal = u32::try_from(ordinal).unwrap_or(u32::MAX);
        }
        let coverage = coverage(&self.reasons);
        self.functions.push(FunctionIr {
            symbol_id: symbol,
            name,
            graph_node_id: None,
            signature_digest: hex_sha256(signature),
            body_digest: hex_sha256(slice(self.input.source, body)),
            anchor: anchor(self.input.source_file, container),
            parameters: parameters(
                self.input,
                function.child_by_field_name("parameters"),
                &definition.id,
            ),
            return_type: function
                .child_by_field_name("return_type")
                .map(|node| TypeRef {
                    spelling: text(self.input.source, node)
                        .trim_start_matches(':')
                        .trim()
                        .to_owned(),
                    resolved_symbol: None,
                    evidence: vec![definition.id.clone()],
                }),
            blocks: vec![BasicBlock {
                id: 0,
                operations,
                terminator: Terminator::Return { value: None },
                evidence: Vec::new(),
            }],
            coverage,
            evidence: vec![definition.id],
        });
    }

    fn finish(mut self) -> EvidenceBatch {
        self.functions
            .sort_by_key(|function| function.anchor.start_byte);
        let coverage = coverage(&self.reasons);
        let evidence_ids = self
            .evidence
            .iter()
            .map(|record| record.id.clone())
            .collect();
        EvidenceBatch {
            descriptor: self.descriptor,
            evidence: self.evidence,
            modules: vec![ModuleIr {
                source_file: self.input.source_file.to_owned(),
                language: self.input.language.to_owned(),
                source_digest: hex_sha256(self.input.source),
                graph_node_id: Some(crate::make_id(&[self.input.source_file])),
                functions: self.functions,
                coverage: coverage.clone(),
                evidence: evidence_ids,
            }],
            facts: Vec::new(),
            coverage: BTreeMap::from([(self.input.source_file.to_owned(), coverage)]),
        }
    }
}

fn function_parts<'tree>(
    source: &[u8],
    node: Node<'tree>,
    owner: Option<&str>,
) -> Option<(String, Node<'tree>, Node<'tree>)> {
    match node.kind() {
        "function_declaration" | "generator_function_declaration" => {
            let name = text(source, node.child_by_field_name("name")?).to_owned();
            Some((name, node, node))
        }
        "method_definition" | "method_signature" => {
            let name = text(source, node.child_by_field_name("name")?);
            Some((
                owner.map_or_else(|| name.to_owned(), |owner| format!("{owner}.{name}")),
                node,
                node,
            ))
        }
        "variable_declarator" => {
            let value = node.child_by_field_name("value")?;
            if !matches!(
                value.kind(),
                "arrow_function" | "function_expression" | "generator_function"
            ) {
                return None;
            }
            let name = text(source, node.child_by_field_name("name")?).to_owned();
            Some((name, node, value))
        }
        _ => None,
    }
}

fn function_name<'tree>(
    source: &[u8],
    node: Node<'tree>,
    owner: Option<&str>,
) -> Option<(String, Node<'tree>)> {
    function_parts(source, node, owner).map(|(name, _, function)| (name, function))
}

fn collect_operations(
    input: &FileInput<'_>,
    node: Node<'_>,
    provider_id: &str,
    definitions: &HashMap<String, Vec<String>>,
    evidence: &mut Vec<compass_ir::EvidenceRecord>,
    operations: &mut Vec<Operation>,
    reasons: &mut BTreeMap<Capability, Vec<String>>,
) {
    match node.kind() {
        "call_expression" | "new_expression" => {
            if let Some(function) = node
                .child_by_field_name("function")
                .or_else(|| node.child_by_field_name("constructor"))
            {
                let callee_node = rightmost_identifier(function).unwrap_or(function);
                let callee = text(input.source, callee_node);
                let callee_anchor = anchor(input.source_file, callee_node);
                let record = evidence_record(
                    provider_id,
                    Some(input.source_file),
                    Capability::Syntax,
                    format!("{} call {callee}", input.language),
                    Some(&callee_anchor),
                    "call",
                    callee,
                );
                evidence.push(record.clone());
                let mut operation_evidence = vec![record.id];
                let mut resolved_symbols = Vec::new();
                if function.kind() == "identifier"
                    && let Some(candidates) = definitions.get(callee)
                    && candidates.len() == 1
                {
                    let resolution = evidence_record(
                        provider_id,
                        Some(input.source_file),
                        Capability::CallResolution,
                        format!("unique same-module target {}", candidates[0]),
                        Some(&callee_anchor),
                        "call_resolution",
                        &candidates[0],
                    );
                    operation_evidence.push(resolution.id.clone());
                    evidence.push(resolution);
                    resolved_symbols.push(candidates[0].clone());
                }
                if matches!(callee, "eval" | "Function") {
                    add_reason(reasons, Capability::Effects, "eval_or_function_constructor");
                }
                operations.push(Operation {
                    ordinal: 0,
                    anchor: anchor(input.source_file, node),
                    evidence: operation_evidence,
                    kind: OperationKind::Call {
                        callee: callee.to_owned(),
                        callee_anchor,
                        resolved_symbols,
                        receiver_type: None,
                    },
                });
            }
        }
        "assignment_expression" => {
            if let Some(left) = node.child_by_field_name("left") {
                push_path(input, provider_id, left, true, evidence, operations);
            }
        }
        "member_expression" => {
            if node.parent().is_none_or(|parent| {
                parent.kind() != "assignment_expression" && parent.kind() != "call_expression"
            }) {
                push_path(input, provider_id, node, false, evidence, operations);
            }
        }
        "await_expression" => push_unit(
            input,
            provider_id,
            node,
            "await",
            OperationKind::Await,
            evidence,
            operations,
        ),
        "throw_statement" => push_unit(
            input,
            provider_id,
            node,
            "throw",
            OperationKind::Throw {
                value: text(input.source, node).to_owned(),
            },
            evidence,
            operations,
        ),
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_operations(
            input,
            child,
            provider_id,
            definitions,
            evidence,
            operations,
            reasons,
        );
    }
}

fn push_path(
    input: &FileInput<'_>,
    provider_id: &str,
    node: Node<'_>,
    write: bool,
    evidence: &mut Vec<compass_ir::EvidenceRecord>,
    operations: &mut Vec<Operation>,
) {
    let path = text(input.source, node);
    let capability = if write {
        Capability::Effects
    } else {
        Capability::References
    };
    let record = evidence_record(
        provider_id,
        Some(input.source_file),
        capability,
        format!(
            "{} {} {path}",
            input.language,
            if write { "write" } else { "read" }
        ),
        Some(&anchor(input.source_file, node)),
        if write { "write" } else { "read" },
        path,
    );
    evidence.push(record.clone());
    operations.push(Operation {
        ordinal: 0,
        anchor: anchor(input.source_file, node),
        evidence: vec![record.id],
        kind: if write {
            OperationKind::Write {
                path: path.to_owned(),
            }
        } else {
            OperationKind::Read {
                path: path.to_owned(),
            }
        },
    });
}

fn push_unit(
    input: &FileInput<'_>,
    provider_id: &str,
    node: Node<'_>,
    fact: &str,
    kind: OperationKind,
    evidence: &mut Vec<compass_ir::EvidenceRecord>,
    operations: &mut Vec<Operation>,
) {
    let record = evidence_record(
        provider_id,
        Some(input.source_file),
        Capability::Effects,
        format!("{} {fact}", input.language),
        Some(&anchor(input.source_file, node)),
        fact,
        fact,
    );
    evidence.push(record.clone());
    operations.push(Operation {
        ordinal: 0,
        anchor: anchor(input.source_file, node),
        evidence: vec![record.id],
        kind,
    });
}

fn parameters(
    input: &FileInput<'_>,
    list: Option<Node<'_>>,
    evidence_id: &str,
) -> Vec<ParameterIr> {
    let Some(list) = list else {
        return Vec::new();
    };
    let mut output = Vec::new();
    let mut cursor = list.walk();
    for parameter in list.children(&mut cursor).filter(|node| node.is_named()) {
        if !matches!(
            parameter.kind(),
            "required_parameter" | "optional_parameter" | "rest_pattern" | "identifier"
        ) {
            continue;
        }
        let name_node = parameter
            .child_by_field_name("pattern")
            .or_else(|| parameter.child_by_field_name("name"))
            .unwrap_or(parameter);
        output.push(ParameterIr {
            name: text(input.source, name_node).to_owned(),
            type_ref: parameter.child_by_field_name("type").map(|node| TypeRef {
                spelling: text(input.source, node)
                    .trim_start_matches(':')
                    .trim()
                    .to_owned(),
                resolved_symbol: None,
                evidence: vec![evidence_id.to_owned()],
            }),
            anchor: anchor(input.source_file, parameter),
            evidence: vec![evidence_id.to_owned()],
        });
    }
    output
}

fn coverage(reasons: &BTreeMap<Capability, Vec<String>>) -> Coverage {
    let mut coverage = Coverage::new();
    coverage.insert(Capability::Syntax, CoverageState::Complete);
    coverage.insert(Capability::Definitions, CoverageState::Complete);
    for capability in [
        Capability::SymbolIdentity,
        Capability::References,
        Capability::CallResolution,
        Capability::ControlFlow,
        Capability::Effects,
    ] {
        let defaults = match capability {
            Capability::SymbolIdentity => "compiler_symbol_identity_unavailable",
            Capability::References => "compiler_references_unavailable",
            Capability::CallResolution => "compiler_call_resolution_unavailable",
            Capability::ControlFlow => "branch_complete_cfg_unavailable",
            Capability::Effects => "interprocedural_effects_unavailable",
            _ => "unavailable",
        };
        coverage.insert(
            capability.clone(),
            CoverageState::Partial {
                reasons: reasons
                    .get(&capability)
                    .cloned()
                    .unwrap_or_else(|| vec![defaults.to_owned()]),
            },
        );
    }
    for capability in [
        Capability::Types,
        Capability::DataFlow,
        Capability::Contracts,
    ] {
        let reason = match capability {
            Capability::Types => "compiler_types_unavailable",
            Capability::DataFlow => "data_flow_unavailable",
            Capability::Contracts => "contract_analysis_unavailable",
            _ => "unavailable",
        };
        coverage.insert(
            capability,
            CoverageState::Unavailable {
                reasons: vec![reason.to_owned()],
            },
        );
    }
    coverage
}

fn add_reason(
    reasons: &mut BTreeMap<Capability, Vec<String>>,
    capability: Capability,
    reason: &str,
) {
    reasons
        .entry(capability)
        .or_default()
        .push(reason.to_owned());
}

fn symbol_id(path: &str, name: &str, signature: &[u8]) -> String {
    hex_sha256(format!("{path}\0{name}\0{}", hex_sha256(signature)).as_bytes())
}

fn unique_symbol_id(base: String, occurrences: &mut HashMap<String, u32>) -> String {
    let occurrence = occurrences.entry(base.clone()).or_default();
    let symbol = if *occurrence == 0 {
        base.clone()
    } else {
        hex_sha256(format!("{base}\0{occurrence}").as_bytes())
    };
    *occurrence = occurrence.saturating_add(1);
    symbol
}

fn signature_bytes<'a>(source: &'a [u8], node: Node<'_>) -> &'a [u8] {
    let end = node
        .child_by_field_name("body")
        .map_or(node.end_byte(), |body| body.start_byte());
    source.get(node.start_byte()..end).unwrap_or_default()
}

fn rightmost_identifier(node: Node<'_>) -> Option<Node<'_>> {
    if matches!(
        node.kind(),
        "identifier" | "property_identifier" | "private_property_identifier"
    ) {
        return Some(node);
    }
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .filter_map(rightmost_identifier)
        .last()
}

fn anchor(path: &str, node: Node<'_>) -> SourceAnchor {
    SourceAnchor {
        source_file: path.to_owned(),
        start_byte: u64::try_from(node.start_byte()).unwrap_or(u64::MAX),
        end_byte: u64::try_from(node.end_byte()).unwrap_or(u64::MAX),
    }
}

fn text<'a>(source: &'a [u8], node: Node<'_>) -> &'a str {
    std::str::from_utf8(slice(source, node)).unwrap_or_default()
}

fn slice<'a>(source: &'a [u8], node: Node<'_>) -> &'a [u8] {
    source
        .get(node.start_byte()..node.end_byte())
        .unwrap_or_default()
}
