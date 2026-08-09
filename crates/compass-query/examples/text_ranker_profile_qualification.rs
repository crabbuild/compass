use std::error::Error;
use std::io;
use std::time::{Duration, Instant};

use compass_model::Graph;
use compass_query::{TextRankProfile, query_terms, score_nodes_with_profile};
use serde_json::{Value, json};

const SCHEMA_V1: &str = "compass.text-ranker-qualification/1";
const SCALE_NODES: usize = 100_000;
const WARM_SAMPLES: usize = 31;
const QUESTION: &str = "how are dependencies solved";
const EXPECTED_ID: &str = "n:dependency-solve";

fn main() -> Result<(), Box<dyn Error>> {
    let graph = scale_graph()?;
    let terms = query_terms(QUESTION);
    let mode = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "--compare".to_owned());
    let report = match mode.as_str() {
        "--legacy" => single_profile_report(&graph, &terms, TextRankProfile::LegacyV1)?,
        "--bm25" => single_profile_report(&graph, &terms, TextRankProfile::Bm25V1)?,
        "--compare" => comparison_report(&graph, &terms)?,
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "expected --legacy, --bm25, or --compare",
            )
            .into());
        }
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn single_profile_report(
    graph: &Graph,
    terms: &[String],
    profile: TextRankProfile,
) -> Result<Value, Box<dyn Error>> {
    let first = measure(graph, terms, profile)?;
    let warm = warm_samples(graph, terms, profile)?;
    let lexical_index = if profile == TextRankProfile::Bm25V1 {
        let index = graph.lexical_index();
        json!({
            "documents": index.document_count(),
            "terms": index.term_count(),
            "averageDocumentLength": index.average_document_length(),
        })
    } else {
        Value::Null
    };
    Ok(json!({
        "schema": SCHEMA_V1,
        "mode": "single-profile",
        "nodes": SCALE_NODES,
        "warmSamples": WARM_SAMPLES,
        "question": QUESTION,
        "expectedId": EXPECTED_ID,
        "result": profile_report(first, &warm),
        "lexicalIndex": lexical_index,
        "interpretation": interpretation()
    }))
}

fn comparison_report(graph: &Graph, terms: &[String]) -> Result<Value, Box<dyn Error>> {
    let legacy_first = measure(graph, terms, TextRankProfile::LegacyV1)?;
    let legacy_warm = warm_samples(graph, terms, TextRankProfile::LegacyV1)?;
    let bm25_first = measure(graph, terms, TextRankProfile::Bm25V1)?;
    let bm25_warm = warm_samples(graph, terms, TextRankProfile::Bm25V1)?;
    let index = graph.lexical_index();
    Ok(json!({
        "schema": SCHEMA_V1,
        "mode": "comparison",
        "nodes": SCALE_NODES,
        "warmSamples": WARM_SAMPLES,
        "question": QUESTION,
        "expectedId": EXPECTED_ID,
        "legacy": profile_report(legacy_first, &legacy_warm),
        "bm25": profile_report(bm25_first, &bm25_warm),
        "lexicalIndex": {
            "documents": index.document_count(),
            "terms": index.term_count(),
            "averageDocumentLength": index.average_document_length(),
        },
        "interpretation": interpretation()
    }))
}

fn interpretation() -> Value {
    json!({
        "legacyFirstIncludes": "full graph scan",
        "bm25FirstIncludes": "lazy lexical index construction and query",
        "warmSamplesExclude": "graph loading and first-use index construction",
        "peakMemory": "measure each single-profile run with the repository qualification script"
    })
}

fn scale_graph() -> Result<Graph, Box<dyn Error>> {
    let mut nodes = (0..SCALE_NODES.saturating_sub(1))
        .map(|index| {
            json!({
                "id": format!("n:scale:{index:05}"),
                "label": format!("symbol_{index:05}"),
                "kind": if index % 5 == 0 { "method" } else { "function" },
                "source_file": format!("src/module_{:03}.rs", index % 1_000)
            })
        })
        .collect::<Vec<_>>();
    nodes.push(json!({
        "id": EXPECTED_ID,
        "label": "solve_dependencies",
        "qualifiedName": "resolver::solve_dependencies",
        "kind": "function",
        "source_file": "src/dependencies.rs"
    }));
    let document = serde_json::from_value(json!({
        "directed": true,
        "multigraph": false,
        "graph": {},
        "nodes": nodes,
        "links": []
    }))?;
    Ok(Graph::from_document(document)?)
}

fn warm_samples(
    graph: &Graph,
    terms: &[String],
    profile: TextRankProfile,
) -> Result<Vec<Duration>, Box<dyn Error>> {
    (0..WARM_SAMPLES)
        .map(|_| measure(graph, terms, profile).map(|measurement| measurement.elapsed))
        .collect()
}

fn measure(
    graph: &Graph,
    terms: &[String],
    profile: TextRankProfile,
) -> Result<Measurement, Box<dyn Error>> {
    let started = Instant::now();
    let result = score_nodes_with_profile(graph, terms, true, profile);
    let elapsed = started.elapsed();
    let top_id = result
        .scores
        .ranked
        .first()
        .map(|candidate| graph.node(candidate.node).id.as_str())
        .ok_or("qualification query returned no candidates")?;
    if top_id != EXPECTED_ID {
        return Err(format!(
            "{} returned {top_id:?}, expected {EXPECTED_ID:?}",
            profile.as_str()
        )
        .into());
    }
    Ok(Measurement {
        profile,
        elapsed,
        candidate_count: result.scores.ranked.len(),
        candidates_truncated: result.candidates_truncated,
        top_id: top_id.to_owned(),
    })
}

fn profile_report(first: Measurement, warm: &[Duration]) -> Value {
    json!({
        "profile": first.profile.as_str(),
        "firstQueryMicroseconds": micros(first.elapsed),
        "warmP50Microseconds": micros(percentile(warm, 50)),
        "warmP95Microseconds": micros(percentile(warm, 95)),
        "candidateCount": first.candidate_count,
        "candidatesTruncated": first.candidates_truncated,
        "topId": first.top_id,
    })
}

fn percentile(samples: &[Duration], percentile: usize) -> Duration {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let index = sorted
        .len()
        .saturating_mul(percentile)
        .div_ceil(100)
        .saturating_sub(1)
        .min(sorted.len().saturating_sub(1));
    sorted[index]
}

fn micros(duration: Duration) -> u128 {
    duration.as_micros()
}

struct Measurement {
    profile: TextRankProfile,
    elapsed: Duration,
    candidate_count: usize,
    candidates_truncated: bool,
    top_id: String,
}
