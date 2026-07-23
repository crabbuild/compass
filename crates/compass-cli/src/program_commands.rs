use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use compass_analysis::{AnalysisBundle, FunctionSummary};
use compass_ir::{Capability, CoverageState, FunctionIr, Operation, OperationKind};
use compass_model::{EdgeRecord, Graph, GraphDocument, NodeRecord};
use serde_json::{Map, Value, json};

use super::{Frontend, Outcome};

const MAX_PROGRAM_BYTES: u64 = 2 * 1024 * 1024 * 1024;

pub(super) fn command(frontend: Frontend, args: &[String]) -> Outcome {
    if frontend != Frontend::Compass {
        return Outcome::failure(
            "error: program inspection is unavailable in Graphify compatibility mode".to_owned(),
        );
    }
    let Some(subcommand) = args.first().map(String::as_str) else {
        return Outcome::success(help());
    };
    if matches!(subcommand, "-h" | "--help" | "help") {
        return Outcome::success(help());
    }
    let (options, remaining) = match parse_common(&args[1..], subcommand != "query") {
        Ok(parsed) => parsed,
        Err(error) => return Outcome::failure_with_code(format!("error: {error}"), 2),
    };
    let analysis = match load_program(&options.program) {
        Ok(analysis) => analysis,
        Err(error) => return Outcome::failure_with_code(format!("error: {error}"), 3),
    };
    match subcommand {
        "summary" => render(summary(&analysis), options.format),
        "coverage" => coverage(&analysis, &remaining, options.format),
        "functions" => functions(&analysis, &remaining, options.format),
        "show" => show(&analysis, &remaining, options.format),
        "callers" => callers(&analysis, &remaining, options.format),
        "explain-call" => explain_call(&analysis, &remaining, options.format),
        "query" => query(&analysis, &remaining),
        unknown => Outcome::failure_with_code(
            format!("error: unknown program command '{unknown}'\n{}", help()),
            2,
        ),
    }
}

#[derive(Clone, Copy)]
enum Format {
    Text,
    Json,
}

struct CommonOptions {
    program: PathBuf,
    format: Format,
}

fn default_program_path() -> PathBuf {
    PathBuf::from(std::env::var("COMPASS_OUT").unwrap_or_else(|_| "compass-out".to_owned()))
        .join("program.json")
}

fn parse_common(
    args: &[String],
    consume_format: bool,
) -> Result<(CommonOptions, Vec<String>), String> {
    let mut program = default_program_path();
    let mut format = Format::Text;
    let mut remaining = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--program" => {
                index += 1;
                let value = args.get(index).ok_or("--program requires a path")?;
                if value.is_empty() {
                    return Err("--program requires a path".to_owned());
                }
                program = value.into();
            }
            "--format" if consume_format => {
                index += 1;
                format = parse_format(args.get(index).ok_or("--format requires text or json")?)?;
            }
            value if value.starts_with("--program=") => {
                let value = &value[10..];
                if value.is_empty() {
                    return Err("--program requires a path".to_owned());
                }
                program = value.into();
            }
            value if consume_format && value.starts_with("--format=") => {
                format = parse_format(&value[9..])?;
            }
            value => remaining.push(value.to_owned()),
        }
        index += 1;
    }
    Ok((CommonOptions { program, format }, remaining))
}

fn parse_format(value: &str) -> Result<Format, String> {
    match value {
        "text" | "table" => Ok(Format::Text),
        "json" => Ok(Format::Json),
        _ => Err("--format must be text or json".to_owned()),
    }
}

fn load_program(path: &Path) -> Result<AnalysisBundle, String> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    if !metadata.is_file() {
        return Err(format!(
            "Program IR is not a regular file: {}",
            path.display()
        ));
    }
    if metadata.len() > MAX_PROGRAM_BYTES {
        return Err(format!(
            "Program IR exceeds the {MAX_PROGRAM_BYTES}-byte safety limit"
        ));
    }
    let bytes =
        fs::read(path).map_err(|error| format!("could not read {}: {error}", path.display()))?;
    let analysis: AnalysisBundle = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid Program IR JSON at {}: {error}", path.display()))?;
    analysis
        .validate()
        .map_err(|error| format!("invalid Program IR at {}: {error}", path.display()))?;
    let canonical = analysis
        .canonical_bytes()
        .map_err(|error| format!("invalid Program IR at {}: {error}", path.display()))?;
    if canonical != bytes {
        return Err(format!("Program IR is not canonical: {}", path.display()));
    }
    Ok(analysis)
}

fn render(value: Value, format: Format) -> Outcome {
    match format {
        Format::Json => match serde_json::to_string_pretty(&value) {
            Ok(output) => Outcome::success(output),
            Err(error) => Outcome::failure(format!("error: could not render JSON: {error}")),
        },
        Format::Text => Outcome::success(render_text(&value)),
    }
}

fn render_text(value: &Value) -> String {
    match value {
        Value::Array(values) => {
            if values.is_empty() {
                return "No results.".to_owned();
            }
            values
                .iter()
                .map(render_text_record)
                .collect::<Vec<_>>()
                .join("\n")
        }
        Value::Object(_) => render_text_record(value),
        _ => value.to_string(),
    }
}

fn render_text_record(value: &Value) -> String {
    let Value::Object(values) = value else {
        return value.to_string();
    };
    values
        .iter()
        .map(|(key, value)| format!("{key}: {}", compact_json(value)))
        .collect::<Vec<_>>()
        .join("  ")
}

fn compact_json(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        _ => value.to_string(),
    }
}

fn summary(analysis: &AnalysisBundle) -> Value {
    let operations = analysis
        .program
        .modules
        .iter()
        .flat_map(|module| &module.functions)
        .flat_map(|function| &function.blocks)
        .flat_map(|block| &block.operations)
        .count();
    let calls = analysis
        .program
        .modules
        .iter()
        .flat_map(|module| &module.functions)
        .flat_map(|function| &function.blocks)
        .flat_map(|block| &block.operations)
        .filter(|operation| matches!(operation.kind, OperationKind::Call { .. }))
        .count();
    let unresolved = analysis
        .summaries
        .iter()
        .map(|summary| summary.unresolved_calls.len())
        .sum::<usize>();
    let impact_complete = analysis
        .program
        .modules
        .iter()
        .flat_map(|module| &module.functions)
        .filter(|function| impact_eligible(function, summary_for(analysis, &function.symbol_id)))
        .count();
    json!({
        "schema": analysis.program.schema,
        "providers": analysis.program.providers.len(),
        "evidence_records": analysis.program.evidence.len(),
        "modules": analysis.program.modules.len(),
        "functions": analysis.summaries.len(),
        "operations": operations,
        "calls": calls,
        "unresolved_calls": unresolved,
        "resolved_call_edges": analysis.reverse_calls.values().map(Vec::len).sum::<usize>(),
        "impact_complete_functions": impact_complete,
        "impact_policy": "resolved edges are positive evidence; unresolved calls never prove absence",
    })
}

fn coverage(analysis: &AnalysisBundle, args: &[String], format: Format) -> Outcome {
    if !args.is_empty() {
        return usage_error("coverage accepts only --program and --format");
    }
    let mut counts = BTreeMap::<(String, String), u64>::new();
    let mut reasons = BTreeMap::<(String, String, String), u64>::new();
    for coverage in analysis.program.modules.iter().flat_map(|module| {
        std::iter::once(&module.coverage)
            .chain(module.functions.iter().map(|function| &function.coverage))
    }) {
        for (capability, state) in coverage {
            let capability = capability_name(capability).to_owned();
            let (state, state_reasons) = coverage_parts(state);
            *counts
                .entry((capability.clone(), state.to_owned()))
                .or_default() += 1;
            for reason in state_reasons {
                *reasons
                    .entry((capability.clone(), state.to_owned(), reason.clone()))
                    .or_default() += 1;
            }
        }
    }
    let rows = counts
        .into_iter()
        .map(|((capability, state), count)| {
            let state_reasons = reasons
                .iter()
                .filter(|((candidate, candidate_state, _), _)| {
                    candidate == &capability && candidate_state == &state
                })
                .map(|((_, _, reason), count)| json!({"reason": reason, "count": count}))
                .collect::<Vec<_>>();
            json!({
                "capability": capability,
                "state": state,
                "count": count,
                "reasons": state_reasons,
            })
        })
        .collect::<Vec<_>>();
    render(Value::Array(rows), format)
}

fn coverage_parts(state: &CoverageState) -> (&'static str, &[String]) {
    match state {
        CoverageState::Complete => ("complete", &[]),
        CoverageState::Partial { reasons } => ("partial", reasons),
        CoverageState::Indeterminate { reasons } => ("indeterminate", reasons),
        CoverageState::Failed { reasons } => ("failed", reasons),
    }
}

fn functions(analysis: &AnalysisBundle, args: &[String], format: Format) -> Outcome {
    let mut file = None;
    let mut language = None;
    let mut name = None;
    let mut limit = 100_usize;
    let mut index = 0;
    while index < args.len() {
        let (key, value, consumed) = match_option(args, index);
        match key {
            "--file" => file = value,
            "--language" => language = value,
            "--name" => name = value,
            "--limit" => {
                let Some(value) = value else {
                    return usage_error("--limit requires a positive integer");
                };
                limit = match value.parse() {
                    Ok(value) if value > 0 => value,
                    _ => return usage_error("--limit requires a positive integer"),
                };
            }
            _ => return usage_error(&format!("unknown functions option {}", args[index])),
        }
        index += consumed;
    }
    let rows = analysis
        .program
        .modules
        .iter()
        .filter(|module| {
            file.as_ref()
                .is_none_or(|value| &module.source_file == value)
                && language
                    .as_ref()
                    .is_none_or(|value| &module.language == value)
        })
        .flat_map(|module| {
            module.functions.iter().filter_map(|function| {
                if name
                    .as_ref()
                    .is_some_and(|value| !function.name.contains(value))
                {
                    return None;
                }
                Some(function_record(
                    module,
                    function,
                    summary_for(analysis, &function.symbol_id),
                ))
            })
        })
        .take(limit)
        .collect::<Vec<_>>();
    render(Value::Array(rows), format)
}

fn show(analysis: &AnalysisBundle, args: &[String], format: Format) -> Outcome {
    if args.len() != 1 {
        return usage_error("usage: compass program show <SYMBOL>");
    }
    let function = match resolve_function(analysis, &args[0]) {
        Ok(function) => function,
        Err(error) => return Outcome::failure_with_code(format!("error: {error}"), 4),
    };
    let Some(module) = analysis.program.modules.iter().find(|module| {
        module
            .functions
            .iter()
            .any(|candidate| candidate.symbol_id == function.symbol_id)
    }) else {
        return Outcome::failure_with_code(
            "error: resolved Program IR function has no owning module".to_owned(),
            3,
        );
    };
    let callers = analysis
        .reverse_calls
        .get(&function.symbol_id)
        .cloned()
        .unwrap_or_default();
    render(
        json!({
            "module": {
                "source_file": module.source_file,
                "language": module.language,
                "graph_node_id": module.graph_node_id,
            },
            "function": function,
            "summary": summary_for(analysis, &function.symbol_id),
            "callers": callers,
        }),
        format,
    )
}

fn callers(analysis: &AnalysisBundle, args: &[String], format: Format) -> Outcome {
    if args.len() != 1 {
        return usage_error("usage: compass program callers <SYMBOL>");
    }
    let target = match resolve_function(analysis, &args[0]) {
        Ok(function) => function,
        Err(error) => return Outcome::failure_with_code(format!("error: {error}"), 4),
    };
    let caller_ids = analysis
        .reverse_calls
        .get(&target.symbol_id)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .collect::<BTreeSet<_>>();
    let rows = analysis
        .program
        .modules
        .iter()
        .flat_map(|module| {
            module
                .functions
                .iter()
                .filter(|function| caller_ids.contains(&function.symbol_id))
                .map(|function| {
                    function_record(module, function, summary_for(analysis, &function.symbol_id))
                })
        })
        .collect::<Vec<_>>();
    render(Value::Array(rows), format)
}

fn explain_call(analysis: &AnalysisBundle, args: &[String], format: Format) -> Outcome {
    if args.len() != 1 {
        return usage_error("usage: compass program explain-call <FILE:BYTE>");
    }
    let Some((file, byte)) = args[0].rsplit_once(':') else {
        return usage_error("call location must be FILE:BYTE");
    };
    let Ok(byte) = byte.parse::<u64>() else {
        return usage_error("call location byte must be a non-negative integer");
    };
    let evidence = analysis
        .program
        .evidence
        .iter()
        .map(|record| (record.id.as_str(), record))
        .collect::<BTreeMap<_, _>>();
    let mut rows = Vec::new();
    for module in &analysis.program.modules {
        if module.source_file != file {
            continue;
        }
        for function in &module.functions {
            for block in &function.blocks {
                for operation in &block.operations {
                    let OperationKind::Call {
                        callee,
                        callee_anchor,
                        resolved_symbols,
                        receiver_type,
                    } = &operation.kind
                    else {
                        continue;
                    };
                    if !(operation.anchor.start_byte <= byte && byte <= operation.anchor.end_byte)
                        && !(callee_anchor.start_byte <= byte && byte <= callee_anchor.end_byte)
                    {
                        continue;
                    }
                    rows.push(json!({
                        "source_file": module.source_file,
                        "function": function.name,
                        "symbol_id": function.symbol_id,
                        "call": {
                            "callee": callee,
                            "start_byte": operation.anchor.start_byte,
                            "end_byte": operation.anchor.end_byte,
                            "callee_start_byte": callee_anchor.start_byte,
                            "callee_end_byte": callee_anchor.end_byte,
                            "resolved_symbols": resolved_symbols,
                            "receiver_type": receiver_type,
                        },
                        "coverage": function.coverage.get(&compass_ir::Capability::CallResolution),
                        "evidence": operation.evidence.iter()
                            .filter_map(|id| evidence.get(id.as_str()).copied())
                            .collect::<Vec<_>>(),
                    }));
                }
            }
        }
    }
    if rows.is_empty() {
        return Outcome::failure_with_code(format!("error: no call contains {file}:{byte}"), 4);
    }
    render(Value::Array(rows), format)
}

fn query(analysis: &AnalysisBundle, args: &[String]) -> Outcome {
    if args.is_empty() {
        return usage_error("usage: compass program query <COMPASSQL> [--format table|json|jsonl]");
    }
    let graph = match program_graph(analysis).and_then(Graph::from_document) {
        Ok(graph) => graph,
        Err(error) => {
            return Outcome::failure(format!("error: Program IR projection failed: {error}"));
        }
    };
    super::query_commands::command_cql_on_graph(args, &graph)
}

fn program_graph(analysis: &AnalysisBundle) -> Result<GraphDocument, compass_model::GraphError> {
    let mut nodes = Vec::new();
    let mut links = Vec::new();
    let local_symbols = analysis
        .program
        .modules
        .iter()
        .flat_map(|module| &module.functions)
        .map(|function| function.symbol_id.as_str())
        .collect::<BTreeSet<_>>();
    for provider in &analysis.program.providers {
        nodes.push(node(
            format!("provider:{}", provider.id),
            json!({
                "label": provider.id,
                "kind": "program_provider",
                "provider_kind": format!("{:?}", provider.kind).to_ascii_lowercase(),
                "version": provider.version,
                "scope": provider.scope,
            }),
        ));
    }
    for evidence in &analysis.program.evidence {
        let evidence_id = format!("evidence:{}", evidence.id);
        nodes.push(node(
            evidence_id.clone(),
            json!({
                "label": evidence.detail,
                "kind": "program_evidence",
                "capability": capability_name(&evidence.capability),
                "source_file": evidence.source_file,
                "evidence_id": evidence.id,
            }),
        ));
        links.push(edge(
            format!("provider:{}", evidence.provider_id),
            evidence_id,
            "produced",
        ));
    }
    for module in &analysis.program.modules {
        let module_id = format!("module:{}", module.source_file);
        nodes.push(node(
            module_id.clone(),
            json!({
                "label": module.source_file,
                "kind": "program_module",
                "source_file": module.source_file,
                "language": module.language,
                "source_digest": module.source_digest,
                "graph_node_id": module.graph_node_id,
                "coverage": module.coverage,
            }),
        ));
        for evidence in &module.evidence {
            links.push(edge(
                module_id.clone(),
                format!("evidence:{evidence}"),
                "evidenced_by",
            ));
        }
        for function in &module.functions {
            let summary = summary_for(analysis, &function.symbol_id);
            let call_resolution = function
                .coverage
                .get(&compass_ir::Capability::CallResolution);
            nodes.push(node(
                function.symbol_id.clone(),
                json!({
                    "label": function.name,
                    "kind": "program_function",
                    "symbol_id": function.symbol_id,
                    "graph_node_id": function.graph_node_id,
                    "source_file": module.source_file,
                    "language": module.language,
                    "start_byte": function.anchor.start_byte,
                    "end_byte": function.anchor.end_byte,
                    "signature_digest": function.signature_digest,
                    "body_digest": function.body_digest,
                    "coverage": function.coverage,
                    "resolved_calls": summary.map(|value| value.resolved_calls.len()).unwrap_or(0),
                    "unresolved_calls": summary.map(|value| value.unresolved_calls.len()).unwrap_or(0),
                    "call_resolution_state": call_resolution.map(coverage_state_name),
                    "impact_eligible": impact_eligible(function, summary),
                }),
            ));
            links.push(edge(
                module_id.clone(),
                function.symbol_id.clone(),
                "contains",
            ));
            for evidence in &function.evidence {
                links.push(edge(
                    function.symbol_id.clone(),
                    format!("evidence:{evidence}"),
                    "evidenced_by",
                ));
            }
            for block in &function.blocks {
                for operation in &block.operations {
                    let operation_id = format!(
                        "operation:{}:{}:{}",
                        function.symbol_id, block.id, operation.ordinal
                    );
                    let (operation_kind, detail) = operation_properties(operation);
                    nodes.push(node(
                        operation_id.clone(),
                        json!({
                            "label": detail,
                            "kind": "program_operation",
                            "operation_kind": operation_kind,
                            "source_file": module.source_file,
                            "start_byte": operation.anchor.start_byte,
                            "end_byte": operation.anchor.end_byte,
                            "detail": detail,
                        }),
                    ));
                    links.push(edge(
                        function.symbol_id.clone(),
                        operation_id.clone(),
                        "has_operation",
                    ));
                    for evidence in &operation.evidence {
                        links.push(edge(
                            operation_id.clone(),
                            format!("evidence:{evidence}"),
                            "evidenced_by",
                        ));
                    }
                    if let OperationKind::Call {
                        resolved_symbols, ..
                    } = &operation.kind
                    {
                        for target in resolved_symbols {
                            if !local_symbols.contains(target.as_str()) {
                                nodes.push(node(
                                    target.clone(),
                                    json!({
                                        "label": target,
                                        "kind": "external_program_symbol",
                                        "symbol_id": target,
                                    }),
                                ));
                            }
                            links.push(edge(operation_id.clone(), target.clone(), "resolves_to"));
                            links.push(edge(function.symbol_id.clone(), target.clone(), "calls"));
                        }
                    }
                }
            }
        }
    }
    Ok(GraphDocument {
        directed: true,
        multigraph: true,
        graph: Map::from_iter([(
            "schema".to_owned(),
            Value::String("compass.program.graph/1".to_owned()),
        )]),
        nodes,
        links,
        extras: BTreeMap::new(),
        used_legacy_edges_key: false,
    })
}

fn node(id: String, attributes: Value) -> NodeRecord {
    NodeRecord {
        id,
        attributes: attributes.as_object().cloned().unwrap_or_default(),
    }
}

fn edge(source: String, target: String, relation: &str) -> EdgeRecord {
    EdgeRecord {
        source,
        target,
        attributes: Map::from_iter([("relation".to_owned(), Value::String(relation.to_owned()))]),
    }
}

fn operation_properties(operation: &Operation) -> (&'static str, String) {
    match &operation.kind {
        OperationKind::Call { callee, .. } => ("call", callee.clone()),
        OperationKind::Read { path } => ("read", path.clone()),
        OperationKind::Write { path } => ("write", path.clone()),
        OperationKind::Await => ("await", "await".to_owned()),
        OperationKind::Throw { value } => ("throw", value.clone()),
    }
}

fn function_record(
    module: &compass_ir::ModuleIr,
    function: &FunctionIr,
    summary: Option<&FunctionSummary>,
) -> Value {
    let call_resolution = function
        .coverage
        .get(&compass_ir::Capability::CallResolution);
    json!({
        "symbol_id": function.symbol_id,
        "name": function.name,
        "source_file": module.source_file,
        "language": module.language,
        "graph_node_id": function.graph_node_id,
        "start_byte": function.anchor.start_byte,
        "end_byte": function.anchor.end_byte,
        "resolved_calls": summary.map(|value| value.resolved_calls.len()).unwrap_or(0),
        "unresolved_calls": summary.map(|value| value.unresolved_calls.len()).unwrap_or(0),
        "call_resolution_state": call_resolution.map(coverage_state_name),
        "impact_eligible": impact_eligible(function, summary),
        "coverage": function.coverage,
    })
}

fn coverage_state_name(state: &CoverageState) -> &'static str {
    coverage_parts(state).0
}

fn capability_name(capability: &Capability) -> &'static str {
    match capability {
        Capability::Syntax => "syntax",
        Capability::SymbolIdentity => "symbol_identity",
        Capability::Definitions => "definitions",
        Capability::References => "references",
        Capability::Types => "types",
        Capability::CallResolution => "call_resolution",
        Capability::ControlFlow => "control_flow",
        Capability::DataFlow => "data_flow",
        Capability::Effects => "effects",
        Capability::Contracts => "contracts",
    }
}

fn impact_eligible(function: &FunctionIr, summary: Option<&FunctionSummary>) -> bool {
    matches!(
        function
            .coverage
            .get(&compass_ir::Capability::CallResolution),
        Some(CoverageState::Complete)
    ) && summary.is_some_and(|summary| summary.unresolved_calls.is_empty())
}

fn summary_for<'a>(analysis: &'a AnalysisBundle, symbol: &str) -> Option<&'a FunctionSummary> {
    analysis
        .summaries
        .binary_search_by(|summary| summary.symbol_id.as_str().cmp(symbol))
        .ok()
        .and_then(|index| analysis.summaries.get(index))
}

fn resolve_function<'a>(
    analysis: &'a AnalysisBundle,
    query: &str,
) -> Result<&'a FunctionIr, String> {
    let mut candidates = analysis
        .program
        .modules
        .iter()
        .flat_map(|module| &module.functions)
        .filter(|function| {
            function.symbol_id == query
                || function.symbol_id.starts_with(query)
                || function.graph_node_id.as_deref() == Some(query)
                || function.name == query
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| left.symbol_id.cmp(&right.symbol_id));
    candidates.dedup_by(|left, right| left.symbol_id == right.symbol_id);
    match candidates.as_slice() {
        [] => Err(format!("no Program IR function matches '{query}'")),
        [function] => Ok(function),
        functions => Err(format!(
            "function selector '{query}' is ambiguous: {}",
            functions
                .iter()
                .take(8)
                .map(|function| format!("{} ({})", function.name, function.symbol_id))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

fn match_option(args: &[String], index: usize) -> (&str, Option<String>, usize) {
    let value = &args[index];
    if let Some((key, value)) = value.split_once('=') {
        return (key, Some(value.to_owned()), 1);
    }
    (
        value,
        args.get(index + 1).cloned(),
        usize::from(args.get(index + 1).is_some()) + 1,
    )
}

fn usage_error(message: &str) -> Outcome {
    Outcome::failure_with_code(format!("error: {message}"), 2)
}

pub(super) fn help() -> String {
    "Inspect and query canonical Program IR

Usage:
  compass program <COMMAND> [OPTIONS]

Commands:
  summary                    Show artifact and evidence counts
  coverage                   Aggregate capability coverage and reasons
  functions                  List functions; filter with --file, --language, --name, --limit
  show <SYMBOL>              Show one function, summary, evidence coverage, and callers
  callers <SYMBOL>           List resolved callers
  explain-call <FILE:BYTE>   Explain calls containing an exact source byte
  query <COMPASSQL>          Query an in-memory Program IR graph projection

Common options:
  --program <PATH>           Program artifact [default: compass-out/program.json]
  --format <text|json>       Inspection output format [default: text]

Examples:
  compass program coverage
  compass program functions --language rust --name build --format json
  compass program show 0123abcd
  compass program explain-call src/lib.rs:240
  compass program query \"MATCH (f) WHERE f.kind = 'program_function' RETURN f LIMIT 10\"

Notes:
  Inspection is offline and read-only. Definite conclusions still require the
  relevant capability to report complete coverage."
        .to_owned()
}
