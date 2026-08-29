#![cfg(feature = "mem")]

use std::collections::BTreeSet;
use std::error::Error;
use std::fs;
use std::io::Write as _;
use std::path::PathBuf;
use std::time::Instant;

use compass_graphdb_surreal::{ProjectionPlan, RelationPageRequest, SurrealProjection};
use compass_model::code_graph::{BuildMetadata, EdgeKind, GraphDocument};
use compass_model::identity::{edge_id, file_id};
use compass_model::provenance::SourceAnchor;
use compass_model::query_contract::{
    CallRequest, CodeQueryLimits, ExploreRequest, ImpactRequest, NodeTrailRequest,
};
use compass_model::validate_code_graph;
use compass_query::{EngineSelection, open_with_engine};
use serde::{Deserialize, Serialize};
use serde_json::json;

const SOURCE_PATH: &str = "qualification/generated.rs";

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct QualificationProfile {
    name: String,
    nodes: u64,
    edges: u64,
    sample_ordinals: Vec<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct QualificationProfiles {
    schema: String,
    profiles: Vec<QualificationProfile>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkloadMeasurement {
    current_microseconds: Vec<u64>,
    surreal_microseconds: Vec<u64>,
    current_p95_microseconds: u64,
    surreal_p95_microseconds: u64,
    current_response_bytes: usize,
    surreal_response_bytes: usize,
}

fn digest(byte: char) -> String {
    format!("sha256:{}", byte.to_string().repeat(64))
}

fn node_id(ordinal: u64) -> String {
    format!("n:qualification:{ordinal:07}")
}

fn anchor(ordinal: u64) -> Result<SourceAnchor, Box<dyn Error>> {
    let line = u32::try_from(ordinal + 1)?;
    Ok(SourceAnchor {
        file: SOURCE_PATH.to_owned(),
        start_byte: ordinal,
        end_byte: ordinal + 1,
        start_line: line,
        start_column: 0,
        end_line: line,
        end_column: 1,
    })
}

fn edge_endpoints(ordinal: u64, nodes: u64) -> (u64, u64) {
    let chain_edges = nodes - 1;
    if ordinal < chain_edges {
        return (ordinal, ordinal + 1);
    }
    let chord = ordinal - chain_edges;
    if chord == 0 {
        return (0, 1);
    }
    let source = (chord - 1) % nodes;
    let band = (chord - 1) / nodes;
    let offset = 2 + (band % (nodes - 2));
    (source, (source + offset) % nodes)
}

fn scale_sample(profile: &QualificationProfile) -> Result<GraphDocument, Box<dyn Error>> {
    if profile.nodes < 3 || profile.edges < profile.nodes {
        return Err(format!("invalid qualification profile {}", profile.name).into());
    }
    let mut edge_ordinals = BTreeSet::from([0, profile.nodes - 1, profile.edges - 1]);
    for ordinal in &profile.sample_ordinals {
        if *ordinal >= profile.nodes {
            return Err(format!("sample ordinal {ordinal} exceeds {}", profile.nodes).into());
        }
        let first = ordinal.saturating_sub(3);
        let last = (ordinal + 3).min(profile.nodes - 1);
        for edge in first..last {
            edge_ordinals.insert(edge);
        }
        if *ordinal < profile.edges {
            edge_ordinals.insert(*ordinal);
        }
    }

    let mut node_ordinals = BTreeSet::new();
    for edge in &edge_ordinals {
        let (source, target) = edge_endpoints(*edge, profile.nodes);
        node_ordinals.insert(source);
        node_ordinals.insert(target);
    }
    node_ordinals.extend(profile.sample_ordinals.iter().copied());

    let mut graph = GraphDocument::empty_v1(BuildMetadata {
        builder_version: "compass.qualification.scale-sample/1".to_owned(),
        schema_fingerprint: digest('1'),
        source_tree_digest: digest(if profile.name.ends_with("medium") {
            '2'
        } else {
            '3'
        }),
        configuration_digest: digest('4'),
        generation_id: digest(if profile.name.ends_with("medium") {
            '5'
        } else {
            '6'
        }),
        source_commit: None,
    });
    graph.graph.files = serde_json::from_value(json!([{
        "id": file_id(SOURCE_PATH),
        "path": SOURCE_PATH,
        "contentDigest": digest('7'),
        "byteSize": profile.edges + 1,
        "generated": true,
        "extractionStatus": "generated",
        "extractorVersions": ["compass.qualification.scale-sample/1"]
    }]))?;

    graph.nodes = node_ordinals
        .iter()
        .map(|ordinal| {
            let source = anchor(*ordinal)?;
            serde_json::from_value(json!({
                "id": node_id(*ordinal),
                "kind": "function",
                "name": format!("Node{ordinal:07}"),
                "qualifiedName": format!("qualification::Node{ordinal:07}"),
                "language": "rust",
                "source": source,
                "evidence": [{
                    "origin": "artifact",
                    "extractor": "compass.qualification.scale-sample/1",
                    "confidence": "exact",
                    "anchors": [source]
                }]
            }))
            .map_err(Into::into)
        })
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;

    graph.links = edge_ordinals
        .iter()
        .map(|ordinal| {
            let (source_ordinal, target_ordinal) = edge_endpoints(*ordinal, profile.nodes);
            let source = node_id(source_ordinal);
            let target = node_id(target_ordinal);
            let site = anchor(*ordinal)?;
            let id = edge_id(&source, EdgeKind::Calls, &target, Some(&site), None);
            serde_json::from_value(json!({
                "id": id,
                "key": id,
                "source": source,
                "target": target,
                "kind": "calls",
                "relationshipSite": site,
                "evidence": [{
                    "origin": "artifact",
                    "extractor": "compass.qualification.scale-sample/1",
                    "confidence": "exact",
                    "anchors": [site]
                }],
                "weight": 1.0
            }))
            .map_err(Into::into)
        })
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    graph.nodes.sort_by(|left, right| left.id.cmp(&right.id));
    graph.links.sort_by(|left, right| left.id.cmp(&right.id));
    validate_code_graph(&graph)?;
    Ok(graph)
}

fn limits() -> CodeQueryLimits {
    CodeQueryLimits {
        max_depth: 8,
        max_nodes: 128,
        max_edges: 256,
        max_paths: 64,
        max_candidates: 32,
        max_source_bytes: 1_024,
        max_response_bytes: 2_097_152,
    }
}

fn elapsed_microseconds(started: Instant) -> Result<u64, Box<dyn Error>> {
    Ok(u64::try_from(started.elapsed().as_micros())?)
}

fn p95(samples: &[u64]) -> Result<u64, Box<dyn Error>> {
    if samples.is_empty() {
        return Err("p95 requires at least one sample".into());
    }
    let mut ordered = samples.to_vec();
    ordered.sort_unstable();
    let rank = (ordered.len() * 95).div_ceil(100);
    ordered
        .get(rank.saturating_sub(1))
        .copied()
        .ok_or_else(|| "p95 rank exceeded the sample set".into())
}

fn measurement(
    current_microseconds: Vec<u64>,
    surreal_microseconds: Vec<u64>,
    current_response: &impl Serialize,
    surreal_response: &impl Serialize,
) -> Result<WorkloadMeasurement, Box<dyn Error>> {
    Ok(WorkloadMeasurement {
        current_p95_microseconds: p95(&current_microseconds)?,
        surreal_p95_microseconds: p95(&surreal_microseconds)?,
        current_response_bytes: serde_json::to_vec(current_response)?.len(),
        surreal_response_bytes: serde_json::to_vec(surreal_response)?.len(),
        current_microseconds,
        surreal_microseconds,
    })
}

fn atomic_write_json(path: &PathBuf, value: &impl Serialize) -> Result<(), Box<dyn Error>> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("qualification output has no parent: {}", path.display()))?;
    fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut temporary, value)?;
    temporary.write_all(b"\n")?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .map_err(|error| error.error)
        .map(|_file| ())
        .map_err(Into::into)
}

#[tokio::test(flavor = "multi_thread")]
async fn medium_and_large_scale_samples_are_dual_engine_equivalent() -> Result<(), Box<dyn Error>> {
    let profiles: QualificationProfiles = serde_json::from_str(include_str!(
        "../../../benchmarks/qualification/profiles-v1.json"
    ))?;
    if profiles.schema != "compass.qualification-profiles/1" || profiles.profiles.len() != 2 {
        return Err(
            "qualification profile inventory is not the ratified two-profile contract".into(),
        );
    }

    for profile in profiles.profiles {
        let repository = format!("qualification:{}", profile.name);
        let graph = scale_sample(&profile)?;
        let generation = graph.graph.build.generation_id.clone();
        let temporary = tempfile::tempdir()?;
        let graph_path = temporary.path().join("graph.json");
        fs::write(&graph_path, serde_json::to_vec(&graph)?)?;
        let current = open_with_engine(
            &graph_path,
            None,
            &temporary.path().join("cache"),
            EngineSelection::Json,
        )?;
        let database = profile.name.replace('-', "_");
        let surreal = SurrealProjection::memory(&database, &database).await?;
        let plan = ProjectionPlan::from_graph(&repository, &graph)?;
        surreal.activate(&plan).await?;

        for ordinal in &profile.sample_ordinals {
            let symbol = node_id(*ordinal);
            let callers = CallRequest {
                symbol: symbol.clone(),
                include_heuristic: false,
                limits: limits(),
            };
            assert_eq!(
                surreal
                    .callers(&repository, callers.clone())
                    .await?
                    .structural_view(&repository, &generation),
                current
                    .callers(callers)?
                    .structural_view(&repository, &generation)
            );
            let callees = CallRequest {
                symbol: symbol.clone(),
                include_heuristic: false,
                limits: limits(),
            };
            assert_eq!(
                surreal
                    .callees(&repository, callees.clone())
                    .await?
                    .structural_view(&repository, &generation),
                current
                    .callees(callees)?
                    .structural_view(&repository, &generation)
            );
            let impact = ImpactRequest {
                symbol: symbol.clone(),
                include_heuristic: false,
                limits: limits(),
            };
            assert_eq!(
                surreal
                    .impact(&repository, impact.clone())
                    .await?
                    .structural_view(&repository, &generation),
                current
                    .impact(impact)?
                    .structural_view(&repository, &generation)
            );

            let first = ordinal.saturating_sub(3);
            let trail = NodeTrailRequest {
                source: node_id(first),
                target: symbol.clone(),
                include_heuristic: false,
                limits: limits(),
            };
            assert_eq!(
                surreal
                    .node_trail(&repository, trail.clone())
                    .await?
                    .structural_view(&repository, &generation),
                current
                    .node_trail(trail)?
                    .structural_view(&repository, &generation)
            );
        }

        let subgraph = ExploreRequest {
            symbols: profile
                .sample_ordinals
                .iter()
                .map(|value| node_id(*value))
                .collect(),
            root: String::new(),
            include_heuristic: false,
            limits: limits(),
        };
        assert_eq!(
            surreal
                .structural_subgraph(&repository, subgraph.clone())
                .await?
                .structural_view(&repository, &generation),
            current
                .structural_subgraph(subgraph)?
                .structural_view(&repository, &generation)
        );

        let expected = plan
            .relations
            .iter()
            .map(|relation| relation.compass_edge_id.clone())
            .collect::<Vec<_>>();
        let mut cursor = None;
        let mut observed = Vec::new();
        loop {
            let page = surreal
                .read_relation_page(
                    &repository,
                    RelationPageRequest {
                        max_items: 3,
                        cursor,
                        include_heuristic: true,
                    },
                )
                .await?;
            observed.extend(page.relations.into_iter().map(|relation| relation.id));
            let Some(next) = page.next_cursor else {
                break;
            };
            cursor = Some(next);
        }
        assert_eq!(observed, expected);
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "explicit full qualification-medium measurement"]
async fn full_medium_dual_engine_measurement_emits_raw_evidence() -> Result<(), Box<dyn Error>> {
    let graph_path = PathBuf::from(std::env::var("COMPASS_SURREAL_QUALIFICATION_GRAPH")?);
    let output_path = PathBuf::from(std::env::var("COMPASS_SURREAL_QUALIFICATION_OUTPUT")?);
    let samples = std::env::var("COMPASS_SURREAL_QUALIFICATION_SAMPLES")
        .unwrap_or_else(|_| "5".to_owned())
        .parse::<usize>()?;
    if samples == 0 || samples > 20 {
        return Err("qualification samples must be in 1..=20".into());
    }

    let graph = GraphDocument::load(&graph_path)?;
    if graph.nodes.len() != 100_000 || graph.links.len() != 250_000 {
        return Err(format!(
            "qualification-medium requires 100000 nodes/250000 edges, found {}/{}",
            graph.nodes.len(),
            graph.links.len()
        )
        .into());
    }
    let repository = "qualification-medium";
    let generation = graph.graph.build.generation_id.clone();
    let plan_started = Instant::now();
    let plan = ProjectionPlan::from_graph(repository, &graph)?;
    let plan_microseconds = elapsed_microseconds(plan_started)?;
    let surreal = SurrealProjection::memory("qualification_medium", "qualification_medium").await?;
    let activation_started = Instant::now();
    let activation = surreal.activate(&plan).await?;
    let activation_microseconds = elapsed_microseconds(activation_started)?;
    drop(plan);
    drop(graph);

    let temporary = tempfile::tempdir()?;
    let current = open_with_engine(
        &graph_path,
        None,
        &temporary.path().join("cache"),
        EngineSelection::Json,
    )?;
    let query_limits = CodeQueryLimits {
        max_depth: 8,
        max_nodes: 512,
        max_edges: 1_024,
        max_paths: 128,
        max_candidates: 256,
        max_source_bytes: 1_048_576,
        max_response_bytes: 8_388_608,
    };
    let callers = CallRequest {
        symbol: "qualification::Node0099999".to_owned(),
        include_heuristic: false,
        limits: query_limits.clone(),
    };
    let callees = CallRequest {
        symbol: "qualification::Node0000000".to_owned(),
        include_heuristic: false,
        limits: query_limits.clone(),
    };
    let impact = ImpactRequest {
        symbol: "qualification::Node0099999".to_owned(),
        include_heuristic: false,
        limits: CodeQueryLimits {
            max_depth: 3,
            ..query_limits.clone()
        },
    };
    let trail = NodeTrailRequest {
        source: "qualification::Node0000000".to_owned(),
        target: "qualification::Node0000009".to_owned(),
        include_heuristic: false,
        limits: query_limits,
    };

    let _warm_current = current.callers(callers.clone())?;
    let _warm_surreal = surreal.callers(repository, callers.clone()).await?;

    let mut caller_current = Vec::with_capacity(samples);
    let mut caller_surreal = Vec::with_capacity(samples);
    let mut caller_current_response = None;
    let mut caller_surreal_response = None;
    for _iteration in 0..samples {
        let started = Instant::now();
        caller_current_response = Some(current.callers(callers.clone())?);
        caller_current.push(elapsed_microseconds(started)?);
        let started = Instant::now();
        caller_surreal_response = Some(surreal.callers(repository, callers.clone()).await?);
        caller_surreal.push(elapsed_microseconds(started)?);
    }
    let caller_current_response = caller_current_response.ok_or("missing caller response")?;
    let caller_surreal_response = caller_surreal_response.ok_or("missing caller response")?;
    assert_eq!(
        caller_current_response.structural_view(repository, &generation),
        caller_surreal_response.structural_view(repository, &generation)
    );

    let mut callee_current = Vec::with_capacity(samples);
    let mut callee_surreal = Vec::with_capacity(samples);
    let mut callee_current_response = None;
    let mut callee_surreal_response = None;
    for _iteration in 0..samples {
        let started = Instant::now();
        callee_current_response = Some(current.callees(callees.clone())?);
        callee_current.push(elapsed_microseconds(started)?);
        let started = Instant::now();
        callee_surreal_response = Some(surreal.callees(repository, callees.clone()).await?);
        callee_surreal.push(elapsed_microseconds(started)?);
    }
    let callee_current_response = callee_current_response.ok_or("missing callee response")?;
    let callee_surreal_response = callee_surreal_response.ok_or("missing callee response")?;
    assert_eq!(
        callee_current_response.structural_view(repository, &generation),
        callee_surreal_response.structural_view(repository, &generation)
    );

    let mut impact_current = Vec::with_capacity(samples);
    let mut impact_surreal = Vec::with_capacity(samples);
    let mut impact_current_response = None;
    let mut impact_surreal_response = None;
    for _iteration in 0..samples {
        let started = Instant::now();
        impact_current_response = Some(current.impact(impact.clone())?);
        impact_current.push(elapsed_microseconds(started)?);
        let started = Instant::now();
        impact_surreal_response = Some(surreal.impact(repository, impact.clone()).await?);
        impact_surreal.push(elapsed_microseconds(started)?);
    }
    let impact_current_response = impact_current_response.ok_or("missing impact response")?;
    let impact_surreal_response = impact_surreal_response.ok_or("missing impact response")?;
    assert_eq!(
        impact_current_response.structural_view(repository, &generation),
        impact_surreal_response.structural_view(repository, &generation)
    );

    let mut trail_current = Vec::with_capacity(samples);
    let mut trail_surreal = Vec::with_capacity(samples);
    let mut trail_current_response = None;
    let mut trail_surreal_response = None;
    for _iteration in 0..samples {
        let started = Instant::now();
        trail_current_response = Some(current.node_trail(trail.clone())?);
        trail_current.push(elapsed_microseconds(started)?);
        let started = Instant::now();
        trail_surreal_response = Some(surreal.node_trail(repository, trail.clone()).await?);
        trail_surreal.push(elapsed_microseconds(started)?);
    }
    let trail_current_response = trail_current_response.ok_or("missing trail response")?;
    let trail_surreal_response = trail_surreal_response.ok_or("missing trail response")?;
    assert_eq!(
        trail_current_response.structural_view(repository, &generation),
        trail_surreal_response.structural_view(repository, &generation)
    );

    let evidence = json!({
        "schema": "compass.surreal-qualification-raw/1",
        "profile": "qualification-medium",
        "graph": {
            "path": graph_path,
            "nodes": activation.nodes,
            "edges": activation.relations,
            "generationId": generation
        },
        "condition": "one in-process current-engine and Surreal Mem projection after one unmeasured caller warmup",
        "samplesPerWorkload": samples,
        "projection": {
            "planMicroseconds": plan_microseconds,
            "activationMicroseconds": activation_microseconds
        },
        "workloads": {
            "callers": measurement(caller_current, caller_surreal, &caller_current_response, &caller_surreal_response)?,
            "callees": measurement(callee_current, callee_surreal, &callee_current_response, &callee_surreal_response)?,
            "impactDepth3": measurement(impact_current, impact_surreal, &impact_current_response, &impact_surreal_response)?,
            "pathDepth3": measurement(trail_current, trail_surreal, &trail_current_response, &trail_surreal_response)?
        },
        "semanticMismatchCount": 0,
        "responseReduction": {
            "method": "canonical semantic response bytes",
            "percent": 0
        }
    });
    atomic_write_json(&output_path, &evidence)?;
    Ok(())
}
