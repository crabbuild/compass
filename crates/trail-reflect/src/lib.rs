//! Deterministic reflection over Trail/Graphify session memory.

mod aggregate;
mod memory;
mod overlay;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub use aggregate::{
    Aggregate, ContestedSource, Correction, Counts, DeadEnd, ProvenanceEvent, SourceScore,
    aggregate_lessons, render_lessons_markdown,
};
pub use memory::{MemoryDoc, load_memory_docs, parse_memory_doc};
pub use overlay::{LEARNING_SIDECAR_NAME, build_learning_overlay, write_learning_sidecar};
use time::OffsetDateTime;
use trail_files::{FileError, write_text_atomic};

pub const DEFAULT_HALF_LIFE_DAYS: f64 = 30.0;
pub const DEFAULT_MIN_CORROBORATION: usize = 2;

#[derive(Debug, thiserror::Error)]
pub enum ReflectError {
    #[error("could not inspect {path}: {source}")]
    Inspect {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("could not create {path}: {source}")]
    Create {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("could not write {path}: {source}")]
    Write { path: PathBuf, source: FileError },
}

#[derive(Clone, Debug)]
pub struct ReflectOptions {
    pub memory_dir: PathBuf,
    pub output: PathBuf,
    pub graph: Option<PathBuf>,
    pub analysis: Option<PathBuf>,
    pub labels: Option<PathBuf>,
    pub now: OffsetDateTime,
    pub half_life_days: f64,
    pub min_corroboration: usize,
}

#[derive(Clone, Debug)]
pub struct ReflectResult {
    pub output: PathBuf,
    pub aggregate: Aggregate,
}

pub fn reflect(options: &ReflectOptions) -> Result<ReflectResult, ReflectError> {
    let docs = load_memory_docs(&options.memory_dir);
    let graph_context = options.graph.as_deref().map(|graph| {
        let analysis = options.analysis.clone().unwrap_or_else(|| {
            graph
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(".graphify_analysis.json")
        });
        let labels = options.labels.clone().unwrap_or_else(|| {
            graph
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(".graphify_labels.json")
        });
        aggregate::load_graph_context(graph, &analysis, &labels)
    });
    let aggregate = aggregate_lessons(
        &docs,
        graph_context
            .as_ref()
            .and_then(|context| context.node_community.as_ref()),
        graph_context
            .as_ref()
            .and_then(|context| context.known_nodes.as_ref()),
        options.now,
        options.half_life_days,
        options.min_corroboration,
    );
    let parent = options
        .output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source| ReflectError::Create {
        path: parent.to_path_buf(),
        source,
    })?;
    write_text_atomic(&options.output, &render_lessons_markdown(&aggregate)).map_err(|source| {
        ReflectError::Write {
            path: options.output.clone(),
            source,
        }
    })?;
    if let Some(graph) = options.graph.as_deref() {
        let _ = write_learning_sidecar(&aggregate, graph, options.now);
    }
    Ok(ReflectResult {
        output: options.output.clone(),
        aggregate,
    })
}

#[must_use]
pub fn lessons_fresh(
    output: &Path,
    memory_dir: &Path,
    graph: Option<&Path>,
    analysis: Option<&Path>,
    labels: Option<&Path>,
) -> bool {
    let Ok(output_time) = output.metadata().and_then(|metadata| metadata.modified()) else {
        return false;
    };
    let mut newest = SystemTime::UNIX_EPOCH;
    if memory_dir.is_dir()
        && let Ok(entries) = fs::read_dir(memory_dir)
    {
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("md") {
                continue;
            }
            if let Ok(modified) = path.metadata().and_then(|metadata| metadata.modified()) {
                newest = newest.max(modified);
            }
        }
    }
    for path in [graph, analysis, labels].into_iter().flatten() {
        if let Ok(modified) = path.metadata().and_then(|metadata| metadata.modified()) {
            newest = newest.max(modified);
        }
    }
    output_time >= newest
}
