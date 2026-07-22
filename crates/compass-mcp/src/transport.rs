use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use rmcp::ServiceExt;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio_util::sync::CancellationToken;
use tower_http::limit::RequestBodyLimitLayer;

use crate::GraphifyMcp;

const MAX_HTTP_REQUEST_BYTES: usize = 16 * 1024 * 1024;
const MAX_HTTP_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
const MAX_STDIO_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

/// Configuration for the Graphify-compatible MCP Streamable HTTP transport.
#[derive(Clone, Debug)]
pub struct HttpOptions {
    pub graph_path: PathBuf,
    pub host: String,
    pub port: u16,
    pub api_key: Option<String>,
    pub path: String,
    pub json_response: bool,
    pub stateless: bool,
    pub session_timeout: Option<Duration>,
}

impl HttpOptions {
    #[must_use]
    pub fn new(graph_path: PathBuf) -> Self {
        Self {
            graph_path,
            host: "127.0.0.1".to_owned(),
            port: 8080,
            api_key: None,
            path: "/mcp".to_owned(),
            json_response: false,
            stateless: false,
            session_timeout: Some(Duration::from_secs(3600)),
        }
    }
}

#[derive(Clone)]
struct HttpGate {
    api_key: Option<Arc<[u8]>>,
    convert_stateful_sse_to_json: bool,
}

/// Serve MCP over stdio while tolerating blank lines sent by desktop clients.
pub async fn serve_stdio(graph_path: PathBuf) -> Result<(), String> {
    let (mut relay_write, relay_read) = tokio::io::duplex(64 * 1024);
    let relay = tokio::spawn(async move {
        let mut input = BufReader::new(tokio::io::stdin());
        let mut line = Vec::new();
        loop {
            line.clear();
            let count = (&mut input)
                .take((MAX_STDIO_MESSAGE_BYTES + 1) as u64)
                .read_until(b'\n', &mut line)
                .await
                .map_err(|error| error.to_string())?;
            if count == 0 {
                break;
            }
            if line.len() > MAX_STDIO_MESSAGE_BYTES {
                return Err(format!(
                    "MCP stdio message exceeds {MAX_STDIO_MESSAGE_BYTES} bytes"
                ));
            }
            if line.iter().any(|byte| !byte.is_ascii_whitespace()) {
                relay_write
                    .write_all(&line)
                    .await
                    .map_err(|error| error.to_string())?;
            }
        }
        Ok::<(), String>(())
    });
    let running = GraphifyMcp::new(graph_path)
        .serve((relay_read, tokio::io::stdout()))
        .await
        .map_err(|error| error.to_string())?;
    running.waiting().await.map_err(|error| error.to_string())?;
    relay.abort();
    Ok(())
}

/// Serve MCP over Streamable HTTP until Ctrl+C or process cancellation.
pub async fn serve_http(mut options: HttpOptions) -> Result<(), String> {
    if !options.path.starts_with('/') || options.path.contains('?') || options.path.contains('#') {
        return Err(
            "HTTP mount path must start with '/' and contain no query or fragment".to_owned(),
        );
    }
    options.api_key = options
        .api_key
        .take()
        .map(|key| key.trim().to_owned())
        .filter(|key| !key.is_empty());

    let cancellation = CancellationToken::new();
    let router = build_http_router(&options, &cancellation);

    let bind_host = if options.host.is_empty() {
        "0.0.0.0"
    } else {
        &options.host
    };
    let listener = tokio::net::TcpListener::bind((bind_host, options.port))
        .await
        .map_err(|error| format!("could not bind {bind_host}:{}: {error}", options.port))?;
    let auth_note = if options.api_key.is_some() {
        "api-key required"
    } else {
        "no auth (set --api-key to require one)"
    };
    eprintln!(
        "graphify MCP server (streamable-http) on http://{}:{}{} - {auth_note}",
        options.host, options.port, options.path
    );
    if is_wildcard_host(&options.host) && options.api_key.is_none() {
        eprintln!(
            "WARNING: binding {} with no api-key exposes the graph unauthenticated on the network. Set --api-key (or GRAPHIFY_API_KEY).",
            if options.host.is_empty() {
                "0.0.0.0"
            } else {
                &options.host
            }
        );
    }

    let shutdown = cancellation.clone();
    let shutdown_signal = async move {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            () = shutdown.cancelled_owned() => {}
        }
    };
    let result = axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal)
        .await;
    cancellation.cancel();
    result.map_err(|error| error.to_string())
}

fn build_http_router(options: &HttpOptions, cancellation: &CancellationToken) -> Router {
    let mut manager = LocalSessionManager::default();
    manager.session_config.keep_alive = if options.stateless {
        None
    } else {
        options.session_timeout.filter(|timeout| !timeout.is_zero())
    };
    let manager = Arc::new(manager);
    let factory_graph = GraphifyMcp::new(options.graph_path.clone());
    let mut config = StreamableHttpServerConfig::default()
        .with_stateful_mode(!options.stateless)
        .with_json_response(options.json_response)
        .with_cancellation_token(cancellation.child_token());
    if is_wildcard_host(&options.host) {
        config = config.disable_allowed_hosts();
    } else {
        config = config.with_allowed_hosts(allowed_hosts(&options.host, options.port));
    }
    let service = StreamableHttpService::new(move || Ok(factory_graph.clone()), manager, config);
    let gate = HttpGate {
        api_key: options
            .api_key
            .as_ref()
            .map(|key| Arc::<[u8]>::from(key.as_bytes())),
        // rmcp 2.2 emits SSE for stateful responses even when json_response is
        // requested. The Python SDK returns plain JSON, so adapt that response.
        convert_stateful_sse_to_json: options.json_response && !options.stateless,
    };
    Router::new()
        .route_service(&options.path, service)
        .layer(RequestBodyLimitLayer::new(MAX_HTTP_REQUEST_BYTES))
        .layer(middleware::from_fn_with_state(gate, http_gate))
}

async fn http_gate(State(gate): State<HttpGate>, request: Request, next: Next) -> Response {
    if let Some(expected) = &gate.api_key {
        let provided = request
            .headers()
            .get("x-api-key")
            .map(HeaderValue::as_bytes)
            .or_else(|| bearer_token(request.headers().get(header::AUTHORIZATION)));
        if !provided.is_some_and(|value| constant_time_eq(value, expected)) {
            return (
                StatusCode::UNAUTHORIZED,
                [(header::CONTENT_TYPE, "application/json")],
                "{\"error\": \"unauthorized\"}",
            )
                .into_response();
        }
    }
    let method = request.method().clone();
    let response = next.run(request).await;
    if gate.convert_stateful_sse_to_json && method == Method::POST {
        return sse_response_to_json(response).await;
    }
    response
}

fn bearer_token(value: Option<&HeaderValue>) -> Option<&[u8]> {
    let value = value?.as_bytes();
    let separator = value.iter().position(|byte| *byte == b' ')?;
    let (scheme, rest) = value.split_at(separator);
    if !scheme.eq_ignore_ascii_case(b"bearer") {
        return None;
    }
    let token = rest.get(1..)?;
    let start = token.iter().position(|byte| !byte.is_ascii_whitespace())?;
    let end = token.iter().rposition(|byte| !byte.is_ascii_whitespace())?;
    token.get(start..=end)
}

fn constant_time_eq(provided: &[u8], expected: &[u8]) -> bool {
    let length = provided.len().max(expected.len());
    let mut difference = provided.len() ^ expected.len();
    for index in 0..length {
        difference |= usize::from(provided.get(index).copied().unwrap_or_default())
            ^ usize::from(expected.get(index).copied().unwrap_or_default());
    }
    difference == 0
}

async fn sse_response_to_json(response: Response) -> Response {
    let is_sse = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/event-stream"));
    if !is_sse {
        return response;
    }
    let (mut parts, body) = response.into_parts();
    let Ok(bytes) = to_bytes(body, MAX_HTTP_RESPONSE_BYTES).await else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "MCP response exceeded the server limit",
        )
            .into_response();
    };
    let payload = bytes
        .split(|byte| *byte == b'\n')
        .filter_map(|line| line.strip_prefix(b"data:"))
        .map(trim_ascii)
        .find(|line| serde_json::from_slice::<serde_json::Value>(line).is_ok());
    let Some(payload) = payload else {
        return Response::from_parts(parts, Body::from(bytes));
    };
    parts.headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    parts.headers.remove(header::CONTENT_LENGTH);
    Response::from_parts(parts, Body::from(payload.to_vec()))
}

fn trim_ascii(mut value: &[u8]) -> &[u8] {
    while value.first().is_some_and(u8::is_ascii_whitespace) {
        value = &value[1..];
    }
    while value.last().is_some_and(u8::is_ascii_whitespace) {
        value = &value[..value.len().saturating_sub(1)];
    }
    value
}

fn is_wildcard_host(host: &str) -> bool {
    matches!(host, "" | "0.0.0.0" | "::")
}

fn allowed_hosts(host: &str, port: u16) -> Vec<String> {
    let mut hosts = vec![
        host.to_owned(),
        "localhost".to_owned(),
        "127.0.0.1".to_owned(),
    ];
    hosts.extend(
        [host, "localhost", "127.0.0.1"]
            .into_iter()
            .map(|name| format_authority(name, port)),
    );
    hosts.sort();
    hosts.dedup();
    hosts
}

fn format_authority(host: &str, port: u16) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::net::SocketAddr;

    use serde_json::Value;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const INITIALIZE: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}"#;

    #[test]
    fn api_keys_compare_without_early_exit() {
        assert!(constant_time_eq(b"secret", b"secret"));
        assert!(!constant_time_eq(b"secrex", b"secret"));
        assert!(!constant_time_eq(b"secret-long", b"secret"));
    }

    #[test]
    fn bearer_scheme_is_case_insensitive_and_token_is_trimmed() {
        let value = HeaderValue::from_static("bEaReR   secret  ");
        assert_eq!(bearer_token(Some(&value)), Some(b"secret".as_slice()));
        assert_eq!(bearer_token(None), None);
        assert_eq!(
            bearer_token(Some(&HeaderValue::from_static("Basic secret"))),
            None
        );
        assert_eq!(
            bearer_token(Some(&HeaderValue::from_static("Bearer"))),
            None
        );
        assert_eq!(
            bearer_token(Some(&HeaderValue::from_static("Bearer    "))),
            None
        );
    }

    #[test]
    fn local_bind_hosts_match_python_security_policy() {
        let hosts = allowed_hosts("127.0.0.1", 8080);
        assert!(hosts.iter().any(|host| host == "localhost:8080"));
        assert!(hosts.iter().any(|host| host == "127.0.0.1"));
        assert_eq!(format_authority("::1", 9000), "[::1]:9000");
        assert_eq!(format_authority("[::1]", 9000), "[::1]:9000");
        assert!(is_wildcard_host(""));
        assert!(is_wildcard_host("0.0.0.0"));
        assert!(is_wildcard_host("::"));
        assert!(!is_wildcard_host("localhost"));
        assert_eq!(trim_ascii(b" \t value \r\n"), b"value");
        assert!(trim_ascii(b" \t\r\n").is_empty());
    }

    #[tokio::test]
    async fn sse_conversion_preserves_non_sse_and_recovers_json_payloads()
    -> Result<(), Box<dyn std::error::Error>> {
        let plain = Response::builder()
            .status(StatusCode::ACCEPTED)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from("{\"plain\":true}"))?;
        let plain = sse_response_to_json(plain).await;
        assert_eq!(plain.status(), StatusCode::ACCEPTED);
        assert_eq!(to_bytes(plain.into_body(), 1024).await?, "{\"plain\":true}");

        let sse = Response::builder()
            .header(header::CONTENT_TYPE, "text/event-stream; charset=utf-8")
            .header(header::CONTENT_LENGTH, "999")
            .body(Body::from(
                "event: message\ndata: not-json\ndata:  {\"jsonrpc\":\"2.0\",\"id\":1}  \n\n",
            ))?;
        let converted = sse_response_to_json(sse).await;
        assert_eq!(
            converted.headers().get(header::CONTENT_TYPE),
            Some(&HeaderValue::from_static("application/json"))
        );
        assert!(!converted.headers().contains_key(header::CONTENT_LENGTH));
        assert_eq!(
            to_bytes(converted.into_body(), 1024).await?,
            "{\"jsonrpc\":\"2.0\",\"id\":1}"
        );

        let invalid = Response::builder()
            .header(header::CONTENT_TYPE, "text/event-stream")
            .body(Body::from("event: ping\ndata: invalid\n\n"))?;
        let invalid = sse_response_to_json(invalid).await;
        assert_eq!(
            to_bytes(invalid.into_body(), 1024).await?,
            "event: ping\ndata: invalid\n\n"
        );
        Ok(())
    }

    #[tokio::test]
    async fn http_configuration_rejects_bad_mounts_and_unbindable_hosts() {
        let mut options = HttpOptions::new(PathBuf::from("missing.json"));
        for path in ["mcp", "/mcp?query", "/mcp#fragment"] {
            options.path = path.to_owned();
            assert!(serve_http(options.clone()).await.is_err(), "{path}");
        }
        options.path = "/mcp".to_owned();
        options.host = "not a valid host".to_owned();
        assert!(serve_http(options).await.is_err());
    }

    #[test]
    fn router_configuration_covers_wildcard_stateless_and_empty_credentials() {
        let mut options = HttpOptions::new(PathBuf::from("missing.json"));
        options.host.clear();
        options.api_key = Some(String::new());
        options.stateless = true;
        options.json_response = true;
        options.session_timeout = Some(Duration::ZERO);
        let cancellation = CancellationToken::new();
        let _router = build_http_router(&options, &cancellation);
    }

    #[tokio::test]
    async fn stateful_json_transport_enforces_auth_and_lists_tools()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::tempdir()?;
        let graph = temp.path().join("graph.json");
        fs::write(
            &graph,
            r#"{"directed":true,"nodes":[{"id":"a","label":"Alpha","community":0}],"links":[]}"#,
        )?;
        let mut options = HttpOptions::new(graph);
        options.api_key = Some("s3cret".to_owned());
        options.json_response = true;
        let cancellation = CancellationToken::new();
        let router = build_http_router(&options, &cancellation);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let server_cancel = cancellation.clone();
        let server = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(server_cancel.cancelled_owned())
                .await
        });

        let unauthorized = request(address, "127.0.0.1", None, None, INITIALIZE).await?;
        assert!(unauthorized.starts_with("HTTP/1.1 401 Unauthorized"));
        assert!(unauthorized.ends_with("{\"error\": \"unauthorized\"}"));

        let initialized = request(
            address,
            "127.0.0.1",
            Some("Bearer s3cret"),
            None,
            INITIALIZE,
        )
        .await?;
        assert!(initialized.starts_with("HTTP/1.1 200 OK"));
        assert!(
            initialized
                .to_lowercase()
                .contains("content-type: application/json")
        );
        let session = header_value(&initialized, "mcp-session-id").ok_or("missing session id")?;
        let payload: Value = serde_json::from_str(response_body(&initialized))?;
        assert_eq!(payload["result"]["serverInfo"]["name"], "graphify");

        let listed = request(
            address,
            "127.0.0.1",
            Some("bearer s3cret"),
            Some(&session),
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#,
        )
        .await?;
        let payload: Value = serde_json::from_str(response_body(&listed))?;
        assert_eq!(
            payload["result"]["tools"].as_array().map(Vec::len),
            Some(10)
        );

        cancellation.cancel();
        server.await??;
        Ok(())
    }

    #[tokio::test]
    async fn local_http_transport_rejects_untrusted_host() -> Result<(), Box<dyn std::error::Error>>
    {
        let temp = tempfile::tempdir()?;
        let graph = temp.path().join("graph.json");
        fs::write(&graph, r#"{"directed":true,"nodes":[],"links":[]}"#)?;
        let mut options = HttpOptions::new(graph);
        options.json_response = true;
        options.stateless = true;
        let cancellation = CancellationToken::new();
        let router = build_http_router(&options, &cancellation);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let server_cancel = cancellation.clone();
        let server = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(server_cancel.cancelled_owned())
                .await
        });
        let response = request(address, "attacker.example", None, None, INITIALIZE).await?;
        assert!(response.starts_with("HTTP/1.1 403 Forbidden"));
        cancellation.cancel();
        server.await??;
        Ok(())
    }

    async fn request(
        address: SocketAddr,
        host: &str,
        authorization: Option<&str>,
        session: Option<&str>,
        body: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let mut stream = tokio::net::TcpStream::connect(address).await?;
        let authorization = authorization
            .map(|value| format!("Authorization: {value}\r\n"))
            .unwrap_or_default();
        let session = session
            .map(|value| format!("Mcp-Session-Id: {value}\r\n"))
            .unwrap_or_default();
        let wire = format!(
            "POST /mcp HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\n{authorization}{session}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(wire.as_bytes()).await?;
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await?;
        Ok(String::from_utf8(response)?)
    }

    fn response_body(response: &str) -> &str {
        response.split_once("\r\n\r\n").map_or("", |(_, body)| body)
    }

    fn header_value(response: &str, name: &str) -> Option<String> {
        response
            .lines()
            .filter_map(|line| line.split_once(':'))
            .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.trim().to_owned())
    }
}
