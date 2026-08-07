use std::error::Error;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;

use serde_json::Value;

#[test]
fn cache_check_publishes_only_visible_compass_output_names() -> Result<(), Box<dyn Error>> {
    let corpus = tempfile::tempdir()?;
    fs::write(corpus.path().join("main.rs"), "fn main() {}\n")?;
    fs::write(corpus.path().join("files.txt"), "main.rs\n")?;

    let output = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args([
            "cache-check",
            "files.txt",
            "--root",
            corpus.path().to_str().ok_or("non-UTF-8 corpus path")?,
        ])
        .current_dir(corpus.path())
        .env_remove("COMPASS_OUT")
        .output()?;
    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let compass_output = corpus.path().join("compass-out");
    assert_eq!(
        fs::read_to_string(compass_output.join("uncached.txt"))?,
        "main.rs"
    );
    for entry in fs::read_dir(&compass_output)? {
        let name = entry?.file_name();
        let name = name.to_string_lossy();
        assert!(!name.starts_with(".compass") && !name.starts_with("compass-"));
    }
    Ok(())
}

#[test]
fn native_semantic_extract_uses_provider_then_runs_warm_without_network()
-> Result<(), Box<dyn Error>> {
    let corpus = tempfile::tempdir()?;
    let guide = corpus.path().join("guide.md");
    fs::write(&guide, "# Guide\n\nThe domain rule coordinates requests.\n")?;
    fs::write(corpus.path().join("main.py"), "def main():\n    return 1\n")?;

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    let source = guide.to_string_lossy().into_owned();
    let server = std::thread::spawn(move || -> Result<(), String> {
        let (mut stream, _) = listener.accept().map_err(|error| error.to_string())?;
        let mut request = Vec::new();
        let mut buffer = [0_u8; 4096];
        let header_end = loop {
            let read = stream
                .read(&mut buffer)
                .map_err(|error| error.to_string())?;
            if read == 0 {
                return Err("provider request ended before its headers".to_owned());
            }
            request.extend_from_slice(&buffer[..read]);
            if let Some(position) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                break position + 4;
            }
        };
        let headers = String::from_utf8_lossy(&request[..header_end]);
        if !headers.starts_with("POST /chat/completions ") {
            return Err(format!("unexpected request: {headers}"));
        }
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .ok_or_else(|| "missing Content-Length".to_owned())?;
        while request.len() - header_end < content_length {
            let read = stream
                .read(&mut buffer)
                .map_err(|error| error.to_string())?;
            if read == 0 {
                return Err("provider request body ended early".to_owned());
            }
            request.extend_from_slice(&buffer[..read]);
        }
        let request_body: Value =
            serde_json::from_slice(&request[header_end..header_end.saturating_add(content_length)])
                .map_err(|error| error.to_string())?;
        if request_body["model"] != "compass-test" {
            return Err(format!("unexpected model: {}", request_body["model"]));
        }
        let fragment = serde_json::json!({
            "nodes": [{
                "id": "guide_domain_rule",
                "label": "Domain rule",
                "file_type": "concept",
                "source_file": source,
                "rationale": "The guide explicitly states that the domain rule coordinates incoming requests."
            }],
            "edges": [],
            "hyperedges": []
        });
        let response = serde_json::json!({
            "choices": [{
                "message": {"content": serde_json::to_string(&fragment).map_err(|error| error.to_string())?},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 17, "completion_tokens": 9}
        });
        let body = serde_json::to_vec(&response).map_err(|error| error.to_string())?;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .map_err(|error| error.to_string())?;
        stream.write_all(&body).map_err(|error| error.to_string())?;
        Ok(())
    });

    let first = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args([
            "extract",
            corpus.path().to_str().ok_or("non-UTF-8 corpus path")?,
            "--backend",
            "openai",
            "--model",
            "compass-test",
            "--no-viz",
        ])
        .env("OPENAI_API_KEY", "test-key")
        .env("OPENAI_BASE_URL", format!("http://{address}"))
        .output()?;
    assert!(
        first.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr)
    );
    server
        .join()
        .map_err(|_| "provider thread panicked")?
        .map_err(|error| format!("provider failed: {error}"))?;

    let output = corpus.path().join("compass-out");
    let active = compass_files::BuildGuard::resolve_current_snapshot_directory(&output)?;
    let graph: Value = serde_json::from_slice(&fs::read(active.join("graph.json"))?)?;
    assert!(
        graph["nodes"].as_array().is_some_and(|nodes| {
            nodes.iter().any(|node| {
                node["name"] == "Domain rule"
                    && node["kind"] == "resource"
                    && node["details"]["type"] == "resource"
                    && node["details"]["data"]["resourceKind"] == "concept"
                    && node["evidence"].as_array().is_some_and(|evidence| {
                        evidence.iter().any(|item| {
                            item["origin"] == "heuristic"
                                && item["rule"] == "semantic-extraction"
                                && item["wiringSite"]["file"] == "guide.md"
                        })
                    })
            })
        }),
        "{graph:#}"
    );
    let analysis: Value = serde_json::from_slice(&fs::read(active.join("analysis.json"))?)?;
    assert_eq!(
        analysis["tokens"],
        serde_json::json!({"input":17,"output":9})
    );
    let marker: Value = serde_json::from_slice(&fs::read(active.join("semantic-marker.json"))?)?;
    assert_eq!(marker["output_tokens"], 9);
    let manifest: Value = serde_json::from_slice(&fs::read(active.join("manifest.json"))?)?;
    assert!(
        manifest["guide.md"]["semantic_hash"]
            .as_str()
            .is_some_and(|hash| !hash.is_empty())
    );

    let warm = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args([
            "extract",
            corpus.path().to_str().ok_or("non-UTF-8 corpus path")?,
            "--backend",
            "openai",
            "--model",
            "compass-test",
            "--no-viz",
        ])
        .env("OPENAI_API_KEY", "test-key")
        .env("OPENAI_BASE_URL", "http://127.0.0.1:9")
        .output()?;
    assert!(
        warm.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&warm.stdout),
        String::from_utf8_lossy(&warm.stderr)
    );
    let warm_graph_path = compass_files::BuildGuard::resolve_artifact(&output, "graph.json")?;
    let warm_graph: Value = serde_json::from_slice(&fs::read(warm_graph_path)?)?;
    assert!(warm_graph["nodes"].as_array().is_some_and(|nodes| {
        nodes
            .iter()
            .any(|node| node["name"] == "Domain rule" && node["kind"] == "resource")
    }));
    Ok(())
}
