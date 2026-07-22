#![no_main]

use libfuzzer_sys::fuzz_target;
use compass_files::{DetectOptions, Manifest, ManifestKind};

fuzz_target!(|data: &[u8]| {
    if data.len() > 262_144 {
        return;
    }
    let Ok(directory) = tempfile::tempdir() else {
        return;
    };
    let manifest_path = directory.path().join("manifest.json");
    if std::fs::write(&manifest_path, data).is_err()
        || std::fs::write(directory.path().join("input.py"), b"def fuzz(): pass\n").is_err()
    {
        return;
    }
    let manifest = Manifest::load(&manifest_path, Some(directory.path()));
    let _ = manifest.entries();
    let _ = Manifest::incremental(
        directory.path(),
        &manifest_path,
        &DetectOptions::default(),
        ManifestKind::Ast,
    );
});
