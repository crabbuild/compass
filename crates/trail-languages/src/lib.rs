//! Statically linked deterministic language extraction for Trail.

mod bash;
mod builtins;
mod config;
mod cpp;
mod csharp;
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
mod pascal_forms;
mod php;
mod powershell;
mod registry;
mod rust_lang;
mod swift;
mod terraform;
mod verilog;
mod xaml;
mod zig;

pub use facts::{Extraction, RawCall};
pub use ids::{file_stem, make_id, normalize_id};
pub use registry::{ExtractorKind, LanguageSpec, Registry};

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
