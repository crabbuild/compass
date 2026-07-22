//! Amazon Bedrock Converse transport using the standard AWS credential chain.

use std::collections::HashMap;

use aws_config::{BehaviorVersion, retry::RetryConfig, timeout::TimeoutConfig};
use aws_sdk_bedrockruntime::{
    Client,
    config::{Builder as BedrockConfigBuilder, Region},
    operation::converse::ConverseOutput,
    primitives::Blob,
    types::{
        ContentBlock, ConversationRole, ImageBlock, ImageFormat, ImageSource,
        InferenceConfiguration, Message, SystemContentBlock,
    },
};
use aws_smithy_http_client::{
    Builder as HttpClientBuilder,
    tls::{Provider as TlsProvider, rustls_provider::CryptoMode},
};
use serde_json::Value;

use super::*;

fn bedrock_image_format(media_type: &str) -> ImageFormat {
    ImageFormat::from(
        media_type
            .split_once('/')
            .map_or(media_type, |(_, format)| format),
    )
}

fn bedrock_content(
    user_message: &str,
    images: &[ImageRef],
) -> Result<Vec<ContentBlock>, SemanticError> {
    let mut content = Vec::new();
    for image in images {
        let Some(raw) = &image.raw else {
            continue;
        };
        let image = ImageBlock::builder()
            .format(bedrock_image_format(&image.media_type))
            .source(ImageSource::Bytes(Blob::new(raw.clone())))
            .build()
            .map_err(|error| {
                SemanticError::InvalidProviderConfiguration(format!(
                    "could not build Bedrock image block: {error}"
                ))
            })?;
        content.push(ContentBlock::Image(image));
    }
    content.push(ContentBlock::Text(with_image_notes(
        user_message,
        images,
        false,
    )));
    Ok(content)
}

fn bedrock_inference_config(
    backend: &ResolvedBackend,
) -> Result<InferenceConfiguration, SemanticError> {
    bedrock_inference_config_with_max(backend, backend.max_output_tokens)
}

fn bedrock_inference_config_with_max(
    backend: &ResolvedBackend,
    max_output_tokens: usize,
) -> Result<InferenceConfiguration, SemanticError> {
    let max_tokens = i32::try_from(max_output_tokens).map_err(|_| {
        SemanticError::InvalidProviderConfiguration(format!(
            "Bedrock max output token count {} exceeds i32",
            max_output_tokens
        ))
    })?;
    let temperature = backend.temperature.map(|value| value as f32);
    if temperature.is_some_and(|value| !value.is_finite()) {
        return Err(SemanticError::InvalidProviderConfiguration(
            "Bedrock temperature must be finite".to_owned(),
        ));
    }
    Ok(InferenceConfiguration::builder()
        .max_tokens(max_tokens)
        .set_temperature(temperature)
        .build())
}

async fn bedrock_client(
    backend: &ResolvedBackend,
    environment: &HashMap<String, String>,
) -> Client {
    let region = environment
        .get("AWS_REGION")
        .or_else(|| environment.get("AWS_DEFAULT_REGION"))
        .map(String::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or("us-east-1");
    let http_client = HttpClientBuilder::new()
        .tls_provider(TlsProvider::Rustls(CryptoMode::Ring))
        .build_https();
    let mut loader = aws_config::defaults(BehaviorVersion::latest())
        .http_client(http_client)
        .region(Region::new(region.to_owned()));
    if let Some(profile) = environment
        .get("AWS_PROFILE")
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        loader = loader.profile_name(profile);
    }
    let shared = loader.load().await;
    let max_attempts = u32::try_from(backend.max_retries.saturating_add(1)).unwrap_or(u32::MAX);
    let service_config = BedrockConfigBuilder::from(&shared)
        .retry_config(RetryConfig::standard().with_max_attempts(max_attempts))
        .timeout_config(
            TimeoutConfig::builder()
                .operation_timeout(backend.timeout)
                .build(),
        )
        .build();
    Client::from_conf(service_config)
}

fn normalize_bedrock_response(
    response: &ConverseOutput,
    model: &str,
) -> Result<Value, SemanticError> {
    let raw_content = response
        .output()
        .and_then(|output| output.as_message().ok())
        .and_then(|message| message.content().first())
        .and_then(|content| content.as_text().ok())
        .map(String::as_str)
        .unwrap_or("{}");
    let mut result = parse_llm_json(raw_content);
    let object = result.as_object_mut().ok_or_else(|| {
        SemanticError::InvalidProviderResponse(
            "parsed Bedrock fragment was not an object".to_owned(),
        )
    })?;
    let usage = response.usage();
    object.insert(
        "input_tokens".to_owned(),
        Value::from(
            usage
                .map(|usage| usage.input_tokens().max(0) as u64)
                .unwrap_or(0),
        ),
    );
    object.insert(
        "output_tokens".to_owned(),
        Value::from(
            usage
                .map(|usage| usage.output_tokens().max(0) as u64)
                .unwrap_or(0),
        ),
    );
    object.insert("model".to_owned(), Value::String(model.to_owned()));
    let finish = if response.stop_reason().as_str() == "max_tokens" {
        "length"
    } else {
        "stop"
    };
    object.insert("finish_reason".to_owned(), Value::String(finish.to_owned()));
    if response_is_hollow(Some(raw_content), &result)
        && result.get("finish_reason").and_then(Value::as_str) != Some("length")
    {
        result["finish_reason"] = Value::String("length".to_owned());
    }
    Ok(result)
}

async fn execute_bedrock_backend_async(
    backend: &ResolvedBackend,
    user_message: &str,
    images: &[ImageRef],
    deep_mode: bool,
    environment: &HashMap<String, String>,
) -> Result<Value, SemanticError> {
    if backend.backend.name != "bedrock" {
        return Err(SemanticError::InvalidProviderConfiguration(format!(
            "backend {:?} is not the Bedrock backend",
            backend.backend.name
        )));
    }
    let message = Message::builder()
        .role(ConversationRole::User)
        .set_content(Some(bedrock_content(user_message, images)?))
        .build()
        .map_err(|error| {
            SemanticError::InvalidProviderConfiguration(format!(
                "could not build Bedrock message: {error}"
            ))
        })?;
    let response = bedrock_client(backend, environment)
        .await
        .converse()
        .model_id(&backend.model)
        .system(SystemContentBlock::Text(extraction_prompt(deep_mode)))
        .messages(message)
        .inference_config(bedrock_inference_config(backend)?)
        .send()
        .await
        .map_err(|error| SemanticError::Transport(format!("Bedrock API error: {error}")))?;
    normalize_bedrock_response(&response, &backend.model)
}

async fn execute_bedrock_plain_text_async(
    backend: &ResolvedBackend,
    prompt: &str,
    max_tokens: usize,
    environment: &HashMap<String, String>,
) -> Result<PlainTextResponse, SemanticError> {
    if backend.backend.name != "bedrock" {
        return Err(SemanticError::InvalidProviderConfiguration(format!(
            "backend {:?} is not the Bedrock backend",
            backend.backend.name
        )));
    }
    let message = Message::builder()
        .role(ConversationRole::User)
        .content(ContentBlock::Text(prompt.to_owned()))
        .build()
        .map_err(|error| {
            SemanticError::InvalidProviderConfiguration(format!(
                "could not build Bedrock message: {error}"
            ))
        })?;
    let response = bedrock_client(backend, environment)
        .await
        .converse()
        .model_id(&backend.model)
        .messages(message)
        .inference_config(bedrock_inference_config_with_max(backend, max_tokens)?)
        .send()
        .await
        .map_err(|error| SemanticError::Transport(format!("Bedrock API error: {error}")))?;
    Ok(normalize_bedrock_plain_response(&response, &backend.model))
}

fn normalize_bedrock_plain_response(response: &ConverseOutput, model: &str) -> PlainTextResponse {
    let text = response
        .output()
        .and_then(|output| output.as_message().ok())
        .and_then(|message| message.content().first())
        .and_then(|content| content.as_text().ok())
        .cloned()
        .unwrap_or_default();
    let usage = response.usage();
    PlainTextResponse {
        text,
        input_tokens: usage
            .and_then(|usage| u64::try_from(usage.input_tokens()).ok())
            .unwrap_or(0),
        output_tokens: usage
            .and_then(|usage| u64::try_from(usage.output_tokens()).ok())
            .unwrap_or(0),
        model: model.to_owned(),
    }
}

pub(super) fn execute_bedrock_plain_text(
    backend: &ResolvedBackend,
    prompt: &str,
    max_tokens: usize,
    environment: &HashMap<String, String>,
) -> Result<PlainTextResponse, SemanticError> {
    let run = || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|error| SemanticError::Transport(format!("Bedrock runtime: {error}")))?;
        runtime.block_on(execute_bedrock_plain_text_async(
            backend,
            prompt,
            max_tokens,
            environment,
        ))
    };
    if tokio::runtime::Handle::try_current().is_ok() {
        return std::thread::scope(|scope| {
            scope
                .spawn(run)
                .join()
                .map_err(|_| SemanticError::Transport("Bedrock runtime panicked".to_owned()))?
        });
    }
    run()
}

/// Execute Bedrock Converse with AWS environment/profile/SSO/container/instance
/// credentials and a Rustls+ring HTTP client that requires no native library.
pub fn execute_bedrock_backend(
    backend: &ResolvedBackend,
    user_message: &str,
    images: &[ImageRef],
    deep_mode: bool,
    environment: &HashMap<String, String>,
) -> Result<Value, SemanticError> {
    let run = || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|error| SemanticError::Transport(format!("Bedrock runtime: {error}")))?;
        runtime.block_on(execute_bedrock_backend_async(
            backend,
            user_message,
            images,
            deep_mode,
            environment,
        ))
    };
    if tokio::runtime::Handle::try_current().is_ok() {
        return std::thread::scope(|scope| {
            scope
                .spawn(run)
                .join()
                .map_err(|_| SemanticError::Transport("Bedrock runtime panicked".to_owned()))?
        });
    }
    run()
}

#[cfg(test)]
mod tests {
    use aws_sdk_bedrockruntime::types::{ConverseOutput as BedrockOutput, StopReason, TokenUsage};

    use super::*;

    #[test]
    fn bedrock_content_and_normalization_follow_converse_contract()
    -> Result<(), Box<dyn std::error::Error>> {
        let environment = HashMap::from([("AWS_REGION".to_owned(), "us-west-2".to_owned())]);
        let backend = resolve_builtin_backend("bedrock", &environment, Some("bedrock-model"))?;
        let images = [
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
        let content = bedrock_content("source", &images)?;
        assert_eq!(content.len(), 2);
        let image = content[0]
            .as_image()
            .map_err(|_| "first Bedrock content block was not an image")?;
        assert_eq!(image.format(), &ImageFormat::Png);
        assert_eq!(
            image
                .source()
                .and_then(|source| source.as_bytes().ok())
                .map(Blob::as_ref),
            Some([0_u8, 1, 2].as_slice())
        );
        let text = content[1]
            .as_text()
            .map_err(|_| "last Bedrock content block was not text")?;
        assert!(text.contains("source_file: diagram.png"));
        assert!(text.contains("large.webp (not shown"));
        let inference = bedrock_inference_config(&backend)?;
        assert_eq!(inference.max_tokens(), Some(16_384));
        assert_eq!(inference.temperature(), Some(0.0));

        let fragment = r#"{"nodes":[{"id":"doc"}],"edges":[]}"#;
        let message = Message::builder()
            .role(ConversationRole::Assistant)
            .content(ContentBlock::Text(fragment.to_owned()))
            .build()?;
        let usage = TokenUsage::builder()
            .input_tokens(11)
            .output_tokens(7)
            .total_tokens(18)
            .build()?;
        let response = ConverseOutput::builder()
            .output(BedrockOutput::Message(message))
            .stop_reason(StopReason::MaxTokens)
            .usage(usage)
            .build()?;
        let normalized = normalize_bedrock_response(&response, "bedrock-model")?;
        assert_eq!(normalized["nodes"][0]["id"], "doc");
        assert_eq!(normalized["input_tokens"], 11);
        assert_eq!(normalized["output_tokens"], 7);
        assert_eq!(normalized["model"], "bedrock-model");
        assert_eq!(normalized["finish_reason"], "length");
        assert_eq!(
            normalize_bedrock_plain_response(&response, "bedrock-model"),
            PlainTextResponse {
                text: fragment.to_owned(),
                input_tokens: 11,
                output_tokens: 7,
                model: "bedrock-model".to_owned(),
            }
        );
        Ok(())
    }

    #[test]
    fn bedrock_rejects_invalid_inference_settings_and_normalizes_edge_responses()
    -> Result<(), Box<dyn std::error::Error>> {
        let environment = HashMap::from([("AWS_REGION".to_owned(), "us-west-2".to_owned())]);
        let mut backend = resolve_builtin_backend("bedrock", &environment, Some("bedrock-model"))?;

        let jpeg = ImageRef {
            path: PathBuf::from("/corpus/photo.jpeg"),
            relative_path: "photo.jpeg".to_owned(),
            media_type: "jpeg".to_owned(),
            raw: Some(vec![3, 4, 5]),
        };
        let content = bedrock_content("inspect", &[jpeg])?;
        assert_eq!(
            content[0]
                .as_image()
                .map_err(|_| "expected image block")?
                .format(),
            &ImageFormat::Jpeg
        );
        assert_eq!(bedrock_content("plain", &[])?.len(), 1);

        assert!(bedrock_inference_config_with_max(&backend, usize::MAX).is_err());
        backend.temperature = Some(f64::NAN);
        assert!(bedrock_inference_config(&backend).is_err());

        let usage = TokenUsage::builder()
            .input_tokens(-11)
            .output_tokens(-7)
            .total_tokens(-18)
            .build()?;
        let hollow_message = Message::builder()
            .role(ConversationRole::Assistant)
            .content(ContentBlock::Text("{}".to_owned()))
            .build()?;
        let hollow = ConverseOutput::builder()
            .output(BedrockOutput::Message(hollow_message))
            .stop_reason(StopReason::EndTurn)
            .usage(usage.clone())
            .build()?;
        let normalized = normalize_bedrock_response(&hollow, "edge-model")?;
        assert_eq!(normalized["input_tokens"], 0);
        assert_eq!(normalized["output_tokens"], 0);
        assert_eq!(normalized["finish_reason"], "length");

        let array_message = Message::builder()
            .role(ConversationRole::Assistant)
            .content(ContentBlock::Text("[]".to_owned()))
            .build()?;
        let array = ConverseOutput::builder()
            .output(BedrockOutput::Message(array_message))
            .stop_reason(StopReason::EndTurn)
            .usage(usage.clone())
            .build()?;
        let array_normalized = normalize_bedrock_response(&array, "edge-model")?;
        assert_eq!(array_normalized["nodes"], serde_json::json!([]));
        assert_eq!(array_normalized["finish_reason"], "length");

        let image_message = Message::builder()
            .role(ConversationRole::Assistant)
            .content(ContentBlock::Image(
                ImageBlock::builder()
                    .format(ImageFormat::Png)
                    .source(ImageSource::Bytes(Blob::new(vec![9])))
                    .build()?,
            ))
            .build()?;
        let image_only = ConverseOutput::builder()
            .output(BedrockOutput::Message(image_message))
            .stop_reason(StopReason::EndTurn)
            .usage(usage)
            .build()?;
        let plain = normalize_bedrock_plain_response(&image_only, "edge-model");
        assert!(plain.text.is_empty());
        assert_eq!(plain.input_tokens, 0);
        assert_eq!(plain.output_tokens, 0);
        Ok(())
    }

    #[test]
    fn bedrock_sync_wrappers_reject_other_backends_inside_and_outside_runtime()
    -> Result<(), Box<dyn std::error::Error>> {
        let environment = HashMap::from([("OPENAI_API_KEY".to_owned(), "test-key".to_owned())]);
        let backend = resolve_builtin_backend("openai", &environment, Some("test-model"))?;

        let assert_rejected = || {
            let extraction = execute_bedrock_backend(&backend, "extract", &[], false, &environment);
            assert!(matches!(
                extraction,
                Err(SemanticError::InvalidProviderConfiguration(_))
            ));
            let plain = execute_bedrock_plain_text(&backend, "plain", 32, &environment);
            assert!(matches!(
                plain,
                Err(SemanticError::InvalidProviderConfiguration(_))
            ));
        };

        assert_rejected();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async { assert_rejected() });
        Ok(())
    }
}
