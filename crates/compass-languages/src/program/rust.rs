use std::collections::{BTreeMap, HashMap};
use std::path::Path;

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
    collector.precollect(root, None);
    collector.walk(root, None);
    collector.finish()
}

struct Collector<'a> {
    descriptor: ProviderDescriptor,
    input: &'a FileInput<'a>,
    definitions: HashMap<String, Vec<String>>,
    functions: Vec<FunctionIr>,
    evidence: Vec<compass_ir::EvidenceRecord>,
    coverage_reasons: BTreeMap<Capability, Vec<String>>,
}

impl<'a> Collector<'a> {
    fn new(descriptor: ProviderDescriptor, input: &'a FileInput<'a>) -> Self {
        Self {
            descriptor,
            input,
            definitions: HashMap::new(),
            functions: Vec::new(),
            evidence: Vec::new(),
            coverage_reasons: BTreeMap::new(),
        }
    }

    fn precollect(&mut self, node: Node<'_>, owner: Option<&str>) {
        match node.kind() {
            "impl_item" => {
                let owner = impl_owner(self.input.source, node);
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    self.precollect(child, owner.as_deref());
                }
                return;
            }
            "function_item" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = text(self.input.source, name_node);
                    let symbol = symbol_id(
                        self.input.source_file,
                        owner,
                        name,
                        signature_bytes(self.input.source, node),
                    );
                    self.definitions
                        .entry(name.to_owned())
                        .or_default()
                        .push(symbol);
                }
                return;
            }
            _ => {}
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.precollect(child, owner);
        }
    }

    fn walk(&mut self, node: Node<'_>, owner: Option<&str>) {
        match node.kind() {
            "impl_item" => {
                let owner = impl_owner(self.input.source, node);
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    self.walk(child, owner.as_deref());
                }
                return;
            }
            "function_item" => {
                self.add_function(node, owner);
                return;
            }
            _ => {}
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk(child, owner);
        }
    }

    fn add_function(&mut self, node: Node<'_>, owner: Option<&str>) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let Some(body) = node.child_by_field_name("body") else {
            return;
        };
        let name = text(self.input.source, name_node);
        if owner.is_some_and(|owner| owner.starts_with('<') && owner.ends_with('>')) {
            add_reason(
                &mut self.coverage_reasons,
                Capability::SymbolIdentity,
                "graph_identity_collision",
            );
        }
        let display_name = owner.map_or_else(
            || name.to_owned(),
            |owner| format!("{owner}.{name}"),
        );
        let signature = signature_bytes(self.input.source, node);
        let symbol = symbol_id(self.input.source_file, owner, name, signature);
        let function_anchor = anchor(self.input.source_file, node);
        let definition = evidence_record(
            &self.descriptor.id,
            Some(self.input.source_file),
            Capability::Definitions,
            format!("Rust function definition {display_name}"),
            Some(&anchor(self.input.source_file, name_node)),
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
            &mut self.coverage_reasons,
        );
        operations.sort_by_key(|operation| operation.anchor.start_byte);
        for (ordinal, operation) in operations.iter_mut().enumerate() {
            operation.ordinal = u32::try_from(ordinal).unwrap_or(u32::MAX);
        }
        let parameters = parameters(
            self.input,
            node.child_by_field_name("parameters"),
            &definition.id,
        );
        let return_type = node
            .child_by_field_name("return_type")
            .or_else(|| child_kind(node, "type_identifier"))
            .map(|node| TypeRef {
                spelling: text(self.input.source, node).to_owned(),
                resolved_symbol: None,
                evidence: vec![definition.id.clone()],
            });
        let coverage = function_coverage(&self.coverage_reasons);
        self.functions.push(FunctionIr {
            symbol_id: symbol,
            name: display_name,
            graph_node_id: Some(graph_node_id(self.input.source_file, owner, name)),
            signature_digest: hex_sha256(signature),
            body_digest: hex_sha256(slice(self.input.source, body)),
            anchor: function_anchor,
            parameters,
            return_type,
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
        self.functions.sort_by_key(|function| function.anchor.start_byte);
        let source_digest = hex_sha256(self.input.source);
        let coverage = function_coverage(&self.coverage_reasons);
        let evidence_ids = self
            .evidence
            .iter()
            .map(|record| record.id.clone())
            .collect::<Vec<_>>();
        EvidenceBatch {
            descriptor: self.descriptor,
            evidence: self.evidence,
            modules: vec![ModuleIr {
                source_file: self.input.source_file.to_owned(),
                language: "rust".to_owned(),
                source_digest,
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
        "call_expression" => {
            if let Some(function) = node.child_by_field_name("function") {
                let callee_node = rightmost_identifier(function).unwrap_or(function);
                let callee = text(input.source, callee_node);
                let call_anchor = anchor(input.source_file, node);
                let callee_anchor = anchor(input.source_file, callee_node);
                let syntax = evidence_record(
                    provider_id,
                    Some(input.source_file),
                    Capability::Syntax,
                    format!("Rust call {callee}"),
                    Some(&callee_anchor),
                    "call",
                    callee,
                );
                evidence.push(syntax.clone());
                let mut operation_evidence = vec![syntax.id];
                let mut resolved_symbols = Vec::new();
                if function.kind() == "identifier"
                    && let Some(candidates) = definitions.get(callee)
                    && candidates.len() == 1
                {
                    let resolution = evidence_record(
                        provider_id,
                        Some(input.source_file),
                        Capability::CallResolution,
                        format!("unique same-module Rust target {}", candidates[0]),
                        Some(&callee_anchor),
                        "call_resolution",
                        &candidates[0],
                    );
                    operation_evidence.push(resolution.id.clone());
                    evidence.push(resolution);
                    resolved_symbols.push(candidates[0].clone());
                } else {
                    add_reason(
                        reasons,
                        Capability::CallResolution,
                        "trait_dispatch_unresolved",
                    );
                }
                operations.push(Operation {
                    ordinal: 0,
                    anchor: call_anchor,
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
                push_path_operation(
                    input,
                    provider_id,
                    left,
                    Capability::Effects,
                    true,
                    evidence,
                    operations,
                );
            }
        }
        "field_expression" => {
            if node
                .parent()
                .is_none_or(|parent| parent.kind() != "assignment_expression")
            {
                push_path_operation(
                    input,
                    provider_id,
                    node,
                    Capability::References,
                    false,
                    evidence,
                    operations,
                );
            }
        }
        "await_expression" => push_unit_operation(
            input,
            provider_id,
            node,
            "await",
            OperationKind::Await,
            evidence,
            operations,
        ),
        "macro_invocation" => {
            let value = text(input.source, node);
            if value.starts_with("panic!")
                || value.starts_with("bail!")
                || value.starts_with("ensure!")
            {
                push_unit_operation(
                    input,
                    provider_id,
                    node,
                    "throw",
                    OperationKind::Throw {
                        value: value.to_owned(),
                    },
                    evidence,
                    operations,
                );
            } else {
                add_reason(
                    reasons,
                    Capability::Effects,
                    "macro_expansion_unavailable",
                );
            }
        }
        "try_expression" => add_reason(
            reasons,
            Capability::ControlFlow,
            "question_mark_control_flow",
        ),
        "if_expression" | "match_expression" | "loop_expression" | "while_expression"
        | "for_expression" => add_reason(
            reasons,
            Capability::ControlFlow,
            "branch_sensitive_cfg",
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

fn push_path_operation(
    input: &FileInput<'_>,
    provider_id: &str,
    node: Node<'_>,
    capability: Capability,
    write: bool,
    evidence: &mut Vec<compass_ir::EvidenceRecord>,
    operations: &mut Vec<Operation>,
) {
    let path = text(input.source, node);
    let record = evidence_record(
        provider_id,
        Some(input.source_file),
        capability,
        if write {
            format!("Rust write {path}")
        } else {
            format!("Rust read {path}")
        },
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

fn push_unit_operation(
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
        format!("Rust {fact}"),
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
        if !matches!(parameter.kind(), "parameter" | "self_parameter") {
            continue;
        }
        let name_node = parameter
            .child_by_field_name("pattern")
            .or_else(|| parameter.child_by_field_name("name"))
            .unwrap_or(parameter);
        let type_ref = parameter.child_by_field_name("type").map(|node| TypeRef {
            spelling: text(input.source, node).to_owned(),
            resolved_symbol: None,
            evidence: vec![evidence_id.to_owned()],
        });
        output.push(ParameterIr {
            name: text(input.source, name_node).to_owned(),
            type_ref,
            anchor: anchor(input.source_file, parameter),
            evidence: vec![evidence_id.to_owned()],
        });
    }
    output
}

fn function_coverage(reasons: &BTreeMap<Capability, Vec<String>>) -> Coverage {
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
        let reasons = reasons.get(&capability).cloned().unwrap_or_else(|| {
            vec![match capability {
                Capability::SymbolIdentity => "compiler_symbol_identity_unavailable",
                Capability::References => "compiler_references_unavailable",
                Capability::CallResolution => "compiler_call_resolution_unavailable",
                Capability::ControlFlow => "branch_complete_cfg_unavailable",
                Capability::Effects => "interprocedural_effects_unavailable",
                _ => "unavailable",
            }
            .to_owned()]
        });
        coverage.insert(capability, CoverageState::Partial { reasons });
    }
    for capability in [Capability::Types, Capability::DataFlow, Capability::Contracts] {
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

fn impl_owner(source: &[u8], node: Node<'_>) -> Option<String> {
    let type_node = node.child_by_field_name("type")?;
    let owner = text(source, type_node);
    let trait_name = node.child_by_field_name("trait").map(|node| text(source, node));
    Some(trait_name.map_or_else(
        || owner.to_owned(),
        |trait_name| format!("<{owner} as {trait_name}>"),
    ))
}

fn graph_node_id(path: &str, owner: Option<&str>, name: &str) -> String {
    owner.map_or_else(
        || crate::make_id(&[&crate::file_stem(Path::new(path)), name]),
        |owner| {
            let graph_owner = owner
                .strip_prefix('<')
                .and_then(|owner| owner.strip_suffix('>'))
                .and_then(|owner| owner.split_once(" as "))
                .map_or(owner, |(owner, _)| owner);
            let parent = crate::make_id(&[&crate::file_stem(Path::new(path)), graph_owner]);
            crate::make_id(&[&parent, name])
        },
    )
}

fn symbol_id(path: &str, owner: Option<&str>, name: &str, signature: &[u8]) -> String {
    hex_sha256(format!("{path}\0{}\0{name}\0{}", owner.unwrap_or_default(), hex_sha256(signature)).as_bytes())
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
        "identifier" | "field_identifier" | "type_identifier"
    ) {
        return Some(node);
    }
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .filter_map(rightmost_identifier)
        .last()
}

fn child_kind<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    node.children(&mut cursor).find(|child| child.kind() == kind)
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
