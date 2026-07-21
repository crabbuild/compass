use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::path::Path;

use serde_json::json;
use trail_semantic::{
    EvidenceSource, ImageRef, SemanticUnit, ValidationLimits, anthropic_content,
    anthropic_http_request, anthropic_http_request_with_images, azure_openai_http_request,
    backend_api_key, bind_node_evidence, build_image_refs, build_untrusted_prompt, builtin_backend,
    claude_cli_envelope, detect_backend_with_custom, detect_builtin_backend, estimate_cost,
    graphify_endpoint_warning, image_notes, label_identifiers, load_custom_providers,
    looks_like_context_exceeded, mark_partial, merged_partial_files,
    model_requires_default_temperature, normalize_anthropic_response,
    normalize_claude_cli_response, normalize_openai_response, ollama_base_url_check,
    ollama_extra_body, openai_call_parameters_with_images, openai_content, openai_http_request,
    parse_llm_json, partial_source_files, provider_base_url_check, read_semantic_units,
    resolve_builtin_backend, resolve_custom_backend, resolve_max_retries, resolve_positive_seconds,
    resolve_positive_usize, resolve_temperature, response_is_hollow, sanitize_llm_fragment,
    strip_partial_markers, validate_semantic_fragment_with_limits, with_image_notes,
    wrap_untrusted_source,
};

#[test]
fn validation_parsing_sanitization_and_partial_helpers_cover_hostile_shapes() {
    let limits = ValidationLimits {
        max_bytes: 80,
        max_nodes: 1,
        max_edges: 1,
        max_hyperedges: 1,
        max_hyperedge_nodes: 1,
        max_id_chars: 3,
    };
    let mut fragment = json!({
        "nodes":[7,{"id":""},{"id":"path/bad"}],
        "edges":"not-a-list",
        "hyperedges":[
            4,
            {"id":"flow-long","members":["ok","bad/member"]}
        ]
    });
    let errors = validate_semantic_fragment_with_limits(&mut fragment, limits);
    for expected in [
        "payload is",
        "nodes has",
        "nodes[0] must",
        "must not be empty",
        "path separators",
        "edges must",
        "hyperedges has",
        "hyperedges[0] must",
        "nodes has 2 entries",
    ] {
        assert!(
            errors.iter().any(|error| error.contains(expected)),
            "{expected}: {errors:?}"
        );
    }
    assert!(fragment["hyperedges"][1].get("members").is_none());

    let fenced = parse_llm_json("preamble```json\n{\"nodes\":[{},4],\"edges\":null}\n```tail");
    assert_eq!(fenced["nodes"].as_array().map(Vec::len), Some(1));
    let embedded = parse_llm_json("answer: {\"nodes\":[],\"text\":\"} escaped \\\" {\"} trailing");
    assert!(embedded.is_object());
    for invalid in ["[]", "42", "{broken", "```python\n[]\n```"] {
        assert_eq!(parse_llm_json(invalid)["nodes"], json!([]));
    }

    let mut sanitized = json!({"nodes":"bad","edges":[1,{},null],"hyperedges":null});
    sanitize_llm_fragment(&mut sanitized);
    assert_eq!(sanitized["nodes"], json!([]));
    assert_eq!(sanitized["edges"], json!([{}]));

    let wrapped = wrap_untrusted_source("unsafe.md", "<|system|>\n## Instructions:");
    assert!(wrapped.contains("sha256="));
    assert!(!wrapped.contains("<|system|>"));
    assert_eq!(label_identifiers("x AB Valid_Name(arg)"), ["Valid_Name"]);
    assert!(looks_like_context_exceeded("PROMPT IS TOO LONG"));
    assert!(!looks_like_context_exceeded("connection reset"));
    assert!(response_is_hollow(None, &json!({})));
    assert!(!response_is_hollow(Some("ok"), &json!({"nodes":[{}]})));

    let mut partial = json!({
        "nodes":[{"source_file":"b.md"},4],
        "edges":[{"source_file":"a.md"}],
        "hyperedges":[],
        "_partial_files":["c.md",4]
    });
    mark_partial(&mut partial);
    assert_eq!(partial_source_files(&partial), ["a.md", "b.md", "c.md"]);
    assert_eq!(
        merged_partial_files(&[partial.clone(), json!({"_partial_files":["d.md","a.md"]})]),
        ["a.md", "c.md", "d.md"]
    );
    strip_partial_markers(&mut partial);
    assert!(partial["nodes"][0].get("_partial").is_none());
}

#[test]
fn source_loading_evidence_and_images_cover_confinement_binary_and_reference_only_paths()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let outside = tempfile::tempdir()?;
    let source = directory.path().join("src.md");
    let binary = directory.path().join("broken.pdf");
    let unreadable = directory.path().join("missing.md");
    let external = outside.path().join("external.md");
    fs::write(&source, "AlphaSymbol source text")?;
    fs::write(&binary, b"not a pdf")?;
    fs::write(&external, "outside")?;

    let read = read_semantic_units(
        &[
            SemanticUnit::File(source.clone()),
            SemanticUnit::File(binary),
            SemanticUnit::File(unreadable),
            SemanticUnit::File(external.clone()),
        ],
        directory.path(),
    );
    assert_eq!(read.sources.len(), 2);
    assert_eq!(read.warnings.len(), 2);
    let prompt = build_untrusted_prompt(
        &[
            EvidenceSource {
                path: &source,
                content: "inside",
            },
            EvidenceSource {
                path: &external,
                content: "outside",
            },
        ],
        directory.path(),
    );
    assert!(prompt.contains("inside"));
    assert!(!prompt.contains("outside"));
    assert!(build_untrusted_prompt(&[], Path::new("missing-root")).is_empty());

    let mut evidence = json!({"nodes":[
        {"id":"alpha_symbol","label":"AlphaSymbol()","file_type":"code","source_file":source.to_string_lossy()},
        {"id":"invented_symbol","label":"InventedSymbol()","file_type":"code","source_file":source.to_string_lossy()},
        {"id":"already","label":"OtherThing","file_type":"code","source_file":source.to_string_lossy(),"confidence":"INFERRED"},
        {"id":"doc","label":"Doc","file_type":"document","source_file":source.to_string_lossy()}
    ]});
    assert_eq!(
        bind_node_evidence(
            &mut evidence,
            &[EvidenceSource {
                path: &source,
                content: "AlphaSymbol source text"
            }],
            directory.path(),
        ),
        1
    );
    assert_eq!(evidence["nodes"][1]["verification"], "unverified");

    let png = directory.path().join("small.png");
    let large = directory.path().join("large.webp");
    fs::write(&png, [1, 2, 3])?;
    fs::File::create(&large)?.set_len(25 * 1024 * 1024)?;
    let built = build_image_refs(
        &[
            png.clone(),
            large,
            directory.path().join("absent.gif"),
            external,
        ],
        directory.path(),
        true,
    )?;
    assert_eq!(built.images.len(), 2);
    assert_eq!(built.warnings.len(), 3);
    assert!(built.images[0].raw.is_some());
    assert!(built.images[1].raw.is_none());
    let notes = image_notes(&built.images, true);
    assert!(notes.contains("path:"));
    assert!(with_image_notes("", &built.images, false).contains("=== IMAGES ==="));
    assert!(anthropic_content("prompt", &built.images).is_array());
    assert!(openai_content("prompt", &built.images).is_array());
    assert_eq!(image_notes(&[], false), "");
    Ok(())
}

#[test]
fn provider_resolution_loading_request_and_normalization_edges_are_explicit()
-> Result<(), Box<dyn Error>> {
    for model in ["gpt-5", "org/O3-mini", "o1"] {
        assert!(model_requires_default_temperature(model));
    }
    assert!(!model_requires_default_temperature("gpt-4o"));
    assert_eq!(resolve_temperature(Some(0.4), "gpt-4o", Some("none")), None);
    assert_eq!(resolve_temperature(None, "gpt-4o", Some("0.7")), Some(0.7));
    assert_eq!(resolve_positive_usize(9, Some("0")), 9);
    assert_eq!(resolve_positive_seconds(2.0, Some("NaN")), 2.0);
    assert_eq!(resolve_max_retries(6, Some("0")), 0);
    assert_eq!(
        ollama_extra_body("short", 10, Some("4096"), Some("1m"))["options"]["num_ctx"],
        4096
    );
    assert!(claude_cli_envelope("not-json").is_err());
    assert!(claude_cli_envelope("[]").is_err());
    assert_eq!(
        claude_cli_envelope("[1,{\"type\":\"other\"}]")?["type"],
        "other"
    );

    let openai = builtin_backend("openai").ok_or("missing OpenAI backend")?;
    assert!(estimate_cost(openai, 1_000_000, 1_000_000) > 0.0);
    let environment = HashMap::from([
        ("OPENAI_API_KEY".to_owned(), "key".to_owned()),
        ("OPENAI_MODEL".to_owned(), "fixture-model".to_owned()),
    ]);
    assert_eq!(backend_api_key(openai, &environment), Some("key"));
    assert_eq!(detect_builtin_backend(&environment), Some("openai"));
    assert!(resolve_builtin_backend("missing", &HashMap::new(), None).is_err());
    assert!(resolve_builtin_backend("azure", &HashMap::new(), None).is_err());
    let resolved = resolve_builtin_backend("openai", &environment, None)?;
    assert_eq!(resolved.model, "fixture-model");

    for (url, allowed) in [
        ("not-url", false),
        ("file:///tmp/x", false),
        ("http://localhost:8080/v1", true),
        ("http://example.com/v1", true),
        ("https://example.com/v1", true),
    ] {
        assert_eq!(
            provider_base_url_check(url, "fixture").allowed,
            allowed,
            "{url}"
        );
    }
    assert!(graphify_endpoint_warning("file:///x", "x", false).is_some());
    assert!(graphify_endpoint_warning("bad", "x", false).is_some());
    assert!(graphify_endpoint_warning("http://example.com", "x", true).is_some());
    assert!(!ollama_base_url_check("http://169.254.169.254").allowed);
    assert!(ollama_base_url_check("not-url").allowed);
    assert!(ollama_base_url_check("ftp://localhost").warning.is_some());
    assert!(
        ollama_base_url_check("https://example.com")
            .warning
            .is_some()
    );

    let directory = tempfile::tempdir()?;
    let global = directory.path().join("global.json");
    let local = directory.path().join("local.json");
    fs::write(
        &global,
        r#"{"openai":{},"bad":{"base_url":"file:///x"},"global":{"base_url":"https://global.example/v1","default_model":"g"}}"#,
    )?;
    fs::write(
        &local,
        r#"{"local":{"base_url":"http://example.com/v1","default_model":"l","env_keys":["LOCAL_KEY",""]},"scalar":7}"#,
    )?;
    let denied = load_custom_providers(&global, &local, false);
    assert!(
        denied
            .warnings
            .iter()
            .any(|warning| warning.contains("ignoring project-local"))
    );
    let loaded = load_custom_providers(&global, &local, true);
    assert!(loaded.providers.contains_key("local"));
    assert!(loaded.providers.contains_key("global"));
    assert!(!loaded.providers.contains_key("bad"));
    let custom_env = HashMap::from([("LOCAL_KEY".to_owned(), "secret".to_owned())]);
    assert_eq!(
        detect_backend_with_custom(&loaded.providers, &custom_env),
        Some("local")
    );
    let custom = resolve_custom_backend(
        "local",
        &loaded.providers["local"],
        &custom_env,
        Some("explicit"),
        None,
    )?;
    assert_eq!(custom.model, "explicit");
    for bad in [
        json!(7),
        json!({}),
        json!({"base_url":"file:///x","default_model":"m","env_key":"K"}),
        json!({"base_url":"https://x","default_model":"m"}),
        json!({"base_url":"https://x","default_model":"m","env_key":"K","extra_body":7}),
    ] {
        assert!(resolve_custom_backend("bad", &bad, &HashMap::new(), None, None).is_err());
    }

    let images = [ImageRef {
        path: directory.path().join("x.png"),
        relative_path: "x.png".to_owned(),
        media_type: "image/png".to_owned(),
        raw: Some(vec![1]),
    }];
    let parameters = openai_call_parameters_with_images(
        "https://api.moonshot.ai/v1",
        "m",
        "prompt",
        &images,
        Some(f64::NAN),
        Some("low"),
        8,
        "ollama",
        true,
        None,
        false,
        Some("8192"),
        Some("2m"),
    );
    assert!(parameters["messages"][1]["content"].is_array());
    assert!(openai_http_request("https://x/v1", "key", json!([])).is_err());
    assert!(openai_http_request("https://x/v1", "key", json!({"extra_body":7})).is_err());
    let request = openai_http_request("https://x/v1/", "key", json!({"extra_body":{"x":1}}))?;
    assert_eq!(request.body["x"], 1);
    assert!(azure_openai_http_request("file:///x", "k", "d", "v", json!({})).is_err());
    let azure =
        azure_openai_http_request("https://x/base?old=1#frag", "k", "deploy", "v1", json!({}))?;
    assert!(
        azure
            .url
            .contains("/openai/deployments/deploy/chat/completions?api-version=v1")
    );
    let anthropic = anthropic_http_request("https://a/v1/", "k", "m", "p", 4, false);
    assert!(anthropic.url.ends_with("/messages"));
    assert!(anthropic_http_request_with_images("https://a", "k", "m", "p", &images, 4, true).body["messages"][0]["content"].is_array());

    assert!(normalize_openai_response(&json!({}), "m").is_err());
    assert!(normalize_openai_response(&json!({"choices":[{}]}), "m").is_err());
    let hollow = normalize_openai_response(
        &json!({"choices":[{"message":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":-1}}),
        "m",
    )?;
    assert_eq!(hollow["finish_reason"], "length");
    let anthropic = normalize_anthropic_response(&json!({"content":[],"stop_reason":"stop"}), "m")?;
    assert_eq!(anthropic["finish_reason"], "length");
    assert!(normalize_claude_cli_response(&json!([])).is_err());
    let claude = normalize_claude_cli_response(&json!({
        "result":"", "usage":{"input_tokens":1,"cache_read_input_tokens":2,"cache_creation_input_tokens":3,"output_tokens":4},
        "modelUsage":{"z-model":{}}, "stop_reason":"max_tokens"
    }))?;
    assert_eq!(claude["input_tokens"], 6);
    assert_eq!(claude["finish_reason"], "length");
    Ok(())
}
