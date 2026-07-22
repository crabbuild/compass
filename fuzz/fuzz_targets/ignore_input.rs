#![no_main]

use libfuzzer_sys::fuzz_target;
use compass_files::{DetectOptions, WatchPathFilter, detect};

fuzz_target!(|data: &[u8]| {
    if data.len() > 131_072 {
        return;
    }
    let Ok(directory) = tempfile::tempdir() else {
        return;
    };
    let root = directory.path();
    let split = data.len() / 2;
    if std::fs::write(root.join(".graphifyignore"), &data[..split]).is_err()
        || std::fs::write(root.join(".gitignore"), &data[split..]).is_err()
        || std::fs::create_dir_all(root.join("src/nested")).is_err()
        || std::fs::write(root.join("src/nested/input.py"), b"def fuzz(): pass\n").is_err()
    {
        return;
    }
    let options = DetectOptions::default();
    let _ = detect(root, &options);
    if let Ok(filter) = WatchPathFilter::new(root, &options) {
        let _ = filter.allows(&root.join("src/nested/input.py"));
        let _ = filter.allows(&root.join("graphify-out/graph.json"));
    }
});
