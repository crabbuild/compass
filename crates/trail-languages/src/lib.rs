//! Statically linked deterministic language extraction for Trail.

mod bash;
mod config;
mod engine;
mod facts;
mod go;
mod ids;
mod registry;
mod rust_lang;

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
