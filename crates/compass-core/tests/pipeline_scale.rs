use std::error::Error;
use std::fs;
use std::time::{Duration, Instant};

use compass_core::{BuildOptions, build_local_graph};

const SOURCE_FILES: usize = 300;
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

    let mut options = BuildOptions::new(directory.path());
    options.no_cluster = true;
    options.no_viz = true;
    options.program_analysis = true;
    options.max_workers = Some(2);
    options.built_at_commit = Some("0123456789012345678901234567890123456789".to_owned());

    let cold_started = Instant::now();
    let cold = build_local_graph(&options)?;
    let cold_elapsed = cold_started.elapsed();
    let cold_graph = fs::read(cold.output_dir.join("graph.json"))?;
    assert_eq!(cold.files_considered, SOURCE_FILES);
    assert_eq!(cold.files_extracted, SOURCE_FILES);

    let warm_started = Instant::now();
    let warm = build_local_graph(&options)?;
    let warm_elapsed = warm_started.elapsed();
    assert_eq!(fs::read(warm.output_dir.join("graph.json"))?, cold_graph);
    assert_eq!(warm.files_extracted, 0);
    assert_eq!(warm.files_cached, SOURCE_FILES);

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
    Ok(())
}
