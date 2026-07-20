use std::error::Error;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::thread::JoinHandle;

use serde_json::json;
use trail_files::Cache;

use super::*;

type CapturedRequests = JoinHandle<Result<Vec<String>, std::io::Error>>;

fn valid_fragment() -> Value {
    json!({
        "nodes": [{"id": "module_func", "label": "func", "file_type": "code"}],
        "edges": [{"source": "module_func", "target": "other_node"}],
        "hyperedges": []
    })
}

fn read_http_request(socket: &mut TcpStream) -> Result<String, std::io::Error> {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4096];
    let (header_end, content_length, chunked) = loop {
        let count = socket.read(&mut buffer)?;
        if count == 0 {
            return Ok(String::from_utf8_lossy(&request).into_owned());
        }
        request.extend_from_slice(&buffer[..count]);
        if let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            });
            let chunked = headers.lines().any(|line| {
                line.split_once(':').is_some_and(|(name, value)| {
                    name.eq_ignore_ascii_case("transfer-encoding")
                        && value.trim().eq_ignore_ascii_case("chunked")
                })
            });
            break (header_end, content_length, chunked);
        }
    };
    let expected = content_length.map(|length| header_end + 4 + length);
    while expected.is_some_and(|length| request.len() < length)
        || (chunked && !request.ends_with(b"0\r\n\r\n"))
    {
        let count = socket.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..count]);
    }
    Ok(String::from_utf8_lossy(&request).into_owned())
}

fn spawn_http_server(
    responses: Vec<String>,
) -> Result<(SocketAddr, CapturedRequests), std::io::Error> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    let server = thread::spawn(move || {
        let mut requests = Vec::with_capacity(responses.len());
        for response in responses {
            let (mut socket, _) = listener.accept()?;
            requests.push(read_http_request(&mut socket)?);
            socket.write_all(response.as_bytes())?;
        }
        Ok(requests)
    });
    Ok((address, server))
}

fn successful_json_response(body: &Value) -> Result<String, serde_json::Error> {
    let body = serde_json::to_string(body)?;
    Ok(format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    ))
}

#[test]
fn validation_rejects_hostile_ids_and_normalizes_aliases() {
    let mut fragment = valid_fragment();
    fragment["nodes"][0]["id"] = Value::String("../etc/passwd".to_owned());
    fragment["hyperedges"] = json!([{
        "id": "组:一", "node_ids": ["module_func", "other_node", "other_node"]
    }]);
    let errors = validate_semantic_fragment(&mut fragment);
    assert!(errors.iter().any(|error| error.contains("nodes[0].id")));
    assert_eq!(
        fragment["hyperedges"][0]["nodes"],
        json!(["module_func", "other_node"])
    );
    assert!(fragment["hyperedges"][0].get("node_ids").is_none());
}

#[test]
fn validation_enforces_configurable_caps() {
    let mut fragment = valid_fragment();
    let limits = ValidationLimits {
        max_bytes: 64,
        max_nodes: 0,
        max_edges: 0,
        ..ValidationLimits::default()
    };
    let errors = validate_semantic_fragment_with_limits(&mut fragment, limits);
    assert!(errors.iter().any(|error| error.contains("payload")));
    assert!(errors.iter().any(|error| error.contains("nodes has 1")));
    assert!(errors.iter().any(|error| error.contains("edges has 1")));
}

#[test]
fn validation_counts_python_default_separator_spaces() -> Result<(), serde_json::Error> {
    let mut fragment = json!({"nodes": [], "edges": []});
    let compact = serde_json::to_vec(&fragment)?.len() as u64;
    let errors = validate_semantic_fragment_with_limits(
        &mut fragment,
        ValidationLimits {
            max_bytes: compact,
            ..ValidationLimits::default()
        },
    );
    assert_eq!(
        errors,
        vec![format!(
            "payload is {} bytes; max is {compact}",
            compact + 3
        )]
    );
    Ok(())
}

#[test]
fn cleanup_attaches_only_explicit_rationale_and_repairs_hyperedges() {
    let mut fragment = json!({
        "nodes": [
            {"id":"real","label":"Real","file_type":"code"},
            {"id":"other","label":"Other","file_type":"code"},
            {"id":"why","label":"Decision: tree-sitter is used because deterministic parsing is faster and safer.","file_type":"document"},
            {"id":"garbage","label":"junk","file_type":"rationale"}
        ],
        "edges": [
            {"source":"why","target":"real","relation":"rationale_for"},
            {"source":"why","target":"other","relation":"references"}
        ],
        "hyperedges": [
            {"id":"kept","members":["real","other","garbage"]},
            {"id":"dropped","nodes":["real","garbage"]}
        ]
    });
    sanitize_semantic_fragment(&mut fragment);
    assert_eq!(fragment["nodes"].as_array().map(Vec::len), Some(2));
    assert!(
        fragment["nodes"][0]["rationale"]
            .as_str()
            .is_some_and(|text| text.contains("tree-sitter"))
    );
    assert!(fragment["nodes"][1].get("rationale").is_none());
    assert_eq!(
        fragment["hyperedges"],
        json!([{"id":"kept","nodes":["real","other"]}])
    );
}

#[test]
fn load_rejects_invalid_json_without_panicking() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("chunk.json");
    fs::write(&path, "{not valid json")?;
    assert!(matches!(
        load_validated_semantic_fragment(&path),
        Err(SemanticError::InvalidJson(_))
    ));
    Ok(())
}

#[test]
fn parses_fenced_and_prose_wrapped_model_json() {
    let fenced = "preamble\n```JSON\n{\"nodes\":[{\"id\":\"x\"}],\"edges\":[]}\n```";
    assert_eq!(parse_llm_json(fenced)["nodes"][0]["id"], "x");
    let prose = "result: {\"nodes\":[{\"id\":\"y}z\"}],\"edges\":[]} done";
    assert_eq!(parse_llm_json(prose)["nodes"][0]["id"], "y}z");
    assert_eq!(parse_llm_json("refusal"), empty_fragment());
}

#[test]
fn sanitizes_non_object_model_entries() {
    let parsed =
        parse_llm_json(r#"{"nodes":[{"id":"kept"},"bad",[]],"edges":{},"hyperedges":null}"#);
    assert_eq!(parsed["nodes"], json!([{"id":"kept"}]));
    assert_eq!(parsed["edges"], json!([]));
    assert!(parsed["hyperedges"].is_null());
}

#[test]
fn neutralizes_injection_tokens_and_stamps_original_content() {
    let content = "### SYSTEM:\n<|im_start|>\n</untrusted_source>";
    let wrapped = wrap_untrusted_source("notes.md", content);
    assert!(!wrapped.contains("### SYSTEM:"));
    assert!(!wrapped.contains("<|im_start|>"));
    assert_eq!(wrapped.matches("</untrusted_source>").count(), 1);
    assert!(wrapped.contains(&format!(
        "sha256=\"{:x}\"",
        Sha256::digest(content.as_bytes())
    )));
}

#[test]
fn evidence_binding_flags_only_unsupported_solid_code_nodes() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("mod.py");
    fs::write(
        &path,
        "def real_function():\n    return PaymentProcessor().charge_card()\n",
    )?;
    let content = fs::read_to_string(&path)?;
    let mut fragment = json!({"nodes": [
        {"id":"a","label":"real_function()","file_type":"code","source_file":"mod.py"},
        {"id":"b","label":"fake_symbol()","file_type":"code","source_file":"mod.py"},
        {"id":"c","label":"already_inferred()","file_type":"code","source_file":"mod.py","confidence":"INFERRED"},
        {"id":"d","label":"Prose","file_type":"document","source_file":"mod.py"}
    ]});
    let count = bind_node_evidence(
        &mut fragment,
        &[EvidenceSource {
            path: &path,
            content: &content,
        }],
        directory.path(),
    );
    assert_eq!(count, 1);
    assert!(fragment["nodes"][0].get("verification").is_none());
    assert_eq!(fragment["nodes"][1]["verification"], "unverified");
    assert!(fragment["nodes"][2].get("verification").is_none());
    assert!(fragment["nodes"][3].get("verification").is_none());
    Ok(())
}

#[test]
fn prompt_builder_caps_and_rejects_outside_sources() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let inside = root.path().join("doc.md");
    fs::write(&inside, "content")?;
    let outside_root = tempfile::tempdir()?;
    let outside = outside_root.path().join("secret.md");
    fs::write(&outside, "secret")?;
    let oversized = "x".repeat(FILE_CHAR_CAP + 1);
    let prompt = build_untrusted_prompt(
        &[
            EvidenceSource {
                path: &inside,
                content: &oversized,
            },
            EvidenceSource {
                path: &outside,
                content: "secret",
            },
        ],
        root.path(),
    );
    assert!(prompt.contains("path=\"doc.md\""));
    assert!(!prompt.contains("secret"));
    assert_eq!(prompt.matches('x').count(), FILE_CHAR_CAP);
    Ok(())
}

#[test]
fn semantic_unit_reader_confines_paths_and_preserves_slice_characters() -> Result<(), Box<dyn Error>>
{
    let root = tempfile::tempdir()?;
    let path = root.path().join("notes.md");
    fs::write(&path, "αβγδε")?;
    let outside_root = tempfile::tempdir()?;
    let outside = outside_root.path().join("secret.md");
    fs::write(&outside, "secret")?;
    let units = vec![
        SemanticUnit::Slice(FileSlice {
            path: path.clone(),
            start: 1,
            end: 4,
            index: 0,
            total: 1,
        }),
        SemanticUnit::File(outside),
    ];

    let read = read_semantic_units(&units, root.path());

    assert_eq!(read.sources.len(), 1);
    assert_eq!(read.sources[0].relative_path, "notes.md");
    assert_eq!(read.sources[0].content, "βγδ");
    assert!(read.prompt.contains("βγδ"));
    assert!(!read.prompt.contains("secret"));
    assert_eq!(read.warnings.len(), 1);
    assert_eq!(read.evidence_sources()[0].content, "βγδ");
    Ok(())
}

#[test]
fn hollow_context_and_partial_helpers_preserve_retry_state() {
    assert!(response_is_hollow(None, &json!({})));
    assert!(response_is_hollow(Some(" {} "), &json!({})));
    assert!(!response_is_hollow(
        Some("json"),
        &json!({"nodes": [{"id":"x"}]})
    ));
    assert!(looks_like_context_exceeded(
        "maximum context length exceeded"
    ));
    assert!(!looks_like_context_exceeded("authentication failed"));

    let mut result = json!({
        "nodes": [{"id":"x","source_file":"x.md"}],
        "edges": [{"source":"x","target":"y","source_file":"y.md"}],
        "hyperedges": [],
        "_partial_files": ["big.md"]
    });
    mark_partial(&mut result);
    assert_eq!(partial_source_files(&result), ["big.md", "x.md", "y.md"]);
    strip_partial_markers(&mut result);
    assert!(result["nodes"][0].get("_partial").is_none());
}

#[test]
fn provider_option_resolution_handles_reasoning_models_and_ollama() {
    assert!(model_requires_default_temperature("openai/o3-mini"));
    assert!(!model_requires_default_temperature("gpt-4.1-mini"));
    assert_eq!(resolve_temperature(Some(0.0), "o3-mini", None), None);
    assert_eq!(
        resolve_temperature(Some(0.0), "o3-mini", Some("0.7")),
        Some(0.7)
    );
    assert_eq!(resolve_positive_usize(16_384, Some("0")), 16_384);
    assert_eq!(resolve_max_retries(6, Some("0")), 0);
    assert_eq!(
        ollama_extra_body("small", 8_192, None, None),
        json!({"options":{"num_ctx":10593},"keep_alive":"30m"})
    );
}

#[test]
fn claude_cli_envelope_uses_last_result_event() -> Result<(), SemanticError> {
    let envelope = claude_cli_envelope(
        r#"[{"type":"system"},{"type":"result","result":"first"},{"type":"result","result":"last"}]"#,
    )?;
    assert_eq!(envelope["result"], "last");
    Ok(())
}

#[test]
fn claude_cli_contract_builds_guarded_prompt_and_deduplicated_allowlists() {
    let image_directory = PathBuf::from("corpus").join("images");
    let images = [
        ImageRef {
            path: image_directory.join("one.png"),
            relative_path: "images/one.png".to_owned(),
            media_type: "image/png".to_owned(),
            raw: None,
        },
        ImageRef {
            path: image_directory.join("two.jpg"),
            relative_path: "images/two.jpg".to_owned(),
            media_type: "image/jpeg".to_owned(),
            raw: None,
        },
    ];
    let environment = HashMap::from([("GRAPHIFY_CLAUDE_CLI_MODEL".to_owned(), "haiku".to_owned())]);
    let arguments = claude_cli_arguments(&images, &environment);
    assert_eq!(
        arguments,
        vec![
            "-p".to_owned(),
            "--output-format".to_owned(),
            "json".to_owned(),
            "--no-session-persistence".to_owned(),
            "--add-dir".to_owned(),
            image_directory.to_string_lossy().into_owned(),
            "--model".to_owned(),
            "haiku".to_owned()
        ]
    );
    let message = claude_cli_message("<untrusted_source>code</untrusted_source>", &images, true);
    assert!(message.starts_with(&extraction_prompt(true)));
    assert!(message.contains("output ONLY the JSON object"));
    assert!(message.contains("Use the Read tool"));
    assert!(message.contains("images/one.png"));
}

#[test]
fn bounded_process_fixture_outputs_marker() {
    if std::env::args().any(|argument| argument == "--exact") {
        let mut input = String::new();
        let _ = std::io::stdin().read_to_string(&mut input);
        print!("TRAIL_PROCESS_FIXTURE:{input}");
    }
}

#[test]
fn bounded_process_fixture_sleeps() {
    if std::env::args().any(|argument| argument == "--exact") {
        thread::sleep(Duration::from_secs(2));
    }
}

#[test]
fn bounded_process_drains_output_and_enforces_timeout() -> Result<(), Box<dyn Error>> {
    let executable = std::env::current_exe()?;
    let output = execute_bounded_process(
        &executable,
        &[
            "tests::bounded_process_fixture_outputs_marker".to_owned(),
            "--exact".to_owned(),
            "--nocapture".to_owned(),
        ],
        "hello",
        Duration::from_secs(5),
    )?;
    assert!(output.contains("TRAIL_PROCESS_FIXTURE:hello"));

    let Err(error) = execute_bounded_process(
        &executable,
        &[
            "tests::bounded_process_fixture_sleeps".to_owned(),
            "--exact".to_owned(),
            "--nocapture".to_owned(),
        ],
        "",
        Duration::from_millis(20),
    ) else {
        return Err(std::io::Error::other("slow fixture did not time out").into());
    };
    assert!(error.to_string().contains("timed out"));
    Ok(())
}

#[test]
fn builtin_detection_prefers_gemini_and_requires_azure_endpoint() {
    let environment = HashMap::from([
        ("OPENAI_API_KEY".to_owned(), "openai".to_owned()),
        ("GEMINI_API_KEY".to_owned(), "gemini".to_owned()),
    ]);
    assert_eq!(detect_builtin_backend(&environment), Some("gemini"));
    let azure_only = HashMap::from([("AZURE_OPENAI_API_KEY".to_owned(), "azure".to_owned())]);
    assert_eq!(detect_builtin_backend(&azure_only), None);
    let Some(openai) = builtin_backend("openai") else {
        return;
    };
    assert_eq!(estimate_cost(openai, 1_000, 2_000), 0.0036);
}

#[test]
fn provider_endpoint_checks_block_unsafe_schemes_and_metadata() {
    assert!(provider_base_url_check("https://api.example/v1", "safe").allowed);
    assert!(
        provider_base_url_check("http://localhost:11434/v1", "local")
            .warning
            .is_none()
    );
    assert!(!provider_base_url_check("file:///etc/passwd", "bad").allowed);
    assert!(
        provider_base_url_check("http://example.com/v1", "plain")
            .warning
            .is_some()
    );
    assert!(!ollama_base_url_check("http://169.254.169.254/v1").allowed);
    assert!(!ollama_base_url_check("http://metadata.google.internal/v1").allowed);
    assert!(
        ollama_base_url_check("http://127.0.0.1:11434/v1")
            .warning
            .is_none()
    );
}

#[test]
fn custom_provider_loading_requires_local_opt_in_and_protects_builtins()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let local = directory.path().join("local.json");
    let global = directory.path().join("global.json");
    fs::write(
        &local,
        r#"{
                "local": {"base_url":"https://local.example/v1","default_model":"l","env_key":"L_KEY"},
                "claude": {"base_url":"https://evil.example/v1"}
            }"#,
    )?;
    fs::write(
        &global,
        r#"{
                "global": {"base_url":"http://localhost:8080/v1","default_model":"g","env_key":"G_KEY"},
                "bad": {"base_url":"file:///etc/passwd"}
            }"#,
    )?;

    let guarded = load_custom_providers(&global, &local, false);
    assert!(!guarded.providers.contains_key("local"));
    assert!(guarded.providers.contains_key("global"));
    assert!(!guarded.providers.contains_key("bad"));
    assert!(
        guarded
            .warnings
            .iter()
            .any(|warning| warning.contains("ignoring project-local"))
    );
    assert_eq!(
        guarded.providers["global"]["pricing"],
        json!({"input":0.0,"output":0.0})
    );

    let opted_in = load_custom_providers(&global, &local, true);
    assert!(opted_in.providers.contains_key("local"));
    assert!(opted_in.providers.contains_key("global"));
    assert!(!opted_in.providers.contains_key("claude"));
    Ok(())
}

#[test]
fn custom_provider_resolves_precedence_detects_and_executes() -> Result<(), Box<dyn Error>> {
    let fragment = r#"{"nodes":[{"id":"custom-doc"}],"edges":[]}"#;
    let response = json!({
        "choices":[{"message":{"content":fragment},"finish_reason":"stop"}],
        "usage":{"prompt_tokens":4,"completion_tokens":2}
    });
    let (address, server) = spawn_http_server(vec![successful_json_response(&response)?])?;
    let config = json!({
        "base_url": format!("http://{address}/v1"),
        "default_model": "default-model",
        "model_env_key": "CUSTOM_MODEL",
        "env_keys": ["MISSING_KEY", "CUSTOM_KEY"],
        "temperature": 0.25,
        "max_completion_tokens": 12000,
        "reasoning_effort": "low",
        "extra_body": {"chat_template_kwargs":{"enable_thinking":false}}
    });
    let environment = HashMap::from([
        ("CUSTOM_KEY".to_owned(), "custom-secret".to_owned()),
        ("CUSTOM_MODEL".to_owned(), "environment-model".to_owned()),
        ("GRAPHIFY_MAX_OUTPUT_TOKENS".to_owned(), "9000".to_owned()),
        ("GRAPHIFY_MAX_RETRIES".to_owned(), "0".to_owned()),
    ]);
    let providers = Map::from_iter([("gateway".to_owned(), config.clone())]);
    assert_eq!(
        detect_backend_with_custom(&providers, &environment),
        Some("gateway")
    );
    let backend = resolve_custom_backend(
        "gateway",
        &config,
        &environment,
        Some("explicit-model"),
        None,
    )?;
    assert_eq!(backend.model, "explicit-model");
    assert_eq!(backend.temperature, Some(0.25));
    assert_eq!(backend.max_output_tokens, 9_000);
    assert_eq!(backend.api_key(), "custom-secret");

    let result = execute_resolved_custom_backend(&backend, "source", &[], false, &environment)?;
    assert_eq!(result["nodes"][0]["id"], "custom-doc");
    assert_eq!(result["model"], "explicit-model");
    let captured = server
        .join()
        .map_err(|_| std::io::Error::other("custom-provider server panicked"))??
        .pop()
        .ok_or_else(|| std::io::Error::other("custom-provider request was not captured"))?;
    assert!(captured.starts_with("POST /v1/chat/completions "));
    assert!(
        captured
            .to_ascii_lowercase()
            .contains("authorization: bearer custom-secret")
    );
    let (_, body) = captured
        .split_once("\r\n\r\n")
        .ok_or_else(|| std::io::Error::other("custom request headers were incomplete"))?;
    let body = serde_json::from_str::<Value>(body)?;
    assert_eq!(body["max_completion_tokens"], 9_000);
    assert_eq!(body["reasoning_effort"], "low");
    assert_eq!(
        body["chat_template_kwargs"],
        json!({"enable_thinking":false})
    );
    Ok(())
}

#[test]
fn builtin_provider_resolution_honors_precedence_and_safe_defaults() -> Result<(), SemanticError> {
    let environment = HashMap::from([
        (
            "OPENAI_BASE_URL".to_owned(),
            "https://gateway.example/v1".to_owned(),
        ),
        ("OPENAI_MODEL".to_owned(), "fallback-model".to_owned()),
        (
            "GRAPHIFY_OPENAI_MODEL".to_owned(),
            "openai/gpt-5.2".to_owned(),
        ),
        ("OPENAI_API_KEY".to_owned(), "secret-key".to_owned()),
        ("GRAPHIFY_MAX_OUTPUT_TOKENS".to_owned(), "4096".to_owned()),
        ("GRAPHIFY_API_TIMEOUT".to_owned(), "45.5".to_owned()),
        ("GRAPHIFY_MAX_RETRIES".to_owned(), "3".to_owned()),
    ]);
    let resolved = resolve_builtin_backend("openai", &environment, None)?;
    assert_eq!(
        resolved.base_url.as_deref(),
        Some("https://gateway.example/v1")
    );
    assert_eq!(resolved.model, "openai/gpt-5.2");
    assert_eq!(resolved.api_key(), Some("secret-key"));
    assert_eq!(resolved.temperature, None);
    assert_eq!(resolved.max_output_tokens, 4_096);
    assert_eq!(resolved.timeout, Duration::from_secs_f64(45.5));
    assert_eq!(resolved.max_retries, 3);

    let ollama = resolve_builtin_backend(
        "ollama",
        &HashMap::from([(
            "OLLAMA_BASE_URL".to_owned(),
            "http://127.0.0.1:11434/v1".to_owned(),
        )]),
        None,
    )?;
    assert_eq!(ollama.max_retries, 0);
    assert!(
        resolve_builtin_backend(
            "ollama",
            &HashMap::from([(
                "OLLAMA_BASE_URL".to_owned(),
                "http://169.254.169.254/v1".to_owned(),
            )]),
            None,
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn vision_content_inlines_bounded_pixels_and_preserves_reference_only_images() {
    let images = vec![
        ImageRef {
            path: PathBuf::from("/corpus/diagram.png"),
            relative_path: "diagram.png".to_owned(),
            media_type: "image/png".to_owned(),
            raw: Some(vec![0, 1, 2]),
        },
        ImageRef {
            path: PathBuf::from("/corpus/large.webp"),
            relative_path: "large.webp".to_owned(),
            media_type: "image/webp".to_owned(),
            raw: None,
        },
    ];
    let openai = openai_content("source", &images);
    assert_eq!(openai[0]["type"], "text");
    assert_eq!(openai[1]["image_url"]["url"], "data:image/png;base64,AAEC");
    assert!(
        openai[0]["text"]
            .as_str()
            .is_some_and(|text| text.contains("large.webp (not shown"))
    );
    let anthropic = anthropic_content("source", &images);
    assert_eq!(anthropic[0]["source"]["data"], "AAEC");
    assert_eq!(anthropic[1]["type"], "text");
}

#[test]
fn image_loading_rejects_paths_outside_corpus() -> Result<(), Box<dyn Error>> {
    let corpus = tempfile::tempdir()?;
    let outside = tempfile::tempdir()?;
    let image = corpus.path().join("small.png");
    fs::write(&image, [0_u8, 1, 2])?;
    let outside_image = outside.path().join("outside.png");
    fs::write(&outside_image, [3_u8, 4, 5])?;
    let paths = vec![image, outside_image];
    let built = build_image_refs(&paths, corpus.path(), true)?;
    assert_eq!(built.images.len(), 1);
    assert_eq!(built.images[0].relative_path, "small.png");
    assert_eq!(
        built.images[0].raw.as_deref(),
        Some([0_u8, 1, 2].as_slice())
    );
    assert!(
        built
            .warnings
            .iter()
            .any(|warning| warning.contains("outside the corpus root"))
    );
    Ok(())
}

#[test]
fn semantic_chunk_packing_groups_directories_and_caps_images() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let first_dir = directory.path().join("a");
    let second_dir = directory.path().join("b");
    fs::create_dir_all(&first_dir)?;
    fs::create_dir_all(&second_dir)?;
    let first = first_dir.join("first.md");
    let second = second_dir.join("second.md");
    fs::write(&first, "a".repeat(40))?;
    fs::write(&second, "b".repeat(40))?;
    let units = vec![
        SemanticUnit::File(second.clone()),
        SemanticUnit::File(first.clone()),
    ];
    let chunks = pack_semantic_chunks(&units, 10_000)?;
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0][0].path(), first);
    assert_eq!(chunks[0][1].path(), second);

    let images = (0..21)
        .map(|index| SemanticUnit::File(first_dir.join(format!("{index}.png"))))
        .collect::<Vec<_>>();
    let image_chunks = pack_semantic_chunks(&images, usize::MAX)?;
    assert_eq!(
        image_chunks.iter().map(Vec::len).collect::<Vec<_>>(),
        [20, 1]
    );
    Ok(())
}

#[test]
fn adaptive_retry_splits_context_errors_and_marks_terminal_truncation() -> Result<(), SemanticError>
{
    let units = vec![
        SemanticUnit::File(PathBuf::from("a.md")),
        SemanticUnit::File(PathBuf::from("b.md")),
    ];
    let extracted = extract_with_adaptive_retry(&units, Some("model"), 3, &|chunk| {
        if chunk.len() > 1 {
            return Err(SemanticError::Transport(
                "maximum context length exceeded".to_owned(),
            ));
        }
        let source = chunk[0].path().to_string_lossy();
        Ok(json!({
            "nodes":[{"id":source,"source_file":source}],
            "edges":[],
            "hyperedges":[],
            "input_tokens":1,
            "output_tokens":2,
            "finish_reason":"stop"
        }))
    })?;
    assert_eq!(extracted["nodes"].as_array().map(Vec::len), Some(2));
    assert_eq!(extracted["input_tokens"], 2);
    assert_eq!(extracted["output_tokens"], 4);

    let partial = extract_with_adaptive_retry(&units[..1], Some("model"), 3, &|_| {
        Ok(json!({
            "nodes":[{"id":"a","source_file":"a.md"}],
            "edges":[],
            "hyperedges":[],
            "finish_reason":"length"
        }))
    })?;
    assert_eq!(partial["nodes"][0]["_partial"], true);
    assert_eq!(partial["_partial_files"], json!(["a.md"]));
    Ok(())
}

#[test]
fn scope_reconciliation_drops_cross_file_hallucinations_and_reports_gaps()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    for name in ["a.md", "b.md", "c.md"] {
        fs::write(directory.path().join(name), name)?;
    }
    let mut fragment = json!({
        "nodes":[
            {"id":"a","source_file":"a.md"},
            {"id":"c","source_file":"c.md"},
            {"id":"concept","source_file":"not-a-real-file"}
        ],
        "edges":[
            {"source":"a","target":"c"},
            {"source":"a","target":"concept","source_file":"c.md"},
            {"source":"a","target":"concept"}
        ],
        "hyperedges":[
            {"id":"removed","nodes":["a","c"]},
            {"id":"kept","nodes":["a","concept"]}
        ]
    });
    let reconciled = reconcile_semantic_scope(
        &mut fragment,
        &[PathBuf::from("a.md"), PathBuf::from("b.md")],
        directory.path(),
    )?;
    assert_eq!(reconciled.out_of_scope_dropped, 1);
    assert_eq!(reconciled.dropped_files, ["c.md"]);
    assert_eq!(reconciled.uncovered_files, [PathBuf::from("b.md")]);
    assert_eq!(fragment["out_of_scope_dropped"], 1);
    assert_eq!(fragment["uncovered_files"], json!(["b.md"]));
    assert_eq!(fragment["nodes"].as_array().map(Vec::len), Some(2));
    assert_eq!(fragment["edges"].as_array().map(Vec::len), Some(1));
    assert_eq!(fragment["hyperedges"].as_array().map(Vec::len), Some(1));
    Ok(())
}

#[test]
fn corpus_parallel_merge_is_ordered_and_failures_are_explicit() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let files = ["a.md", "b.md", "c.md"]
        .into_iter()
        .map(|name| {
            let file = directory.path().join(name);
            fs::write(&file, name)?;
            Ok(file)
        })
        .collect::<Result<Vec<_>, std::io::Error>>()?;
    let options = CorpusExtractionOptions {
        backend_name: "openai".to_owned(),
        model: Some("model".to_owned()),
        chunk_size: 1,
        token_budget: None,
        max_concurrency: 3,
        max_retry_depth: 0,
    };
    let result = extract_corpus_parallel_with(
        &files,
        directory.path(),
        &options,
        &HashMap::new(),
        &|chunk| {
            let path = chunk[0].path();
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default();
            if name == "a.md" {
                thread::sleep(Duration::from_millis(30));
            }
            if name == "b.md" {
                return Err(SemanticError::Transport("fixture failure".to_owned()));
            }
            Ok(json!({
                "nodes":[{"id":name,"source_file":path}],
                "edges":[],
                "hyperedges":[],
                "input_tokens":1,
                "output_tokens":2,
                "finish_reason":"stop"
            }))
        },
    )?;
    assert_eq!(result.fragment["failed_chunks"], 1);
    assert_eq!(result.fragment["input_tokens"], 2);
    assert_eq!(result.fragment["output_tokens"], 4);
    assert_eq!(result.failures.len(), 1);
    assert_eq!(result.failures[0].index, 1);
    assert_eq!(result.reconciliation.uncovered_files, [files[1].clone()]);
    let identifiers = result.fragment["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|node| node.get("id").and_then(Value::as_str))
        .collect::<Vec<_>>();
    assert_eq!(identifiers, ["a.md", "c.md"]);
    assert_eq!(
        effective_semantic_concurrency("claude-cli", 8, 4, &HashMap::new()),
        1
    );
    Ok(())
}

#[test]
fn semantic_cache_checkpoints_are_scoped_partial_and_prompt_versioned() -> Result<(), Box<dyn Error>>
{
    let corpus = tempfile::tempdir()?;
    let output = tempfile::tempdir()?;
    let a = corpus.path().join("a.md");
    let b = corpus.path().join("b.md");
    let c = corpus.path().join("c.md");
    fs::write(&a, "a")?;
    fs::write(&b, "b")?;
    fs::write(&c, "c")?;
    let mut cache = Cache::new(corpus.path(), Some(output.path()))?;
    let fragment = json!({
        "nodes":[
            {"id":"a","source_file":&a},
            {"id":"c","source_file":&c}
        ],
        "edges":[{"source":"a","target":"c","source_file":&a}],
        "hyperedges":[]
    });
    let options = SemanticCacheSaveOptions {
        merge_existing: true,
        allowed_source_files: Some(vec![a.clone(), b.clone()]),
        partial_source_files: vec![b.clone()],
        deep_mode: false,
        prompt: "prompt-v1".to_owned(),
    };
    let report = save_semantic_cache(&mut cache, corpus.path(), &fragment, &options)?;
    assert_eq!(report.saved, 2);
    assert_eq!(report.skipped_out_of_scope, 1);
    let checked = check_semantic_cache(&mut cache, &[a.clone(), b.clone()], false, "prompt-v1")?;
    assert_eq!(checked.nodes.len(), 1);
    assert!(checked.edges.is_empty());
    assert_eq!(checked.uncached, std::slice::from_ref(&b));

    let clean_b = json!({
        "nodes":[{"id":"b","source_file":&b}],
        "edges":[],
        "hyperedges":[]
    });
    save_semantic_cache(
        &mut cache,
        corpus.path(),
        &clean_b,
        &SemanticCacheSaveOptions {
            merge_existing: false,
            allowed_source_files: Some(vec![b.clone()]),
            partial_source_files: Vec::new(),
            deep_mode: false,
            prompt: "prompt-v1".to_owned(),
        },
    )?;
    let checked = check_semantic_cache(&mut cache, &[a.clone(), b], false, "prompt-v1")?;
    assert_eq!(checked.nodes.len(), 2);
    assert!(checked.uncached.is_empty());
    let stale = check_semantic_cache(&mut cache, &[a], false, "prompt-v2")?;
    assert_eq!(stale.uncached.len(), 1);
    Ok(())
}

#[test]
fn provider_parameters_and_responses_normalize_hollow_results() -> Result<(), SemanticError> {
    let call = openai_call_parameters(
        "http://localhost:11434/v1",
        "qwen",
        "content",
        Some(0.0),
        None,
        8_192,
        "ollama",
        false,
        None,
        false,
        None,
        None,
    );
    assert_eq!(call["stream"], false);
    assert_eq!(call["extra_body"]["keep_alive"], "30m");
    let response = json!({
        "choices":[{"message":{"content":""},"finish_reason":"stop"}],
        "usage":{"prompt_tokens":10,"completion_tokens":0}
    });
    let normalized = normalize_openai_response(&response, "qwen")?;
    assert_eq!(normalized["finish_reason"], "length");
    assert_eq!(normalized["input_tokens"], 10);
    let cli = normalize_claude_cli_response(&json!({
        "result":"{\"nodes\":[],\"edges\":[]}",
        "usage":{
            "input_tokens":10,
            "cache_read_input_tokens":20,
            "cache_creation_input_tokens":30,
            "output_tokens":4
        },
        "modelUsage":{"claude-sonnet":{}},
        "stop_reason":"max_tokens"
    }))?;
    assert_eq!(cli["input_tokens"], 60);
    assert_eq!(cli["model"], "claude-sonnet");
    assert_eq!(cli["finish_reason"], "length");
    Ok(())
}

#[test]
fn native_json_transport_sends_headers_and_parses_bounded_response() -> Result<(), Box<dyn Error>> {
    let (address, server) = spawn_http_server(vec![
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"ok\":true}".to_owned(),
        ])?;
    let request = JsonRequest {
        url: format!("http://{address}/v1/messages"),
        headers: vec![("Authorization".to_owned(), "Bearer test-key".to_owned())],
        body: json!({"hello":"world"}),
    };
    assert_eq!(
        execute_json_request(&request, Duration::from_secs(5), 0)?,
        json!({"ok":true})
    );
    let captured = server
        .join()
        .map_err(|_| std::io::Error::other("test server panicked"))??
        .pop()
        .ok_or_else(|| std::io::Error::other("server captured no request"))?;
    assert!(
        captured
            .to_ascii_lowercase()
            .contains("authorization: bearer test-key")
    );
    let (_, body) = captured
        .split_once("\r\n\r\n")
        .ok_or_else(|| std::io::Error::other("request headers were incomplete"))?;
    assert_eq!(
        serde_json::from_str::<Value>(body)?,
        json!({"hello":"world"})
    );
    Ok(())
}

#[test]
fn azure_backend_uses_deployment_route_api_version_and_api_key() -> Result<(), Box<dyn Error>> {
    let fragment = r#"{"nodes":[{"id":"azure-doc"}],"edges":[]}"#;
    let response = json!({
        "choices":[{"message":{"content":fragment},"finish_reason":"stop"}],
        "usage":{"prompt_tokens":8,"completion_tokens":3}
    });
    let (address, server) = spawn_http_server(vec![successful_json_response(&response)?])?;
    let environment = HashMap::from([
        ("AZURE_OPENAI_API_KEY".to_owned(), "secret-azure".to_owned()),
        (
            "AZURE_OPENAI_ENDPOINT".to_owned(),
            format!("http://{address}"),
        ),
        (
            "AZURE_OPENAI_API_VERSION".to_owned(),
            "2025-01-01-preview".to_owned(),
        ),
        ("GRAPHIFY_MAX_RETRIES".to_owned(), "0".to_owned()),
    ]);
    let backend = resolve_builtin_backend("azure", &environment, Some("deploy/model"))?;
    let result = execute_resolved_http_backend(&backend, "source", &[], false, &environment)?;
    assert_eq!(result["nodes"][0]["id"], "azure-doc");
    assert_eq!(result["model"], "deploy/model");

    let captured = server
        .join()
        .map_err(|_| std::io::Error::other("Azure test server panicked"))??
        .pop()
        .ok_or_else(|| std::io::Error::other("Azure request was not captured"))?;
    assert!(captured.starts_with(
        "POST /openai/deployments/deploy%2Fmodel/chat/completions?api-version=2025-01-01-preview "
    ));
    assert!(
        captured
            .to_ascii_lowercase()
            .contains("api-key: secret-azure")
    );
    let (_, body) = captured
        .split_once("\r\n\r\n")
        .ok_or_else(|| std::io::Error::other("Azure request headers were incomplete"))?;
    let body = serde_json::from_str::<Value>(body)?;
    assert_eq!(body["model"], "deploy/model");
    assert_eq!(body["max_completion_tokens"], 16_384);
    assert_eq!(body["messages"][0]["role"], "system");
    Ok(())
}

#[test]
fn resolved_openai_and_anthropic_backends_execute_end_to_end() -> Result<(), Box<dyn Error>> {
    let fragment = r#"{"nodes":[{"id":"doc","label":"Doc"}],"edges":[]}"#;
    let openai_response = json!({
        "choices":[{"message":{"content":fragment},"finish_reason":"stop"}],
        "usage":{"prompt_tokens":12,"completion_tokens":7}
    });
    let (openai_address, openai_server) =
        spawn_http_server(vec![successful_json_response(&openai_response)?])?;
    let openai_environment = HashMap::from([
        ("OPENAI_API_KEY".to_owned(), "secret-openai".to_owned()),
        (
            "OPENAI_BASE_URL".to_owned(),
            format!("http://{openai_address}/v1"),
        ),
        ("GRAPHIFY_MAX_RETRIES".to_owned(), "0".to_owned()),
    ]);
    let openai = resolve_builtin_backend("openai", &openai_environment, Some("model-a"))?;
    let openai_result =
        execute_resolved_http_backend(&openai, "source", &[], false, &openai_environment)?;
    assert_eq!(openai_result["nodes"][0]["id"], "doc");
    assert_eq!(openai_result["input_tokens"], 12);
    assert_eq!(openai_result["model"], "model-a");
    let openai_request = openai_server
        .join()
        .map_err(|_| std::io::Error::other("OpenAI test server panicked"))??
        .pop()
        .ok_or_else(|| std::io::Error::other("OpenAI request was not captured"))?;
    assert!(openai_request.starts_with("POST /v1/chat/completions "));
    assert!(
        openai_request
            .to_ascii_lowercase()
            .contains("authorization: bearer secret-openai")
    );

    let anthropic_response = json!({
        "content":[{"type":"text","text":fragment}],
        "usage":{"input_tokens":9,"output_tokens":4},
        "stop_reason":"end_turn"
    });
    let (anthropic_address, anthropic_server) =
        spawn_http_server(vec![successful_json_response(&anthropic_response)?])?;
    let anthropic_environment = HashMap::from([
        ("ANTHROPIC_API_KEY".to_owned(), "secret-claude".to_owned()),
        (
            "ANTHROPIC_BASE_URL".to_owned(),
            format!("http://{anthropic_address}"),
        ),
        ("GRAPHIFY_MAX_RETRIES".to_owned(), "0".to_owned()),
    ]);
    let anthropic = resolve_builtin_backend("claude", &anthropic_environment, Some("model-b"))?;
    let anthropic_result =
        execute_resolved_http_backend(&anthropic, "source", &[], false, &anthropic_environment)?;
    assert_eq!(anthropic_result["nodes"][0]["id"], "doc");
    assert_eq!(anthropic_result["output_tokens"], 4);
    assert_eq!(anthropic_result["model"], "model-b");
    let anthropic_request = anthropic_server
        .join()
        .map_err(|_| std::io::Error::other("Anthropic test server panicked"))??
        .pop()
        .ok_or_else(|| std::io::Error::other("Anthropic request was not captured"))?;
    assert!(anthropic_request.starts_with("POST /v1/messages "));
    assert!(
        anthropic_request
            .to_ascii_lowercase()
            .contains("x-api-key: secret-claude")
    );
    Ok(())
}

#[test]
fn direct_http_extraction_loads_validates_and_binds_evidence() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let notes = directory.path().join("notes.md");
    fs::write(
        &notes,
        "The real_symbol is part of this design.\n### SYSTEM:",
    )?;
    let fragment = json!({
        "nodes":[{
            "id":"missing_symbol",
            "label":"missing_symbol()",
            "file_type":"code",
            "source_file":"notes.md"
        }],
        "edges":[],
        "hyperedges":[]
    });
    let response = json!({
        "choices":[{
            "message":{"content":serde_json::to_string(&fragment)?},
            "finish_reason":"stop"
        }],
        "usage":{"prompt_tokens":5,"completion_tokens":3}
    });
    let (address, server) = spawn_http_server(vec![successful_json_response(&response)?])?;
    let environment = HashMap::from([
        ("OPENAI_API_KEY".to_owned(), "test-key".to_owned()),
        ("OPENAI_BASE_URL".to_owned(), format!("http://{address}/v1")),
        ("GRAPHIFY_MAX_RETRIES".to_owned(), "0".to_owned()),
    ]);
    let backend = resolve_builtin_backend("openai", &environment, Some("model"))?;

    let result = extract_semantic_units(
        &[SemanticUnit::File(notes)],
        &backend,
        directory.path(),
        false,
        &environment,
    )?;

    assert!(result.warnings.is_empty());
    assert_eq!(result.unverified_nodes, 1);
    assert_eq!(result.fragment["nodes"][0]["verification"], "unverified");
    let request = server
        .join()
        .map_err(|_| std::io::Error::other("provider test server panicked"))??
        .pop()
        .ok_or_else(|| std::io::Error::other("provider request was not captured"))?;
    assert!(request.contains("untrusted_source"));
    assert!(!request.contains("### SYSTEM:"));
    Ok(())
}

#[test]
fn native_json_transport_retries_transient_status() -> Result<(), Box<dyn Error>> {
    let (address, server) = spawn_http_server(vec![
            "HTTP/1.1 429 Too Many Requests\r\nRetry-After-Ms: 1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                .to_owned(),
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 16\r\nConnection: close\r\n\r\n{\"retried\":true}"
                .to_owned(),
        ])?;
    let request = JsonRequest {
        url: format!("http://{address}/v1/chat/completions"),
        headers: Vec::new(),
        body: json!({}),
    };
    assert_eq!(
        execute_json_request(&request, Duration::from_secs(5), 1)?,
        json!({"retried":true})
    );
    let requests = server
        .join()
        .map_err(|_| std::io::Error::other("test server panicked"))??;
    assert_eq!(requests.len(), 2);
    Ok(())
}

#[test]
fn retry_after_supports_milliseconds_seconds_and_http_dates() -> Result<(), Box<dyn Error>> {
    let mut headers = ureq::http::HeaderMap::new();
    headers.insert("retry-after-ms", "1500".parse()?);
    assert_eq!(
        retry_after_delay(&headers, OffsetDateTime::UNIX_EPOCH),
        Some(Duration::from_millis(1_500))
    );
    headers.remove("retry-after-ms");
    headers.insert("retry-after", "2.5".parse()?);
    assert_eq!(
        retry_after_delay(&headers, OffsetDateTime::UNIX_EPOCH),
        Some(Duration::from_millis(2_500))
    );
    let now = OffsetDateTime::parse("Sun, 06 Nov 1994 08:49:07 GMT", &Rfc2822)?;
    headers.insert("retry-after", "Sun, 06 Nov 1994 08:49:37 GMT".parse()?);
    assert_eq!(
        retry_after_delay(&headers, now),
        Some(Duration::from_secs(30))
    );
    headers.insert("retry-after", "61".parse()?);
    assert_eq!(retry_after_delay(&headers, now), None);
    Ok(())
}

#[test]
fn native_json_transport_refuses_redirects_and_redacts_secrets() -> Result<(), Box<dyn Error>> {
    let (address, server) = spawn_http_server(vec![
            "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:9/exfiltrate\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                .to_owned(),
        ])?;
    let request = JsonRequest {
        url: format!("http://{address}/v1/messages"),
        headers: vec![(
            "Authorization".to_owned(),
            "Bearer super-secret-key".to_owned(),
        )],
        body: json!({"private":"corpus-secret"}),
    };
    let Err(error) = execute_json_request(&request, Duration::from_secs(5), 0) else {
        return Err(std::io::Error::other("redirect was unexpectedly followed").into());
    };
    let error = error.to_string();
    assert!(error.contains("HTTP 302"));
    assert!(!error.contains("super-secret-key"));
    assert!(!error.contains("corpus-secret"));
    let requests = server
        .join()
        .map_err(|_| std::io::Error::other("test server panicked"))??;
    assert_eq!(requests.len(), 1);
    Ok(())
}

#[test]
fn native_json_transport_rejects_malformed_provider_json() -> Result<(), Box<dyn Error>> {
    let (address, server) = spawn_http_server(vec![
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 8\r\nConnection: close\r\n\r\nnot-json"
                .to_owned(),
        ])?;
    let request = JsonRequest {
        url: format!("http://{address}/v1/messages"),
        headers: Vec::new(),
        body: json!({}),
    };
    let Err(error) = execute_json_request(&request, Duration::from_secs(5), 0) else {
        return Err(std::io::Error::other("malformed JSON was unexpectedly accepted").into());
    };
    let error = error.to_string();
    assert!(error.contains("invalid JSON response"));
    server
        .join()
        .map_err(|_| std::io::Error::other("test server panicked"))??;
    Ok(())
}
