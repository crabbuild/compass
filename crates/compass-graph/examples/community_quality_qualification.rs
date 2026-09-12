use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::path::Path;

use compass_graph::{
    Communities, CommunityLimits, CommunityProfile, CommunityRequest, CommunityResult,
    ResolutionPolicy, adjusted_mutual_information, adjusted_rand_index, build_communities,
};
use compass_model::code_graph::{BuildMetadata, EdgeKind, EdgeRecord, NodeKind, NodeRecord};
use compass_model::provenance::{EvidenceConfidence, EvidenceOrigin, Provenance, SourceAnchor};
use serde::Serialize;

const REPORT_SCHEMA: &str = "compass.community-quality-qualification/1";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    schema: &'static str,
    fixture_count: usize,
    acceptance: Acceptance,
    fixtures: Vec<FixtureReport>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Acceptance {
    zero_disconnected_quality_communities: bool,
    deterministic_repeat_and_permutation: bool,
    exact_recovery_for_required_fixtures: bool,
    no_adjusted_rand_regression_over_point_zero_two: bool,
    resolution_limit_improved: bool,
    articulation_improved: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FixtureReport {
    name: String,
    nodes: usize,
    edges: usize,
    exact_recovery_required: bool,
    deterministic_repeat: bool,
    permutation_equal: bool,
    compatibility: ProfileReport,
    quality: ProfileReport,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProfileReport {
    algorithm: String,
    topology: String,
    selector: String,
    selected_candidate_resolution: f64,
    communities: usize,
    disconnected_communities: usize,
    modularity: f64,
    weighted_mean_conductance: f64,
    worst_conductance: f64,
    largest_community_fraction: f64,
    non_isolate_singletons: usize,
    quality_visits: usize,
    adjusted_rand_index: Option<f64>,
    adjusted_mutual_information: Option<f64>,
    exact_recovery: Option<bool>,
    false_merges: Option<usize>,
    false_splits: Option<usize>,
}

struct Fixture {
    name: String,
    document: compass_model::code_graph::GraphDocument,
    planted: Option<Communities>,
    exact_recovery_required: bool,
}

#[derive(Clone, Copy)]
struct EdgeSpec {
    source: usize,
    target: usize,
    kind: EdgeKind,
    confidence: EvidenceConfidence,
}

fn main() -> Result<(), Box<dyn Error>> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if arguments
        .first()
        .is_some_and(|value| value == "graph-profile")
    {
        return graph_profile(&arguments);
    }
    if let Some((profile, fixed_resolution)) = arguments.first().and_then(|argument| match argument
        .as_str()
    {
        "benchmark-compatibility" => Some((CommunityProfile::CompatibilityV1, None)),
        "benchmark-quality" => Some((CommunityProfile::QualityV1, None)),
        "benchmark-quality-fixed" => Some((CommunityProfile::QualityV1, Some(1.0))),
        _ => None,
    }) {
        return benchmark(profile, fixed_resolution);
    }
    let selected = arguments.first();
    let fixed_resolution = arguments
        .get(1)
        .map(|value| value.parse::<f64>())
        .transpose()?;
    let fixtures = fixtures()
        .into_iter()
        .filter(|fixture| selected.is_none_or(|name| fixture.name == *name))
        .collect::<Vec<_>>();
    if fixtures.is_empty() {
        return Err("no community qualification fixture matched".into());
    }
    let mut reports = Vec::with_capacity(fixtures.len());
    for fixture in fixtures {
        reports.push(qualify_fixture(fixture, fixed_resolution)?);
    }
    let report = Report {
        schema: REPORT_SCHEMA,
        fixture_count: reports.len(),
        acceptance: Acceptance {
            zero_disconnected_quality_communities: reports
                .iter()
                .all(|fixture| fixture.quality.disconnected_communities == 0),
            deterministic_repeat_and_permutation: reports
                .iter()
                .all(|fixture| fixture.deterministic_repeat && fixture.permutation_equal),
            exact_recovery_for_required_fixtures: reports.iter().all(|fixture| {
                !fixture.exact_recovery_required || fixture.quality.exact_recovery == Some(true)
            }),
            no_adjusted_rand_regression_over_point_zero_two: reports.iter().all(|fixture| {
                match (
                    fixture.compatibility.adjusted_rand_index,
                    fixture.quality.adjusted_rand_index,
                ) {
                    (Some(compatibility), Some(quality)) => quality + 0.02 >= compatibility,
                    _ => true,
                }
            }),
            resolution_limit_improved: fixture_improved(&reports, "ring-of-cliques"),
            articulation_improved: fixture_improved(&reports, "articulation"),
        },
        fixtures: reports,
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn graph_profile(arguments: &[String]) -> Result<(), Box<dyn Error>> {
    let path = arguments
        .get(1)
        .ok_or("graph-profile requires a graph path")?;
    let profile = arguments.get(2).map(String::as_str).unwrap_or("fixed");
    if arguments.len() > 3 {
        return Err("graph-profile accepts only a graph path and profile".into());
    }
    let document = compass_model::code_graph::GraphDocument::load_for_recluster(Path::new(path))?;
    let result = match profile {
        "compatibility" => detect(&document, CommunityProfile::CompatibilityV1)?,
        "fixed" => detect_quality(&document, Some(1.0))?,
        "automatic" => detect_quality(&document, None)?,
        _ => return Err("graph-profile must be compatibility, fixed, or automatic".into()),
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&profile_report(&result, None))?
    );
    Ok(())
}

fn benchmark(
    profile: CommunityProfile,
    fixed_resolution: Option<f64>,
) -> Result<(), Box<dyn Error>> {
    let fixtures = fixtures();
    let mut observed = 0usize;
    for _ in 0..50 {
        for fixture in &fixtures {
            let result = if profile == CommunityProfile::QualityV1 {
                detect_quality(&fixture.document, fixed_resolution)?
            } else {
                detect(&fixture.document, profile)?
            };
            observed = observed.saturating_add(result.communities.len());
        }
    }
    println!("{observed}");
    Ok(())
}
fn fixture_improved(reports: &[FixtureReport], name: &str) -> bool {
    reports.iter().any(|fixture| {
        fixture.name == name
            && matches!(
                (
                    fixture.compatibility.adjusted_rand_index,
                    fixture.quality.adjusted_rand_index,
                ),
                (Some(compatibility), Some(quality)) if quality > compatibility + 0.02
            )
    })
}

fn qualify_fixture(
    fixture: Fixture,
    fixed_resolution: Option<f64>,
) -> Result<FixtureReport, Box<dyn Error>> {
    let compatibility = detect(&fixture.document, CommunityProfile::CompatibilityV1)?;
    let quality = detect_quality(&fixture.document, fixed_resolution)?;
    let repeated = detect_quality(&fixture.document, fixed_resolution)?;
    let mut permuted = fixture.document.clone();
    permuted.nodes.reverse();
    permuted.links.reverse();
    let permuted = detect_quality(&permuted, fixed_resolution)?;
    Ok(FixtureReport {
        name: fixture.name,
        nodes: fixture.document.nodes.len(),
        edges: fixture.document.links.len(),
        exact_recovery_required: fixture.exact_recovery_required,
        deterministic_repeat: quality.communities == repeated.communities
            && quality.quality == repeated.quality,
        permutation_equal: quality.communities == permuted.communities
            && quality.quality == permuted.quality,
        compatibility: profile_report(&compatibility, fixture.planted.as_ref()),
        quality: profile_report(&quality, fixture.planted.as_ref()),
    })
}

fn detect_quality(
    document: &compass_model::code_graph::GraphDocument,
    fixed_resolution: Option<f64>,
) -> Result<CommunityResult, compass_graph::CommunityError> {
    let changed_sources = BTreeSet::new();
    build_communities(
        document,
        &CommunityRequest {
            profile: CommunityProfile::QualityV1,
            resolution: fixed_resolution.map_or(
                ResolutionPolicy::Auto { base: 1.0 },
                ResolutionPolicy::Fixed,
            ),
            exclude_hubs_percentile: None,
            previous: None,
            incremental: false,
            changed_sources: &changed_sources,
            limits: CommunityLimits::default(),
        },
    )
}

fn detect(
    document: &compass_model::code_graph::GraphDocument,
    profile: CommunityProfile,
) -> Result<CommunityResult, compass_graph::CommunityError> {
    let changed_sources = BTreeSet::new();
    build_communities(
        document,
        &CommunityRequest {
            profile,
            resolution: if profile == CommunityProfile::QualityV1 {
                ResolutionPolicy::Auto { base: 1.0 }
            } else {
                ResolutionPolicy::Fixed(1.0)
            },
            exclude_hubs_percentile: None,
            previous: None,
            incremental: false,
            changed_sources: &changed_sources,
            limits: CommunityLimits::default(),
        },
    )
}

fn profile_report(result: &CommunityResult, planted: Option<&Communities>) -> ProfileReport {
    let (ari, ami, exact, false_merges, false_splits) =
        planted.map_or((None, None, None, None, None), |planted| {
            let (merges, splits) = merge_split_counts(&result.communities, planted);
            (
                Some(adjusted_rand_index(&result.communities, planted)),
                adjusted_mutual_information(&result.communities, planted).ok(),
                Some(canonical_memberships(&result.communities) == canonical_memberships(planted)),
                Some(merges),
                Some(splits),
            )
        });
    ProfileReport {
        algorithm: result.identity.algorithm.clone(),
        topology: result.identity.topology.clone(),
        selector: result.identity.selector.clone(),
        selected_candidate_resolution: result
            .quality
            .candidate_summaries
            .iter()
            .find(|candidate| candidate.selected)
            .map_or(result.quality.resolution, |candidate| candidate.resolution),
        communities: result.communities.len(),
        disconnected_communities: result.quality.disconnected_community_count,
        modularity: result.quality.modularity,
        weighted_mean_conductance: result.quality.weighted_mean_conductance,
        worst_conductance: result.quality.worst_conductance,
        largest_community_fraction: result.quality.largest_community_fraction,
        non_isolate_singletons: result.quality.non_isolate_singleton_count,
        quality_visits: result.quality.quality_visit_count,
        adjusted_rand_index: ari,
        adjusted_mutual_information: ami,
        exact_recovery: exact,
        false_merges,
        false_splits,
    }
}

fn canonical_memberships(communities: &Communities) -> Vec<Vec<String>> {
    let mut memberships = communities.values().cloned().collect::<Vec<_>>();
    for members in &mut memberships {
        members.sort();
    }
    memberships.sort();
    memberships
}

fn merge_split_counts(detected: &Communities, planted: &Communities) -> (usize, usize) {
    let planted_assignment = assignment(planted);
    let detected_assignment = assignment(detected);
    let false_merges = detected
        .values()
        .filter(|members| {
            members
                .iter()
                .filter_map(|member| planted_assignment.get(member))
                .collect::<BTreeSet<_>>()
                .len()
                > 1
        })
        .count();
    let false_splits = planted
        .values()
        .filter(|members| {
            members
                .iter()
                .filter_map(|member| detected_assignment.get(member))
                .collect::<BTreeSet<_>>()
                .len()
                > 1
        })
        .count();
    (false_merges, false_splits)
}

fn assignment(communities: &Communities) -> BTreeMap<String, usize> {
    communities
        .iter()
        .flat_map(|(community, members)| {
            members
                .iter()
                .map(move |member| (member.clone(), *community))
        })
        .collect()
}

fn fixtures() -> Vec<Fixture> {
    let mut output = vec![
        dense_bridge(),
        ring_of_cliques(),
        articulation(),
        stars_and_hubs(),
        directed_fan(),
        reciprocal_and_one_way(),
        parallel_confidence(),
        containment_dominated(),
        isolates_and_subsystem(),
        large_weak_community(),
        layered_code(),
        tests_and_documentation(),
    ];
    output.extend([0, 2, 4].map(lfr_style));
    output
}

fn dense_bridge() -> Fixture {
    let mut edges = clique(0, 5, EdgeKind::Calls);
    edges.extend(clique(5, 10, EdgeKind::Calls));
    edges.push(exact(4, 5, EdgeKind::Calls));
    graph_fixture("dense-groups-one-bridge", 10, edges, groups(&[5, 5]), true)
}

fn ring_of_cliques() -> Fixture {
    let mut edges = Vec::new();
    let group_count = 12;
    for group in 0..group_count {
        edges.extend(clique(group * 3, group * 3 + 3, EdgeKind::Calls));
        edges.push(exact(
            group * 3 + 2,
            ((group + 1) % group_count) * 3,
            EdgeKind::References,
        ));
    }
    graph_fixture(
        "ring-of-cliques",
        group_count * 3,
        edges,
        groups(&[3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3]),
        true,
    )
}

fn articulation() -> Fixture {
    let mut edges = clique(0, 4, EdgeKind::Calls);
    edges.extend(clique(4, 8, EdgeKind::Calls));
    edges.extend([
        exact(3, 8, EdgeKind::References),
        exact(8, 4, EdgeKind::Calls),
    ]);
    graph_fixture("articulation", 9, edges, groups(&[4, 5]), true)
}

fn stars_and_hubs() -> Fixture {
    let mut edges = (1..8)
        .map(|node| exact(0, node, EdgeKind::Calls))
        .collect::<Vec<_>>();
    edges.extend((9..16).map(|node| exact(8, node, EdgeKind::Imports)));
    edges.push(exact(0, 8, EdgeKind::References));
    graph_fixture("stars-and-multi-hubs", 16, edges, None, false)
}

fn directed_fan() -> Fixture {
    let mut edges = (1..6)
        .map(|node| exact(0, node, EdgeKind::Publishes))
        .collect::<Vec<_>>();
    edges.extend((7..12).map(|node| exact(node, 6, EdgeKind::Subscribes)));
    graph_fixture("directed-fan-in-out", 12, edges, None, false)
}

fn reciprocal_and_one_way() -> Fixture {
    graph_fixture(
        "reciprocal-versus-one-way",
        6,
        vec![
            exact(0, 1, EdgeKind::Calls),
            exact(1, 0, EdgeKind::Calls),
            exact(1, 2, EdgeKind::Calls),
            exact(3, 4, EdgeKind::Calls),
            exact(4, 5, EdgeKind::Calls),
        ],
        groups(&[3, 3]),
        false,
    )
}

fn parallel_confidence() -> Fixture {
    let mut edges = vec![
        exact(0, 1, EdgeKind::Calls),
        inferred(0, 1, EdgeKind::Calls),
    ];
    edges.extend((0..5).map(|_| exact(1, 2, EdgeKind::Calls)));
    edges.push(EdgeSpec {
        source: 2,
        target: 3,
        kind: EdgeKind::Calls,
        confidence: EvidenceConfidence::Ambiguous,
    });
    graph_fixture("parallel-confidence-evidence", 4, edges, None, false)
}

fn containment_dominated() -> Fixture {
    let mut edges = (1..10)
        .map(|node| exact(0, node, EdgeKind::Contains))
        .collect::<Vec<_>>();
    edges.extend(clique(1, 5, EdgeKind::Calls));
    edges.extend(clique(5, 10, EdgeKind::Calls));
    graph_fixture(
        "containment-dominated-files",
        10,
        edges,
        groups(&[1, 4, 5]),
        false,
    )
}

fn isolates_and_subsystem() -> Fixture {
    graph_fixture(
        "isolates-and-connected-subsystems",
        9,
        clique(0, 6, EdgeKind::Calls),
        groups(&[6, 1, 1, 1]),
        false,
    )
}

fn large_weak_community() -> Fixture {
    let edges = (0..39)
        .map(|node| exact(node, node + 1, EdgeKind::References))
        .collect();
    graph_fixture("large-weak-community", 40, edges, groups(&[40]), false)
}

fn layered_code() -> Fixture {
    let mut edges = clique(0, 4, EdgeKind::RoutesTo);
    edges.extend(clique(4, 8, EdgeKind::Calls));
    edges.extend(clique(8, 12, EdgeKind::Reads));
    for node in 0..4 {
        edges.push(exact(node, node + 4, EdgeKind::Calls));
        edges.push(exact(node + 4, node + 8, EdgeKind::Calls));
    }
    graph_fixture(
        "layered-handler-domain-repository",
        12,
        edges,
        groups(&[4, 4, 4]),
        false,
    )
}

fn tests_and_documentation() -> Fixture {
    let mut edges = clique(0, 6, EdgeKind::Calls);
    edges.extend((6..9).map(|node| exact(node, node - 6, EdgeKind::Tests)));
    edges.extend((9..12).map(|node| exact(node, node - 9, EdgeKind::Documents)));
    graph_fixture("tests-and-documentation", 12, edges, groups(&[12]), false)
}

fn lfr_style(mixing: usize) -> Fixture {
    let group_size = 8;
    let group_count = 4;
    let mut edges = Vec::new();
    for group in 0..group_count {
        let start = group * group_size;
        for left in start..start + group_size {
            for right in left + 1..start + group_size {
                if (left * 31 + right * 17 + group) % 3 != 0 {
                    edges.push(exact(left, right, EdgeKind::Calls));
                }
            }
        }
        for offset in 0..mixing {
            edges.push(inferred(
                start + offset,
                ((group + 1) % group_count) * group_size + offset,
                EdgeKind::DependsOn,
            ));
        }
    }
    graph_fixture(
        &format!("lfr-style-mixing-{mixing}"),
        group_size * group_count,
        edges,
        groups(&[group_size, group_size, group_size, group_size]),
        mixing == 0,
    )
}

fn clique(start: usize, end: usize, kind: EdgeKind) -> Vec<EdgeSpec> {
    (start..end)
        .flat_map(|left| (left + 1..end).map(move |right| exact(left, right, kind)))
        .collect()
}

fn exact(source: usize, target: usize, kind: EdgeKind) -> EdgeSpec {
    EdgeSpec {
        source,
        target,
        kind,
        confidence: EvidenceConfidence::Exact,
    }
}

fn inferred(source: usize, target: usize, kind: EdgeKind) -> EdgeSpec {
    EdgeSpec {
        source,
        target,
        kind,
        confidence: EvidenceConfidence::Inferred,
    }
}

fn groups(sizes: &[usize]) -> Option<Communities> {
    let mut start = 0;
    Some(
        sizes
            .iter()
            .enumerate()
            .map(|(community, size)| {
                let members = (start..start + size)
                    .map(|node| format!("node-{node:03}"))
                    .collect();
                start += size;
                (community, members)
            })
            .collect(),
    )
}

fn graph_fixture(
    name: &str,
    node_count: usize,
    edges: Vec<EdgeSpec>,
    planted: Option<Communities>,
    exact_recovery_required: bool,
) -> Fixture {
    let mut document = compass_model::code_graph::GraphDocument::empty_v1(BuildMetadata {
        builder_version: "community-quality-qualification".to_owned(),
        schema_fingerprint: "fixture-v1".to_owned(),
        source_tree_digest: name.to_owned(),
        configuration_digest: "quality-v1".to_owned(),
        generation_id: format!("fixture:{name}"),
        source_commit: None,
    });
    document.nodes = (0..node_count)
        .map(|index| NodeRecord {
            id: format!("node-{index:03}"),
            kind: NodeKind::Function,
            roles: Vec::new(),
            name: format!("node_{index}"),
            qualified_name: format!("fixture::{name}::node_{index}"),
            language: Some("rust".to_owned()),
            framework: None,
            source: Some(anchor(name, index)),
            details: None,
            evidence: Vec::new(),
            coverage: Vec::new(),
            diagnostics: Vec::new(),
            community: None,
        })
        .collect();
    document.links = edges
        .into_iter()
        .enumerate()
        .map(|(index, edge)| EdgeRecord {
            id: format!("edge-{index:04}"),
            key: format!("edge-{index:04}"),
            source: format!("node-{:03}", edge.source),
            target: format!("node-{:03}", edge.target),
            kind: edge.kind,
            occurrence_rule: None,
            relationship_site: Some(anchor(name, index + node_count)),
            details: None,
            evidence: vec![Provenance {
                origin: EvidenceOrigin::Ast,
                extractor: "community-quality-fixture".to_owned(),
                confidence: edge.confidence,
                rule: None,
                anchors: Vec::new(),
                wiring_site: None,
                score: None,
                candidates: Vec::new(),
            }],
            weight: Some(1.0),
            context: None,
            deferred: false,
            diagnostics: Vec::new(),
        })
        .collect();
    Fixture {
        name: name.to_owned(),
        document,
        planted,
        exact_recovery_required,
    }
}

fn anchor(name: &str, index: usize) -> SourceAnchor {
    let byte = u64::try_from(index).unwrap_or(u64::MAX - 1);
    let line = u32::try_from(index.saturating_add(1)).unwrap_or(u32::MAX);
    SourceAnchor {
        file: format!("fixtures/{name}.rs"),
        start_byte: byte,
        end_byte: byte.saturating_add(1),
        start_line: line,
        start_column: 0,
        end_line: line,
        end_column: 1,
    }
}
