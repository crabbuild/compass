//! Lightweight provider calls used by labeling and deduplication.

use std::collections::HashMap;
use std::path::Path;

use serde_json::Value;

use super::*;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PlainTextResponse {
    pub text: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub model: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlainTextOptions {
    pub max_tokens: usize,
    /// Python only passes `--model` to Claude Code when `_call_llm` received an
    /// explicit model argument; the resolved default alone does not add it.
    pub claude_cli_model_argument: Option<String>,
}

impl Default for PlainTextOptions {
    fn default() -> Self {
        Self {
            max_tokens: 200,
            claude_cli_model_argument: None,
        }
    }
}

#[allow(clippy::too_many_arguments)]
#[must_use]
pub fn openai_plain_call_parameters(
    base_url: &str,
    model: &str,
    prompt: &str,
    max_tokens: usize,
    temperature: Option<f64>,
    reasoning_effort: Option<&str>,
    extra_body: Option<&Value>,
    disable_thinking: bool,
) -> Value {
    let mut parameters = serde_json::json!({
        "model":model,
        "messages":[{"role":"user","content":prompt}],
        "max_completion_tokens":max_tokens,
        "stream":false,
    });
    let Some(object) = parameters.as_object_mut() else {
        return parameters;
    };
    if let Some(temperature) = temperature.and_then(serde_json::Number::from_f64) {
        object.insert("temperature".to_owned(), Value::Number(temperature));
    }
    if let Some(reasoning_effort) = reasoning_effort {
        object.insert(
            "reasoning_effort".to_owned(),
            Value::String(reasoning_effort.to_owned()),
        );
    }
    let body = if let Some(extra_body) = extra_body {
        Some(extra_body.clone())
    } else if base_url.contains("moonshot") || disable_thinking {
        Some(serde_json::json!({"thinking":{"type":"disabled"}}))
    } else {
        None
    };
    if let Some(body) = body {
        object.insert("extra_body".to_owned(), body);
    }
    parameters
}

fn normalize_openai_plain(
    response: &Value,
    model: &str,
) -> Result<PlainTextResponse, SemanticError> {
    let choice = response
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .ok_or_else(|| {
            SemanticError::InvalidProviderResponse("missing response choice".to_owned())
        })?;
    let message = choice
        .get("message")
        .filter(|message| message.is_object())
        .ok_or_else(|| {
            SemanticError::InvalidProviderResponse("missing response message".to_owned())
        })?;
    let usage = response.get("usage");
    Ok(PlainTextResponse {
        text: message
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        input_tokens: numeric_u64(usage.and_then(|value| value.get("prompt_tokens"))),
        output_tokens: numeric_u64(usage.and_then(|value| value.get("completion_tokens"))),
        model: model.to_owned(),
    })
}

fn normalize_anthropic_plain(
    response: &Value,
    model: &str,
) -> Result<PlainTextResponse, SemanticError> {
    let content = response
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            SemanticError::InvalidProviderResponse("missing Anthropic content".to_owned())
        })?;
    let usage = response.get("usage");
    Ok(PlainTextResponse {
        text: content
            .first()
            .and_then(|block| block.get("text"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        input_tokens: numeric_u64(usage.and_then(|value| value.get("input_tokens"))),
        output_tokens: numeric_u64(usage.and_then(|value| value.get("output_tokens"))),
        model: model.to_owned(),
    })
}

fn anthropic_plain_request(
    backend: &ResolvedBackend,
    prompt: &str,
    max_tokens: usize,
) -> Result<JsonRequest, SemanticError> {
    let base_url = backend.base_url.as_deref().ok_or_else(|| {
        SemanticError::InvalidProviderConfiguration("Claude has no resolved base URL".to_owned())
    })?;
    let api_key = backend.api_key().ok_or_else(|| {
        SemanticError::InvalidProviderConfiguration("Claude has no API key".to_owned())
    })?;
    Ok(JsonRequest {
        url: format!("{}/v1/messages", base_url.trim_end_matches('/')),
        headers: vec![
            ("x-api-key".to_owned(), api_key.to_owned()),
            ("anthropic-version".to_owned(), "2023-06-01".to_owned()),
        ],
        body: serde_json::json!({
            "model":backend.model,
            "max_tokens":max_tokens,
            "messages":[{"role":"user","content":prompt}],
        }),
    })
}

fn execute_claude_cli_plain(
    backend: &ResolvedBackend,
    prompt: &str,
    options: &PlainTextOptions,
) -> Result<PlainTextResponse, SemanticError> {
    let program = if cfg!(windows) {
        Path::new("claude.cmd")
    } else {
        Path::new("claude")
    };
    let mut arguments = vec![
        "-p".to_owned(),
        "--output-format".to_owned(),
        "json".to_owned(),
        "--no-session-persistence".to_owned(),
    ];
    if let Some(model) = options
        .claude_cli_model_argument
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty())
    {
        arguments.push("--model".to_owned());
        arguments.push(model.to_owned());
    }
    let stdout = execute_bounded_process(program, &arguments, prompt, backend.timeout)?;
    let envelope = claude_cli_envelope(&stdout)?;
    let usage = envelope.get("usage");
    let input_tokens = numeric_u64(usage.and_then(|value| value.get("input_tokens")))
        .saturating_add(numeric_u64(
            usage.and_then(|value| value.get("cache_read_input_tokens")),
        ))
        .saturating_add(numeric_u64(
            usage.and_then(|value| value.get("cache_creation_input_tokens")),
        ));
    Ok(PlainTextResponse {
        text: envelope
            .get("result")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        input_tokens,
        output_tokens: numeric_u64(usage.and_then(|value| value.get("output_tokens"))),
        model: backend.model.clone(),
    })
}

/// Execute a lightweight built-in provider call without the extraction system
/// prompt or graph JSON parser.
pub fn execute_plain_text_backend(
    backend: &ResolvedBackend,
    prompt: &str,
    options: &PlainTextOptions,
    environment: &HashMap<String, String>,
) -> Result<PlainTextResponse, SemanticError> {
    if options.max_tokens == 0 {
        return Err(SemanticError::InvalidProviderConfiguration(
            "plain-text max_tokens must be positive".to_owned(),
        ));
    }
    match backend.backend.name {
        "bedrock" => execute_bedrock_plain_text(backend, prompt, options.max_tokens, environment),
        "claude-cli" => execute_claude_cli_plain(backend, prompt, options),
        "claude" => {
            let request = anthropic_plain_request(backend, prompt, options.max_tokens)?;
            let response = execute_json_request(&request, backend.timeout, backend.max_retries)?;
            normalize_anthropic_plain(&response, &backend.model)
        }
        name => {
            let base_url = backend.base_url.as_deref().ok_or_else(|| {
                SemanticError::InvalidProviderConfiguration(format!(
                    "backend {name:?} has no resolved base URL"
                ))
            })?;
            let api_key = match backend.api_key() {
                Some(key) => key,
                None if name == "ollama" => "ollama",
                None => {
                    return Err(SemanticError::InvalidProviderConfiguration(format!(
                        "backend {name:?} has no API key"
                    )));
                }
            };
            let disable_thinking = environment
                .get("GRAPHIFY_DISABLE_THINKING")
                .is_some_and(|value| env_truthy(value));
            let parameters = openai_plain_call_parameters(
                base_url,
                &backend.model,
                prompt,
                options.max_tokens,
                backend.temperature,
                backend.backend.reasoning_effort,
                None,
                disable_thinking,
            );
            let request = if name == "azure" {
                let api_version = environment
                    .get("AZURE_OPENAI_API_VERSION")
                    .map_or("2024-12-01-preview", String::as_str)
                    .trim();
                azure_openai_http_request(
                    base_url,
                    api_key,
                    &backend.model,
                    api_version,
                    parameters,
                )?
            } else {
                openai_http_request(base_url, api_key, parameters)?
            };
            let response = execute_json_request(&request, backend.timeout, backend.max_retries)?;
            normalize_openai_plain(&response, &backend.model)
        }
    }
}

/// Execute a lightweight custom OpenAI-compatible provider call.
pub fn execute_plain_text_custom_backend(
    backend: &ResolvedCustomBackend,
    prompt: &str,
    options: &PlainTextOptions,
    environment: &HashMap<String, String>,
) -> Result<PlainTextResponse, SemanticError> {
    if options.max_tokens == 0 {
        return Err(SemanticError::InvalidProviderConfiguration(
            "plain-text max_tokens must be positive".to_owned(),
        ));
    }
    let disable_thinking = environment
        .get("GRAPHIFY_DISABLE_THINKING")
        .is_some_and(|value| env_truthy(value));
    let parameters = openai_plain_call_parameters(
        &backend.base_url,
        &backend.model,
        prompt,
        options.max_tokens,
        backend.temperature,
        backend.reasoning_effort.as_deref(),
        backend.extra_body.as_ref(),
        disable_thinking,
    );
    let request = openai_http_request(&backend.base_url, backend.api_key(), parameters)?;
    let response = execute_json_request(&request, backend.timeout, backend.max_retries)?;
    normalize_openai_plain(&response, &backend.model)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn backend(name: &str, base_url: Option<&str>, api_key: Option<&str>) -> ResolvedBackend {
        let spec = BUILTIN_BACKENDS
            .iter()
            .find(|candidate| candidate.name == name)
            .unwrap_or(&BUILTIN_BACKENDS[0]);
        ResolvedBackend {
            backend: spec,
            base_url: base_url.map(str::to_owned),
            model: "fixture-model".to_owned(),
            api_key: api_key.map(str::to_owned),
            temperature: None,
            max_output_tokens: 200,
            timeout: Duration::from_millis(10),
            max_retries: 0,
        }
    }

    #[test]
    fn response_normalizers_reject_shapes_and_default_optional_fields() {
        assert!(normalize_openai_plain(&serde_json::json!({}), "model").is_err());
        assert!(normalize_openai_plain(&serde_json::json!({"choices":[7]}), "model").is_err());
        let openai = normalize_openai_plain(
            &serde_json::json!({
                "choices":[{"message":{"content":7}}],
                "usage":{"prompt_tokens":3,"completion_tokens":"bad"}
            }),
            "model",
        )
        .unwrap_or_default();
        assert_eq!(openai.text, "");
        assert_eq!(openai.input_tokens, 3);
        assert_eq!(openai.output_tokens, 0);

        assert!(normalize_anthropic_plain(&serde_json::json!({}), "model").is_err());
        let anthropic = normalize_anthropic_plain(
            &serde_json::json!({"content":[],"usage":{"input_tokens":2,"output_tokens":1}}),
            "model",
        )
        .unwrap_or_default();
        assert_eq!(anthropic.text, "");
        assert_eq!((anthropic.input_tokens, anthropic.output_tokens), (2, 1));
    }

    #[test]
    fn request_and_execution_validation_fail_before_transport()
    -> Result<(), Box<dyn std::error::Error>> {
        assert!(anthropic_plain_request(&backend("claude", None, Some("key")), "p", 1).is_err());
        assert!(
            anthropic_plain_request(
                &backend("claude", Some("https://api.example.test"), None),
                "p",
                1
            )
            .is_err()
        );
        let request = anthropic_plain_request(
            &backend("claude", Some("https://api.example.test/"), Some("key")),
            "prompt",
            9,
        )?;
        assert_eq!(request.url, "https://api.example.test/v1/messages");
        assert_eq!(request.body["max_tokens"], 9);

        let options = PlainTextOptions {
            max_tokens: 0,
            claude_cli_model_argument: None,
        };
        assert!(
            execute_plain_text_backend(
                &backend("claude", None, None),
                "prompt",
                &options,
                &HashMap::new()
            )
            .is_err()
        );
        assert!(
            execute_plain_text_backend(
                &backend("openai", None, Some("key")),
                "prompt",
                &PlainTextOptions::default(),
                &HashMap::new()
            )
            .is_err()
        );
        assert!(
            execute_plain_text_backend(
                &backend("openai", Some("https://api.example.test"), None),
                "prompt",
                &PlainTextOptions::default(),
                &HashMap::new()
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn openai_parameters_ignore_non_finite_temperature_and_prefer_explicit_body() {
        let parameters = openai_plain_call_parameters(
            "https://moonshot.example",
            "model",
            "prompt",
            4,
            Some(f64::NAN),
            None,
            Some(&serde_json::json!({"custom":true})),
            true,
        );
        assert!(parameters.get("temperature").is_none());
        assert_eq!(parameters["extra_body"]["custom"], true);
    }
}
