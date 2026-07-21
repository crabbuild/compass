//! Statically linked deterministic language extraction for Trail.

mod apex;
mod bash;
mod builtins;
mod config;
mod cpp;
mod csharp;
mod dart;
mod dm;
mod dotnet_project;
mod elixir;
mod engine;
mod facts;
mod fortran;
mod go;
mod groovy;
mod ids;
mod json_config;
mod julia;
mod markdown;
mod mcp;
mod objc;
mod package_manifest;
mod pascal;
mod pascal_forms;
mod php;
mod powershell;
mod registry;
mod rust_lang;
mod scip;
mod sql;
mod swift;
mod templates;
mod terraform;
mod verilog;
mod xaml;
mod zig;

pub use facts::{Extraction, RawCall};
pub use ids::{file_stem, make_id, normalize_id};
pub use registry::{ExtractorKind, LanguageSpec, Registry};
pub use scip::{ScipExtraction, ingest_scip_json};

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum ExtractError {
    #[error("unsupported deterministic source format: {0}")]
    Unsupported(PathBuf),
    #[error("grammar {language} is not statically linked: {detail}")]
    MissingGrammar { language: String, detail: String },
    #[error("parser returned no syntax tree for {0}")]
    ParseCancelled(PathBuf),
    #[error(transparent)]
    File(#[from] trail_files::FileError),
}
pub use engine::Engine;

/// Extract deterministic SQL facts from in-memory content.
///
/// Live schema introspectors use a virtual path so credentials and temporary
/// files never enter the graph.
#[must_use]
pub fn extract_sql_content(path: &std::path::Path, content: &[u8]) -> Extraction {
    sql::extract(path, content)
}
