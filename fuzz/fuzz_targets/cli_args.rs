#![no_main]

use std::ffi::OsString;

use libfuzzer_sys::fuzz_target;
use compass_cli::{Frontend, run};

const GRAPH: &[u8] = br#"{"directed":true,"multigraph":false,"graph":{},"nodes":[{"id":"a","label":"Alpha"},{"id":"b","label":"Beta"}],"links":[{"source":"a","target":"b","relation":"calls","confidence":"EXTRACTED"}]}"#;

fuzz_target!(|data: &[u8]| {
    if data.len() > 16_384 {
        return;
    }
    let Ok(directory) = tempfile::tempdir() else {
        return;
    };
    let graph = directory.path().join("graph.json");
    if std::fs::write(&graph, GRAPH).is_err() {
        return;
    }
    let text = String::from_utf8_lossy(data);
    let value = text.chars().take(256).collect::<String>();
    let graph_text = graph.to_string_lossy().into_owned();
    let command = data.first().copied().unwrap_or_default() % 5;
    let args = match command {
        0 => vec!["query".to_owned(), value, "--graph".to_owned(), graph_text],
        1 => vec![
            "path".to_owned(),
            value,
            "b".to_owned(),
            "--graph".to_owned(),
            graph_text,
        ],
        2 => vec![
            "explain".to_owned(),
            value,
            "--graph".to_owned(),
            graph_text,
        ],
        3 => vec![
            "affected".to_owned(),
            value,
            "--graph".to_owned(),
            graph_text,
        ],
        _ => vec!["benchmark".to_owned(), graph_text],
    };
    let graphify = args.iter().cloned().map(OsString::from).collect::<Vec<_>>();
    let compass = graphify.iter().cloned().collect::<Vec<_>>();
    let _ = run(Frontend::Graphify, graphify);
    let _ = run(Frontend::Compass, compass);
});
