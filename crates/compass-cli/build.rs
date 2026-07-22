use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

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
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let root = manifest.join("assets");
    let mut files = Vec::new();
    collect(&root, &root, &mut files)?;
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
