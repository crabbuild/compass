use std::collections::HashMap;
use std::error::Error;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};
use trail_semantic::{
    PlainTextOptions, execute_plain_text_backend, execute_plain_text_custom_backend,
    openai_plain_call_parameters, resolve_builtin_backend, resolve_custom_backend,
};

fn mock_json_server(
    body: Value,
) -> Result<(String, thread::JoinHandle<Result<String, std::io::Error>>), Box<dyn Error>> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    let handle = thread::spawn(move || {
        let (mut socket, _) = listener.accept()?;
        socket.set_read_timeout(Some(Duration::from_secs(5)))?;
        let mut request = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            let read = socket.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
            let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n")
            else {
                continue;
            };
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or_default();
            if request.len() >= header_end + 4 + content_length {
                break;
            }
        }
        let encoded = serde_json::to_vec(&body)?;
        write!(
            socket,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            encoded.len()
        )?;
        socket.write_all(&encoded)?;
        Ok(String::from_utf8_lossy(&request).into_owned())
    });
    Ok((format!("http://{address}"), handle))
}

#[test]
fn custom_openai_plain_text_round_trips_prompt_usage_and_extra_body() -> Result<(), Box<dyn Error>>
{
    let (base_url, server) = mock_json_server(json!({
        "choices":[{"message":{"content":"yes"}}],
        "usage":{"prompt_tokens":12,"completion_tokens":3}
    }))?;
    let config = json!({
        "base_url": format!("{base_url}/v1"),
        "default_model":"fixture-model",
        "env_key":"FIXTURE_API_KEY",
        "temperature":0.25,
        "reasoning_effort":"low",
        "extra_body":{"fixture":true},
        "max_retries":0,
        "timeout":5
    });
    let environment = HashMap::from([("FIXTURE_API_KEY".to_owned(), "secret".to_owned())]);
    let backend = resolve_custom_backend("fixture", &config, &environment, None, None)?;
    let response = execute_plain_text_custom_backend(
        &backend,
        "answer briefly",
        &PlainTextOptions {
            max_tokens: 17,
            claude_cli_model_argument: None,
        },
        &environment,
    )?;
    assert_eq!(response.text, "yes");
    assert_eq!(response.input_tokens, 12);
    assert_eq!(response.output_tokens, 3);
    assert_eq!(response.model, "fixture-model");
    let request = server.join().map_err(|_| "mock server panicked")??;
    assert!(request.starts_with("POST /v1/chat/completions"));
    assert!(
        request
            .to_ascii_lowercase()
            .contains("authorization: bearer secret")
    );
    assert!(request.contains("answer briefly"));
    assert!(request.contains("\"fixture\": true"), "{request}");
    assert!(!request.contains("\"extra_body\""), "{request}");
    Ok(())
}

#[test]
fn builtin_openai_and_anthropic_plain_paths_normalize_provider_envelopes()
-> Result<(), Box<dyn Error>> {
    let (openai_url, openai_server) = mock_json_server(json!({
        "choices":[{"message":{"content":"openai text"}}],
        "usage":{"prompt_tokens":"9","completion_tokens":4}
    }))?;
    let openai_environment = HashMap::from([
        ("OPENAI_API_KEY".to_owned(), "openai-key".to_owned()),
        ("OPENAI_BASE_URL".to_owned(), format!("{openai_url}/v1")),
        ("GRAPHIFY_DISABLE_THINKING".to_owned(), "yes".to_owned()),
        ("GRAPHIFY_API_MAX_RETRIES".to_owned(), "0".to_owned()),
    ]);
    let openai = resolve_builtin_backend("openai", &openai_environment, Some("gpt-fixture"))?;
    let response = execute_plain_text_backend(
        &openai,
        "openai prompt",
        &PlainTextOptions::default(),
        &openai_environment,
    )?;
    assert_eq!(response.text, "openai text");
    assert_eq!(response.input_tokens, 0);
    assert!(
        openai_server
            .join()
            .map_err(|_| "mock server panicked")??
            .contains("gpt-fixture")
    );

    let (claude_url, claude_server) = mock_json_server(json!({
        "content":[{"type":"text","text":"claude text"}],
        "usage":{"input_tokens":7,"output_tokens":2}
    }))?;
    let claude_environment = HashMap::from([
        ("ANTHROPIC_API_KEY".to_owned(), "claude-key".to_owned()),
        ("ANTHROPIC_BASE_URL".to_owned(), claude_url),
        ("GRAPHIFY_API_MAX_RETRIES".to_owned(), "0".to_owned()),
    ]);
    let claude = resolve_builtin_backend("claude", &claude_environment, None)?;
    let response = execute_plain_text_backend(
        &claude,
        "claude prompt",
        &PlainTextOptions {
            max_tokens: 21,
            claude_cli_model_argument: None,
        },
        &claude_environment,
    )?;
    assert_eq!(response.text, "claude text");
    assert_eq!(response.input_tokens, 7);
    assert_eq!(response.output_tokens, 2);
    let request = claude_server.join().map_err(|_| "mock server panicked")??;
    assert!(request.starts_with("POST /v1/messages"));
    assert!(
        request
            .to_ascii_lowercase()
            .contains("x-api-key: claude-key")
    );
    Ok(())
}

#[test]
fn plain_text_parameter_and_validation_paths_reject_empty_limits_and_bad_envelopes()
-> Result<(), Box<dyn Error>> {
    let parameters = openai_plain_call_parameters(
        "https://api.moonshot.ai/v1",
        "model",
        "prompt",
        8,
        Some(f64::NAN),
        Some("medium"),
        None,
        false,
    );
    assert!(parameters.get("temperature").is_none());
    assert_eq!(parameters["reasoning_effort"], "medium");
    assert_eq!(parameters["extra_body"]["thinking"]["type"], "disabled");

    let config = json!({
        "base_url":"http://127.0.0.1:9/v1",
        "default_model":"fixture",
        "env_key":"KEY"
    });
    let environment = HashMap::from([("KEY".to_owned(), "secret".to_owned())]);
    let backend = resolve_custom_backend("fixture", &config, &environment, None, None)?;
    assert!(
        execute_plain_text_custom_backend(
            &backend,
            "prompt",
            &PlainTextOptions {
                max_tokens: 0,
                claude_cli_model_argument: None
            },
            &environment,
        )
        .is_err()
    );

    let (base_url, server) = mock_json_server(json!({"not_choices":[]}))?;
    let invalid_config = json!({
        "base_url":format!("{base_url}/v1"),
        "default_model":"fixture",
        "env_key":"KEY",
        "max_retries":0
    });
    let invalid = resolve_custom_backend("invalid", &invalid_config, &environment, None, None)?;
    assert!(
        execute_plain_text_custom_backend(
            &invalid,
            "prompt",
            &PlainTextOptions::default(),
            &environment,
        )
        .is_err()
    );
    server.join().map_err(|_| "mock server panicked")??;
    Ok(())
}
