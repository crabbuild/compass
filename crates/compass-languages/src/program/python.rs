use std::collections::BTreeMap;
use std::path::Path;

use compass_ir::{
    BasicBlock, Capability, Coverage, CoverageState, ExceptionEffect, ExceptionKind, ExecutionMode,
    FunctionIr, ModuleIr, Operation, OperationKind, ParameterIr, ParameterKind, ProviderDescriptor,
    SourceAnchor, Terminator, TypeRef, Visibility, hex_sha256,
};
use compass_program::{EvidenceBatch, FileInput, evidence_record};
use tree_sitter::Node;

pub(super) fn extract(
    descriptor: ProviderDescriptor,
    input: &FileInput<'_>,
    root: Node<'_>,
) -> EvidenceBatch {
    let mut collector = Collector {
        descriptor,
        input,
        functions: Vec::new(),
        evidence: Vec::new(),
    };
    collector.walk(root, None);
    collector.finish()
}

struct Collector<'a> {
    descriptor: ProviderDescriptor,
    input: &'a FileInput<'a>,
    functions: Vec<FunctionIr>,
    evidence: Vec<compass_ir::EvidenceRecord>,
}

impl Collector<'_> {
    fn walk(&mut self, node: Node<'_>, owner: Option<&str>) {
        if node.kind() == "class_definition" {
            let class_name = node
                .child_by_field_name("name")
                .map(|name| text(self.input.source, name).to_owned());
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                self.walk(child, class_name.as_deref().or(owner));
            }
            return;
        }
        if node.kind() == "function_definition" {
            self.add_function(node, owner);
            let nested_owner = node.child_by_field_name("name").map(|name| {
                let name = text(self.input.source, name);
                owner.map_or_else(|| name.to_owned(), |owner| format!("{owner}.{name}"))
            });
            if let Some(body) = node.child_by_field_name("body") {
                let mut cursor = body.walk();
                for child in body.named_children(&mut cursor) {
                    if child.kind() == "function_definition" {
                        self.walk(child, nested_owner.as_deref());
                    }
                }
            }
            return;
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.walk(child, owner);
        }
    }

    fn add_function(&mut self, node: Node<'_>, owner: Option<&str>) {
        let (Some(name_node), Some(body)) = (
            node.child_by_field_name("name"),
            node.child_by_field_name("body"),
        ) else {
            return;
        };
        let short_name = text(self.input.source, name_node);
        let name = owner.map_or_else(
            || short_name.to_owned(),
            |owner| format!("{owner}.{short_name}"),
        );
        let signature = signature_bytes(self.input.source, node);
        let symbol_id = hex_sha256(
            format!(
                "{}\0{}\0{}",
                self.input.source_file,
                name,
                hex_sha256(signature)
            )
            .as_bytes(),
        );
        let definition = evidence_record(
            &self.descriptor.id,
            Some(self.input.source_file),
            Capability::Definitions,
            format!("Python function definition {name}"),
            Some(&anchor(self.input.source_file, name_node)),
            "definition",
            &symbol_id,
        );
        self.evidence.push(definition.clone());

        let mut operations = Vec::new();
        collect_operations(
            self.input,
            body,
            &self.descriptor.id,
            &mut self.evidence,
            &mut operations,
        );
        operations.sort_by_key(|operation| operation.anchor.start_byte);
        for (ordinal, operation) in operations.iter_mut().enumerate() {
            operation.ordinal = u32::try_from(ordinal).unwrap_or(u32::MAX);
        }

        self.functions.push(FunctionIr {
            symbol_id,
            name: name.clone(),
            graph_node_id: Some(graph_node_id(self.input.source_file, &name)),
            signature_digest: hex_sha256(signature),
            body_digest: hex_sha256(slice(self.input.source, body)),
            visibility: if short_name.starts_with('_') && !short_name.starts_with("__") {
                Visibility::Private
            } else {
                Visibility::Public
            },
            execution_mode: if contains_token(signature, "async") {
                ExecutionMode::Async
            } else {
                ExecutionMode::Sync
            },
            is_test: is_test_path(self.input.source_file, short_name),
            anchor: anchor(self.input.source_file, node),
            parameters: parameters(
                self.input,
                node.child_by_field_name("parameters"),
                owner.is_some(),
                &definition.id,
            ),
            return_type: node
                .child_by_field_name("return_type")
                .map(|type_node| TypeRef {
                    spelling: text(self.input.source, type_node).to_owned(),
                    resolved_symbol: None,
                    evidence: vec![definition.id.clone()],
                }),
            blocks: vec![BasicBlock {
                id: 0,
                operations,
                terminator: Terminator::Return { value: None },
                evidence: Vec::new(),
            }],
            coverage: coverage(),
            evidence: vec![definition.id],
        });
    }

    fn finish(mut self) -> EvidenceBatch {
        self.functions
            .sort_by_key(|function| function.anchor.start_byte);
        let evidence_ids = self
            .evidence
            .iter()
            .map(|record| record.id.clone())
            .collect::<Vec<_>>();
        let coverage = coverage();
        EvidenceBatch {
            descriptor: self.descriptor,
            evidence: self.evidence,
            modules: vec![ModuleIr {
                source_file: self.input.source_file.to_owned(),
                language: "python".to_owned(),
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

fn is_test_path(source_file: &str, function_name: &str) -> bool {
    let path = source_file.replace('\\', "/").to_ascii_lowercase();
    let name = path.rsplit('/').next().unwrap_or(&path);
    path.starts_with("tests/")
        || path.contains("/tests/")
        || name.starts_with("test_")
        || name.ends_with("_test.py")
        || function_name.starts_with("test_")
}

fn parameters(
    input: &FileInput<'_>,
    list: Option<Node<'_>>,
    method: bool,
    evidence_id: &str,
) -> Vec<ParameterIr> {
    let Some(list) = list else {
        return Vec::new();
    };
    let mut separator_cursor = list.walk();
    let has_positional_separator = list
        .children(&mut separator_cursor)
        .any(|child| matches!(child.kind(), "/" | "positional_separator"));
    let mut output = Vec::new();
    let mut positional_only = has_positional_separator;
    let mut keyword_only = false;
    let mut cursor = list.walk();
    for child in list.children(&mut cursor) {
        if matches!(child.kind(), "/" | "positional_separator") {
            positional_only = false;
            continue;
        }
        if matches!(child.kind(), "*" | "keyword_separator") {
            keyword_only = true;
            continue;
        }
        if !child.is_named() {
            continue;
        }
        let default = child.child_by_field_name("value");
        let pattern = child
            .child_by_field_name("name")
            .or_else(|| child.child_by_field_name("pattern"))
            .unwrap_or(child);
        let Some(name_node) = leftmost_identifier(pattern) else {
            continue;
        };
        let name = text(input.source, name_node).to_owned();
        let kind = match child.kind() {
            "list_splat_pattern" => {
                keyword_only = true;
                ParameterKind::VariadicPositional
            }
            "dictionary_splat_pattern" => ParameterKind::VariadicKeyword,
            _ if method && output.is_empty() && matches!(name.as_str(), "self" | "cls") => {
                ParameterKind::Receiver
            }
            _ if keyword_only => ParameterKind::KeywordOnly,
            _ if positional_only => ParameterKind::PositionalOnly,
            _ => ParameterKind::PositionalOrKeyword,
        };
        let type_node = child
            .child_by_field_name("type")
            .or_else(|| pattern.child_by_field_name("type"));
        output.push(ParameterIr {
            name,
            kind,
            required: default.is_none()
                && !matches!(
                    kind,
                    ParameterKind::VariadicPositional | ParameterKind::VariadicKeyword
                ),
            default_digest: default.map(|node| hex_sha256(slice(input.source, node))),
            type_ref: type_node.map(|node| TypeRef {
                spelling: text(input.source, node).to_owned(),
                resolved_symbol: None,
                evidence: vec![evidence_id.to_owned()],
            }),
            anchor: anchor(input.source_file, child),
            evidence: vec![evidence_id.to_owned()],
        });
    }
    output
}

fn collect_operations(
    input: &FileInput<'_>,
    node: Node<'_>,
    provider_id: &str,
    evidence: &mut Vec<compass_ir::EvidenceRecord>,
    operations: &mut Vec<Operation>,
) {
    let operation = match node.kind() {
        "call" => {
            let callee = node
                .child_by_field_name("function")
                .map(|callee| text(input.source, callee).to_owned())
                .unwrap_or_default();
            let callee_node = node.child_by_field_name("function").unwrap_or(node);
            Some((
                Capability::References,
                "call",
                callee.clone(),
                OperationKind::Call {
                    callee,
                    callee_anchor: anchor(input.source_file, callee_node),
                    resolved_symbols: Vec::new(),
                    receiver_type: None,
                },
            ))
        }
        "await" => Some((
            Capability::Effects,
            "await",
            "await".to_owned(),
            OperationKind::Await,
        )),
        "raise_statement" => {
            let value = text(input.source, node)
                .trim_start_matches("raise")
                .trim()
                .to_owned();
            let effect = python_exception_effect(&value);
            Some((
                Capability::Effects,
                "throw",
                effect.display_name(),
                OperationKind::Throw { effect },
            ))
        }
        _ => None,
    };
    if let Some((capability, fact_kind, payload, kind)) = operation {
        let record = evidence_record(
            provider_id,
            Some(input.source_file),
            capability,
            format!("Python {fact_kind} {payload}"),
            Some(&anchor(input.source_file, node)),
            fact_kind,
            &payload,
        );
        evidence.push(record.clone());
        operations.push(Operation {
            ordinal: 0,
            anchor: anchor(input.source_file, node),
            evidence: vec![record.id],
            kind,
        });
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_operations(input, child, provider_id, evidence, operations);
    }
}

fn python_exception_effect(value: &str) -> ExceptionEffect {
    if value.is_empty() {
        return ExceptionEffect {
            kind: ExceptionKind::Rethrow,
            type_name: None,
            expression: None,
            chained: false,
        };
    }
    let (expression, chained) = value
        .split_once(" from ")
        .map_or((value, false), |(expression, _)| (expression.trim(), true));
    let type_name = expression
        .split_once('(')
        .map(|(candidate, _)| candidate.trim())
        .filter(|candidate| {
            !candidate.is_empty()
                && candidate.split('.').all(|part| {
                    !part.is_empty()
                        && part
                            .chars()
                            .all(|character| character == '_' || character.is_alphanumeric())
                })
        })
        .map(str::to_owned);
    if let Some(type_name) = type_name {
        ExceptionEffect {
            kind: ExceptionKind::Exception,
            type_name: Some(type_name),
            expression: None,
            chained,
        }
    } else {
        ExceptionEffect {
            kind: ExceptionKind::Dynamic,
            type_name: None,
            expression: Some(expression.to_owned()),
            chained,
        }
    }
}

fn coverage() -> Coverage {
    BTreeMap::from([
        (Capability::Syntax, CoverageState::Complete),
        (Capability::Definitions, CoverageState::Complete),
        (Capability::Contracts, CoverageState::Complete),
        (
            Capability::SymbolIdentity,
            CoverageState::Partial {
                reasons: vec!["compiler_symbol_identity_unavailable".to_owned()],
            },
        ),
        (
            Capability::References,
            CoverageState::Partial {
                reasons: vec!["compiler_references_unavailable".to_owned()],
            },
        ),
        (
            Capability::Types,
            CoverageState::Partial {
                reasons: vec!["compiler_types_unavailable".to_owned()],
            },
        ),
        (
            Capability::CallResolution,
            CoverageState::Partial {
                reasons: vec!["dynamic_call_resolution".to_owned()],
            },
        ),
        (
            Capability::ControlFlow,
            CoverageState::Partial {
                reasons: vec!["branch_complete_cfg_unavailable".to_owned()],
            },
        ),
        (
            Capability::DataFlow,
            CoverageState::Indeterminate {
                reasons: vec!["data_flow_unavailable".to_owned()],
            },
        ),
        (
            Capability::Effects,
            CoverageState::Partial {
                reasons: vec!["interprocedural_effects_unavailable".to_owned()],
            },
        ),
    ])
}

fn graph_node_id(path: &str, name: &str) -> String {
    let stem = crate::file_stem(Path::new(path));
    name.rsplit_once('.').map_or_else(
        || crate::make_id(&[&stem, name]),
        |(owner, function)| {
            let parent = crate::make_id(&[&stem, owner]);
            crate::make_id(&[&parent, function])
        },
    )
}

fn leftmost_identifier(node: Node<'_>) -> Option<Node<'_>> {
    if node.kind() == "identifier" {
        return Some(node);
    }
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find_map(leftmost_identifier)
}

fn contains_token(source: &[u8], expected: &str) -> bool {
    std::str::from_utf8(source)
        .unwrap_or_default()
        .split(|character: char| !character.is_alphanumeric() && character != '_')
        .any(|token| token == expected)
}

fn signature_bytes<'a>(source: &'a [u8], node: Node<'_>) -> &'a [u8] {
    let end = node
        .child_by_field_name("body")
        .map_or(node.end_byte(), |body| body.start_byte());
    source.get(node.start_byte()..end).unwrap_or_default()
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
