//! Bounded offline qualification helper: reuse one native engine for a batch.
use compass_model::query_contract::{CallRequest, CodeQueryLimits, ImpactRequest};
use serde::Deserialize;
use std::error::Error;
use std::path::PathBuf;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    operation: String,
    symbol: String,
    #[serde(default)]
    semantic_search: bool,
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let graph = PathBuf::from(arguments.next().ok_or("graph path required")?);
    let requests_path = PathBuf::from(arguments.next().ok_or("requests path required")?);
    let output = PathBuf::from(arguments.next().ok_or("output path required")?);
    let cache = arguments
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| output.with_extension("query-cache"));
    if arguments.next().is_some() {
        return Err("expected graph, requests, output and optional cache paths".into());
    }
    let requests: Vec<Request> = serde_json::from_slice(&compass_files::read_bytes_bounded(
        &requests_path,
        1024 * 1024,
    )?)?;
    if requests.is_empty() || requests.len() > 32 {
        return Err("request count must be 1 to 32".into());
    }
    let (document, identity) =
        compass_model::code_graph::GraphDocument::load_with_artifact_digest(&graph)?;
    let mut engine =
        compass_query::open_with_verified_document(document, identity, &graph, None, &cache)?;
    let mut responses = Vec::new();
    let mut bytes = 0_usize;
    for (ordinal, request) in requests.into_iter().enumerate() {
        engine.set_semantic_search(request.semantic_search);
        let concept = request.operation == "ask";
        let limits = CodeQueryLimits {
            max_nodes: if concept { 32 } else { 2000 },
            max_edges: if concept { 1000 } else { 10000 },
            max_depth: if request.operation == "callees" { 8 } else { 1 },
            max_response_bytes: 32 * 1024 * 1024,
            ..CodeQueryLimits::default()
        };
        let response = match request.operation.as_str() {
            "ask" => engine.query_natural(compass_query::NaturalQueryRequest {
                question: request.symbol,
                include_heuristic: false,
                limits,
            })?,
            "callees" => engine.callees(CallRequest {
                symbol: request.symbol,
                include_heuristic: false,
                limits,
            })?,
            "impact" => engine.impact(ImpactRequest {
                symbol: request.symbol,
                include_heuristic: false,
                limits,
            })?,
            _ => return Err("only ask, callees and impact are supported".into()),
        };
        bytes = bytes.saturating_add(serde_json::to_vec(&response)?.len());
        if bytes > 64 * 1024 * 1024 {
            return Err("batch output exceeds 64 MiB".into());
        }
        responses.push(response);
        eprintln!("completed query {}", ordinal + 1);
    }
    // A qualification report is written only after every native query succeeds.
    compass_files::write_bytes_atomic(&output, &serde_json::to_vec(&responses)?)?;
    Ok(())
}
