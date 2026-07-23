use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[path = "../../tools/skillgen/mod.rs"]
mod skillgen;

fn collect(root: &Path, directory: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    let mut entries = fs::read_dir(directory)?
        .map(|entry| entry.map(|value| value.path()))
        .collect::<io::Result<Vec<_>>>()?;
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect(root, &path, files)?;
        } else if path.is_file() {
            files.push(
                path.strip_prefix(root)
                    .map_err(io::Error::other)?
                    .to_path_buf(),
            );
        }
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=assets");
    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=src/help.rs");
    println!("cargo:rerun-if-changed=../../tools/skillgen");
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let root = manifest.join("assets");
    skillgen::validate(
        &root,
        &manifest.join("src/lib.rs"),
        &manifest.join("src/help.rs"),
    )?;
    let mut files = Vec::new();
    for directory in ["compass-skill", "compass-integrations"] {
        collect(&root, &root.join(directory), &mut files)?;
    }
    let mut generated = String::from("static EMBEDDED_ASSETS: &[EmbeddedAsset] = &[\n");
    for relative in files {
        let source = root.join(&relative);
        generated.push_str(&format!(
            "    EmbeddedAsset {{ path: {:?}, bytes: include_bytes!({:?}) }},\n",
            relative.to_string_lossy().replace('\\', "/"),
            source
        ));
    }
    generated.push_str("];\n");
    let output = PathBuf::from(env::var("OUT_DIR")?);
    fs::write(output.join("install_assets.rs"), generated)?;
    Ok(())
}
