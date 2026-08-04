use std::error::Error;
use std::fs::{self, OpenOptions};
use std::time::{Duration, Instant};

use compass_core::{BuildOptions, build_local_graph};
use compass_model::code_graph::{CoverageStatus, ExtractionStatus, GraphDocument};

const SOURCE_FILES: usize = 300;
const OVERSIZED_SOURCE_BYTES: u64 = 8 * 1024 * 1024;
const COLD_CEILING: Duration = Duration::from_secs(60);
const WARM_CEILING: Duration = Duration::from_secs(10);

#[test]
fn cold_and_warm_in_process_builds_stay_within_enterprise_ceiling() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let source_root = directory.path().join("src");
    fs::create_dir_all(&source_root)?;
    for index in 0..SOURCE_FILES {
        fs::write(
            source_root.join(format!("module_{index:03}.rs")),
            format!("pub fn function_{index:03}(value: u64) -> u64 {{ value + {index} }}\n"),
        )?;
    }
    OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(source_root.join("generated.rs"))?
        .set_len(OVERSIZED_SOURCE_BYTES)?;

    let mut options = BuildOptions::new(directory.path());
    options.no_cluster = true;
    options.no_viz = true;
    options.program_analysis = true;
    options.max_workers = Some(2);
    options.max_source_bytes = 64 * 1024;
    options.built_at_commit = Some("0123456789012345678901234567890123456789".to_owned());

    let cold_started = Instant::now();
    let cold = build_local_graph(&options)?;
    let cold_elapsed = cold_started.elapsed();
    let cold_graph = fs::read(cold.output_dir.join("graph.json"))?;
    let graph = GraphDocument::load(&cold.output_dir.join("graph.json"))?;
    assert_eq!(cold.files_considered, SOURCE_FILES + 1);
    assert_eq!(cold.files_extracted, SOURCE_FILES);
    let oversized = graph
        .graph
        .files
        .iter()
        .find(|file| file.path == "src/generated.rs")
        .ok_or("missing oversized source inventory")?;
    assert_eq!(oversized.byte_size, OVERSIZED_SOURCE_BYTES);
    assert_eq!(oversized.extraction_status, ExtractionStatus::Partial);
    assert!(graph.graph.coverage.iter().any(|coverage| {
        coverage.file_id.as_deref() == Some(oversized.id.as_str())
            && coverage.status == CoverageStatus::Partial
    }));
    assert!(graph.nodes.iter().all(|node| {
        node.source
            .as_ref()
            .is_none_or(|source| source.file != "src/generated.rs")
    }));
    assert!(graph.links.iter().all(|edge| {
        edge.relationship_site
            .as_ref()
            .is_none_or(|anchor| anchor.file != "src/generated.rs")
    }));

    let warm_started = Instant::now();
    let warm = build_local_graph(&options)?;
    let warm_elapsed = warm_started.elapsed();
    assert_eq!(fs::read(warm.output_dir.join("graph.json"))?, cold_graph);
    assert_eq!(warm.files_extracted, 0);
    assert_eq!(warm.files_cached, SOURCE_FILES + 1);

    assert!(
        cold_elapsed < COLD_CEILING,
        "in-process cold build took {cold_elapsed:?}, exceeding {COLD_CEILING:?}"
    );
    assert!(
        warm_elapsed < WARM_CEILING,
        "in-process warm build took {warm_elapsed:?}, exceeding {WARM_CEILING:?}"
    );
    assert!(
        warm_elapsed <= cold_elapsed + Duration::from_secs(2),
        "warm build regressed materially: cold={cold_elapsed:?}, warm={warm_elapsed:?}"
    );
    println!(
        "{{\"sources\":{},\"oversizedBytes\":{OVERSIZED_SOURCE_BYTES},\"oversizedCoverage\":\"partial\",\"coldMs\":{},\"warmMs\":{}}}",
        SOURCE_FILES + 1,
        cold_elapsed.as_millis(),
        warm_elapsed.as_millis()
    );
    Ok(())
}

#[test]
fn code_only_build_excludes_structural_documents() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    fs::write(
        directory.path().join("service.rs"),
        "pub fn service() -> u64 { 1 }\n",
    )?;
    fs::write(
        directory.path().join("README.md"),
        "# Service\n\nThis document is outside the code-only profile.\n",
    )?;

    let mut options = BuildOptions::new(directory.path());
    options.code_only = true;
    options.no_cluster = true;
    options.no_viz = true;
    let result = build_local_graph(&options)?;
    let graph = GraphDocument::load(&result.output_dir.join("graph.json"))?;

    assert_eq!(result.files_considered, 1);
    assert_eq!(result.files_extracted, 1);
    assert!(
        graph
            .nodes
            .iter()
            .all(|node| node.source_file() != Some("README.md"))
    );
    Ok(())
}
