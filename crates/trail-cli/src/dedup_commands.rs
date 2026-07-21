use std::collections::HashMap;
use std::path::Path;

use trail_graph::{AmbiguousPair, EntityTiebreaker};
use trail_semantic::{
    PlainTextOptions, ResolvedBackend, ResolvedCustomBackend, detect_backend_with_custom,
    execute_plain_text_backend, execute_plain_text_custom_backend, load_custom_providers,
    resolve_builtin_backend, resolve_custom_backend,
};

enum Backend {
    Builtin(ResolvedBackend),
    Custom(ResolvedCustomBackend),
}

pub(super) struct DedupLlmTiebreaker {
    backend: Backend,
    environment: HashMap<String, String>,
    explicit_model: Option<String>,
    warnings: Vec<String>,
}

impl DedupLlmTiebreaker {
    pub(super) fn prepare(
        requested_backend: Option<&str>,
        requested_model: Option<&str>,
        environment: HashMap<String, String>,
        global_providers: &Path,
        local_providers: &Path,
        allow_local_providers: bool,
        claude_cli_available: bool,
    ) -> Result<Self, String> {
        let custom =
            load_custom_providers(global_providers, local_providers, allow_local_providers);
        let selected = requested_backend
            .map(str::to_owned)
            .or_else(|| {
                detect_backend_with_custom(&custom.providers, &environment).map(str::to_owned)
            })
            .ok_or_else(|| {
                "no LLM API key found (--dedup-llm was passed). Set GEMINI_API_KEY or GOOGLE_API_KEY (gemini), MOONSHOT_API_KEY (kimi), ANTHROPIC_API_KEY (claude), OPENAI_API_KEY (openai), DEEPSEEK_API_KEY (deepseek), or pass --backend. A code-only corpus needs no key."
                    .to_owned()
            })?;

        let backend = if let Some(spec) = trail_semantic::builtin_backend(&selected) {
            let resolved = resolve_builtin_backend(&selected, &environment, requested_model)
                .map_err(|error| error.to_string())?;
            if selected == "claude-cli" && !claude_cli_available {
                return Err(
                    "backend 'claude-cli' requires the `claude` CLI on $PATH (install Claude Code and run `claude` once to authenticate)."
                        .to_owned(),
                );
            }
            let allows_keyless = (selected == "ollama"
                && resolved
                    .base_url
                    .as_deref()
                    .is_some_and(super::provider_url_is_loopback))
                || (selected == "bedrock"
                    && [
                        "AWS_PROFILE",
                        "AWS_REGION",
                        "AWS_DEFAULT_REGION",
                        "AWS_ACCESS_KEY_ID",
                    ]
                    .into_iter()
                    .any(|key| environment.get(key).is_some_and(|value| !value.is_empty())))
                || selected == "claude-cli";
            if !spec.api_key_variables.is_empty() && resolved.api_key().is_none() && !allows_keyless
            {
                return Err(format!(
                    "backend '{selected}' requires {} to be set.",
                    format_environment_keys(spec.api_key_variables)
                ));
            }
            Backend::Builtin(resolved)
        } else if let Some(config) = custom.providers.get(&selected) {
            Backend::Custom(
                resolve_custom_backend(&selected, config, &environment, requested_model, None)
                    .map_err(|error| error.to_string())?,
            )
        } else {
            let mut available = trail_semantic::BUILTIN_BACKENDS
                .iter()
                .map(|backend| backend.name.to_owned())
                .chain(custom.providers.keys().cloned())
                .collect::<Vec<_>>();
            available.sort();
            return Err(format!(
                "unknown backend '{selected}'. Available: {}",
                available.join(", ")
            ));
        };

        Ok(Self {
            backend,
            environment,
            explicit_model: requested_model.map(str::to_owned),
            warnings: custom.warnings,
        })
    }

    pub(super) fn take_warnings(&mut self) -> Vec<String> {
        std::mem::take(&mut self.warnings)
    }

    fn call(&self, prompt: &str) -> Result<String, String> {
        let options = PlainTextOptions {
            max_tokens: 200,
            claude_cli_model_argument: match &self.backend {
                Backend::Builtin(backend) if backend.backend.name == "claude-cli" => {
                    self.explicit_model.clone()
                }
                _ => None,
            },
        };
        match &self.backend {
            Backend::Builtin(backend) => {
                execute_plain_text_backend(backend, prompt, &options, &self.environment)
            }
            Backend::Custom(backend) => {
                execute_plain_text_custom_backend(backend, prompt, &options, &self.environment)
            }
        }
        .map(|response| response.text)
        .map_err(|error| error.to_string())
    }
}

impl EntityTiebreaker for DedupLlmTiebreaker {
    fn decide(&mut self, pairs: &[AmbiguousPair]) -> Vec<bool> {
        let mut decisions = vec![false; pairs.len()];
        for (batch_index, batch) in pairs.chunks(30).enumerate() {
            let pairs_text = batch
                .iter()
                .enumerate()
                .map(|(index, pair)| {
                    format!(
                        "{}. \"{}\" vs \"{}\"",
                        index + 1,
                        pair.left.label(),
                        pair.right.label()
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            let prompt = format!(
                "For each pair below, answer only 'yes' or 'no': are they the same real-world concept?\n\n{pairs_text}\n\nReply with one line per pair: '1. yes', '2. no', etc."
            );
            let response = match self.call(&prompt) {
                Ok(response) => response,
                Err(error) => {
                    self.warnings
                        .push(format!("[graphify] --dedup-llm batch failed: {error}"));
                    continue;
                }
            };
            let offset = batch_index * 30;
            for line in response
                .trim()
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
            {
                let Some((number, answer)) = line.split_once('.') else {
                    continue;
                };
                let Ok(index) = number.trim().parse::<usize>() else {
                    continue;
                };
                if index == 0 || index > batch.len() {
                    continue;
                }
                if answer.trim().to_ascii_lowercase().starts_with("yes") {
                    decisions[offset + index - 1] = true;
                }
            }
        }
        decisions
    }
}

fn format_environment_keys(keys: &[&str]) -> String {
    match keys {
        [] => String::new(),
        [key] => (*key).to_owned(),
        [left, right] => format!("{left} or {right}"),
        _ => keys.join(" or "),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Map, Value};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use trail_model::NodeRecord;

    fn pair(left: &str, right: &str) -> AmbiguousPair {
        let mut left_attributes = Map::new();
        left_attributes.insert("label".to_owned(), Value::String(left.to_owned()));
        let mut right_attributes = Map::new();
        right_attributes.insert("label".to_owned(), Value::String(right.to_owned()));
        AmbiguousPair {
            left: NodeRecord {
                id: "left".to_owned(),
                attributes: left_attributes,
            },
            right: NodeRecord {
                id: "right".to_owned(),
                attributes: right_attributes,
            },
            score: 80.0,
        }
    }

    #[test]
    fn numbered_answers_follow_python_parser() {
        let lines = "1. yes\n2. no\n3. YES, definitely\ninvalid\n0. yes";
        let mut decisions = vec![false; 3];
        for line in lines.lines() {
            let Some((number, answer)) = line.split_once('.') else {
                continue;
            };
            let Ok(index) = number.trim().parse::<usize>() else {
                continue;
            };
            if index > 0
                && index <= decisions.len()
                && answer.trim().to_ascii_lowercase().starts_with("yes")
            {
                decisions[index - 1] = true;
            }
        }
        assert_eq!(decisions, vec![true, false, true]);
        assert_eq!(pair("Account", "Identity").left.label(), "Account");
    }

    #[test]
    fn provider_tiebreaker_sends_python_prompt_and_applies_yes()
    -> Result<(), Box<dyn std::error::Error>> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        let server = std::thread::spawn(move || -> Result<String, String> {
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
            let body: Value = serde_json::from_slice(
                &request[header_end..header_end.saturating_add(content_length)],
            )
            .map_err(|error| error.to_string())?;
            let prompt = body["messages"][0]["content"]
                .as_str()
                .ok_or_else(|| "missing prompt".to_owned())?
                .to_owned();
            let response = serde_json::json!({
                "choices": [{"message": {"content": "1. yes"}}],
                "usage": {"prompt_tokens": 10, "completion_tokens": 2}
            });
            let response = serde_json::to_vec(&response).map_err(|error| error.to_string())?;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                response.len()
            )
            .map_err(|error| error.to_string())?;
            stream
                .write_all(&response)
                .map_err(|error| error.to_string())?;
            Ok(prompt)
        });

        let temp = tempfile::tempdir()?;
        let mut environment = HashMap::new();
        environment.insert("OPENAI_API_KEY".to_owned(), "test-key".to_owned());
        environment.insert("OPENAI_BASE_URL".to_owned(), format!("http://{address}/v1"));
        let mut tiebreaker = DedupLlmTiebreaker::prepare(
            Some("openai"),
            Some("trail-test"),
            environment,
            &temp.path().join("global.json"),
            &temp.path().join("local.json"),
            false,
            false,
        )?;
        let decisions = tiebreaker.decide(&[pair(
            "Customer Account Management",
            "Customer Identity Management",
        )]);
        assert_eq!(decisions, vec![true]);
        let prompt = server.join().map_err(|_| "provider thread panicked")??;
        assert_eq!(
            prompt,
            "For each pair below, answer only 'yes' or 'no': are they the same real-world concept?\n\n1. \"Customer Account Management\" vs \"Customer Identity Management\"\n\nReply with one line per pair: '1. yes', '2. no', etc."
        );
        Ok(())
    }
}
