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

fn canonical_text(input: &str) -> String {
    input.replace("\r\n", "\n").replace('\r', "\n")
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "cargo:rustc-env=COMPASS_BUILD_TARGET={}",
        env::var("TARGET")?
    );
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
    let output = PathBuf::from(env::var("OUT_DIR")?);
    let embedded_root = output.join("install-assets");
    let mut generated = String::from("static EMBEDDED_ASSETS: &[EmbeddedAsset] = &[\n");
    for relative in files {
        let source = root.join(&relative);
        let embedded = embedded_root.join(&relative);
        if let Some(parent) = embedded.parent() {
            fs::create_dir_all(parent)?;
        }
        let body = fs::read_to_string(&source)?;
        fs::write(&embedded, canonical_text(&body))?;
        generated.push_str(&format!(
            "    EmbeddedAsset {{ path: {:?}, bytes: include_bytes!({:?}) }},\n",
            relative.to_string_lossy().replace('\\', "/"),
            embedded
        ));
    }
    generated.push_str("];\n");
    fs::write(output.join("install_assets.rs"), generated)?;
    Ok(())
}
