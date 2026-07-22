#![no_main]

use std::io::Write;

use libfuzzer_sys::fuzz_target;
use compass_languages::Engine;

const EXTENSIONS: &[&str] = &[
    "py", "js", "ts", "tsx", "go", "rs", "java", "groovy", "c", "cpp", "cs", "kt", "scala", "php",
    "swift", "lua", "zig", "ps1", "ex", "m", "jl", "f90", "vue", "svelte", "astro", "dart", "sv",
    "sql", "pas", "sh", "json", "tf", "dm", "sln", "csproj", "xaml", "razor", "cls",
];

fuzz_target!(|data: &[u8]| {
    if data.is_empty() || data.len() > 262_144 {
        return;
    }
    let extension = EXTENSIONS[usize::from(data[0]) % EXTENSIONS.len()];
    let Ok(mut source) = tempfile::Builder::new()
        .prefix("compass-fuzz-source-")
        .suffix(&format!(".{extension}"))
        .tempfile()
    else {
        return;
    };
    if source.write_all(&data[1..]).is_err() {
        return;
    }
    let mut engine = Engine::default();
    let _ = engine.extract(source.path());
});
