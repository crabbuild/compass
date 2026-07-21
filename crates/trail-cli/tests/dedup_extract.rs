use std::error::Error;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Value;

const BACKEND_ENVIRONMENT: &[&str] = &[
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
    "OPENAI_API_KEY",
    "ANTHROPIC_API_KEY",
    "DEEPSEEK_API_KEY",
    "MOONSHOT_API_KEY",
    "AWS_PROFILE",
    "AWS_REGION",
    "AWS_DEFAULT_REGION",
    "AWS_ACCESS_KEY_ID",
    "OLLAMA_BASE_URL",
];

#[test]
fn dedup_llm_without_backend_matches_python_diagnostic() -> Result<(), Box<dyn Error>> {
    let corpus = tempfile::tempdir()?;
    fs::write(corpus.path().join("main.py"), "def main():\n    return 1\n")?;
    let mut command = Command::new(env!("CARGO_BIN_EXE_trail"));
    command
        .args([
            "graph",
            "extract",
            corpus.path().to_str().ok_or("non-UTF-8 corpus path")?,
            "--code-only",
            "--dedup-llm",
            "--no-viz",
        ])
        .current_dir(corpus.path())
        .env("HOME", corpus.path())
        .env("USERPROFILE", corpus.path());
    for key in BACKEND_ENVIRONMENT {
        command.env_remove(key);
    }
    let output = command.output()?;
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr)?,
        "error: no LLM API key found (--dedup-llm was passed). Set GEMINI_API_KEY or GOOGLE_API_KEY (gemini), MOONSHOT_API_KEY (kimi), ANTHROPIC_API_KEY (claude), OPENAI_API_KEY (openai), DEEPSEEK_API_KEY (deepseek), or pass --backend. A code-only corpus needs no key.\n"
    );
    Ok(())
}

#[test]
fn dedup_llm_resolves_ambiguous_semantic_entities() -> Result<(), Box<dyn Error>> {
    let corpus = tempfile::tempdir()?;
    let account = corpus.path().join("account.md");
    let identity = corpus.path().join("identity.md");
    fs::write(&account, "# Customer Account Management\n")?;
    fs::write(&identity, "# Customer Identity Management\n")?;

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    let saw_tiebreak = Arc::new(AtomicBool::new(false));
    let server_saw_tiebreak = Arc::clone(&saw_tiebreak);
    let account_source = account.to_string_lossy().into_owned();
    let identity_source = identity.to_string_lossy().into_owned();
    let server = std::thread::spawn(move || -> Result<(), String> {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().map_err(|error| error.to_string())?;
            let request = read_json_request(&mut stream)?;
            let prompt = request.to_string();
            let content = if prompt.contains("For each pair below") {
                server_saw_tiebreak.store(true, Ordering::SeqCst);
                "1. yes".to_owned()
            } else {
                serde_json::to_string(&serde_json::json!({
                    "nodes": [
                        {
                            "id": "customer_account_management",
                            "label": "Customer Account Management",
                            "file_type": "concept",
                            "source_file": account_source,
                            "rationale": "The source names this customer concept."
                        },
                        {
                            "id": "customer_identity_management",
                            "label": "Customer Identity Management",
                            "file_type": "concept",
                            "source_file": identity_source,
                            "rationale": "The source names this customer concept."
                        }
                    ],
                    "edges": [],
                    "hyperedges": []
                }))
                .map_err(|error| error.to_string())?
            };
            write_openai_response(&mut stream, &content)?;
        }
        Ok(())
    });

    let output = Command::new(env!("CARGO_BIN_EXE_trail"))
        .args([
            "graph",
            "extract",
            corpus.path().to_str().ok_or("non-UTF-8 corpus path")?,
            "--backend",
            "openai",
            "--model",
            "trail-test",
            "--dedup-llm",
            "--no-viz",
        ])
        .env("OPENAI_API_KEY", "test-key")
        .env("OPENAI_BASE_URL", format!("http://{address}"))
        .output()?;
    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    server
        .join()
        .map_err(|_| "provider thread panicked")?
        .map_err(|error| format!("provider failed: {error}"))?;
    assert!(saw_tiebreak.load(Ordering::SeqCst));

    let graph: Value = serde_json::from_slice(&fs::read(
        corpus.path().join("graphify-out").join("graph.json"),
    )?)?;
    let surviving = graph["nodes"]
        .as_array()
        .ok_or("graph nodes are not an array")?
        .iter()
        .filter(|node| {
            matches!(
                node["id"].as_str(),
                Some("customer_account_management" | "customer_identity_management")
            )
        })
        .count();
    assert_eq!(surviving, 1, "ambiguous concepts were not merged: {graph}");
    Ok(())
}

fn read_json_request(stream: &mut TcpStream) -> Result<Value, String> {
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
    serde_json::from_slice(&request[header_end..header_end + content_length])
        .map_err(|error| error.to_string())
}

fn write_openai_response(stream: &mut TcpStream, content: &str) -> Result<(), String> {
    let response = serde_json::json!({
        "choices": [{"message": {"content": content}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 17, "completion_tokens": 9}
    });
    let body = serde_json::to_vec(&response).map_err(|error| error.to_string())?;
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .map_err(|error| error.to_string())?;
    stream.write_all(&body).map_err(|error| error.to_string())
}
