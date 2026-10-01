//! Rebuild a production qualification graph using unchanged validated AST facts.
use compass_core::{BuildOptions, GraphStorage, InferenceLevel, build_local_graph};
use std::error::Error;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args_os().skip(1);
    let root = PathBuf::from(args.next().ok_or("source root required")?);
    let output = PathBuf::from(args.next().ok_or("output root required")?);
    if args.next().is_some() {
        return Err("expected source and output roots".into());
    }
    let mut options = BuildOptions::new(&root);
    options.output_root = Some(output);
    options.force = true;
    options.reuse_cache_on_force = true;
    options.code_only = true;
    options.inference_level = InferenceLevel::Max;
    options.graph_storage = GraphStorage::Json;
    options.extra_excludes = vec!["tests/**".to_owned()];
    options.no_cluster = true;
    options.no_viz = true;
    options.max_workers = Some(2);
    let result = build_local_graph(&options)?;
    println!("{}", result.output_dir.display());
    Ok(())
}
