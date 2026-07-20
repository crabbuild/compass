//! Bounded, SSRF-resistant URL ingestion compatible with Graphify's corpus files.

use std::fs;
use std::io::{ErrorKind, Read as _};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs as _};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;

use regex::Regex;
use serde_json::Value;
use time::{OffsetDateTime, UtcOffset};
use trail_files::{write_bytes_atomic, write_text_atomic};
use trail_transcribe::AudioDownloader as _;
use trail_transcribe::downloader::{ManagedYtDlp, validate_public_url};
use ureq::unversioned::resolver::{ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{DefaultConnector, NextTimeout};
use url::Url;

const MAX_BINARY_BYTES: u64 = 52_428_800;
const MAX_TEXT_BYTES: u64 = 10_485_760;
const USER_AGENT: &str = "Mozilla/5.0 graphify/1.0";

static UNSAFE_FILENAME: LazyLock<Regex> = LazyLock::new(|| compile_regex(r"[^\w\-]"));
static REPEATED_UNDERSCORE: LazyLock<Regex> = LazyLock::new(|| compile_regex(r"_+"));
static SCRIPT: LazyLock<Regex> = LazyLock::new(|| compile_regex(r"(?is)<script[^>]*>.*?</script>"));
static STYLE: LazyLock<Regex> = LazyLock::new(|| compile_regex(r"(?is)<style[^>]*>.*?</style>"));
static TAG: LazyLock<Regex> = LazyLock::new(|| compile_regex(r"(?s)<[^>]+>"));
static WHITESPACE: LazyLock<Regex> = LazyLock::new(|| compile_regex(r"\s+"));
static TITLE: LazyLock<Regex> = LazyLock::new(|| compile_regex(r"(?is)<title[^>]*>(.*?)</title>"));
static ARXIV_ID: LazyLock<Regex> = LazyLock::new(|| compile_regex(r"(\d{4}\.\d{4,5})"));
static ARXIV_ABSTRACT: LazyLock<Regex> =
    LazyLock::new(|| compile_regex(r#"(?is)class="abstract[^"]*"[^>]*>(.*?)</blockquote>"#));
static ARXIV_TITLE: LazyLock<Regex> =
    LazyLock::new(|| compile_regex(r#"(?is)class="title[^"]*"[^>]*>(.*?)</h1>"#));
static ARXIV_AUTHORS: LazyLock<Regex> =
    LazyLock::new(|| compile_regex(r#"(?is)class="authors"[^>]*>(.*?)</div>"#));

fn compile_regex(pattern: &str) -> Regex {
    match Regex::new(pattern) {
        Ok(regex) => regex,
        Err(_) => std::process::abort(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UrlKind {
    Tweet,
    Arxiv,
    Github,
    Youtube,
    Pdf,
    Image,
    Webpage,
}

impl UrlKind {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Tweet => "tweet",
            Self::Arxiv => "arxiv",
            Self::Github => "github",
            Self::Youtube => "youtube",
            Self::Pdf => "PDF",
            Self::Image => "image",
            Self::Webpage => "webpage",
        }
    }
}

#[derive(Debug, Clone)]
pub struct IngestRequest<'a> {
    pub url: &'a str,
    pub target_dir: &'a Path,
    pub author: Option<&'a str>,
    pub contributor: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestResult {
    pub path: PathBuf,
    pub message: String,
    pub kind: UrlKind,
}

#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    #[error("ingest: {0}")]
    RejectedUrl(String),
    #[error("ingest: failed to fetch {url:?}: {message}")]
    Fetch { url: String, message: String },
    #[error("ingest: could not create {path}: {source}")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("ingest: could not save {path}: {message}")]
    Persist { path: PathBuf, message: String },
    #[error("ingest: YouTube audio download failed: {0}")]
    Youtube(String),
}

pub trait Fetcher {
    fn fetch(&self, url: &str, max_bytes: u64, timeout: Duration) -> Result<Vec<u8>, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SafeHttpFetcher;

impl Fetcher for SafeHttpFetcher {
    fn fetch(&self, url: &str, max_bytes: u64, timeout: Duration) -> Result<Vec<u8>, String> {
        validate_public_url(url).map_err(|error| error.to_string())?;
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .max_redirects(10)
            .proxy(None)
            .build();
        let agent = ureq::Agent::with_parts(config, DefaultConnector::default(), PublicResolver);
        let response = agent
            .get(url)
            .header("User-Agent", USER_AGENT)
            .call()
            .map_err(|error| error.to_string())?;
        let limit = max_bytes
            .checked_add(1)
            .ok_or_else(|| "response size limit overflowed".to_owned())?;
        let mut bytes = Vec::new();
        response
            .into_body()
            .into_with_config()
            .limit(limit)
            .reader()
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > max_bytes {
            return Err(format!(
                "Response from {url:?} exceeds size limit ({} MB). Aborting download.",
                max_bytes / 1_048_576
            ));
        }
        Ok(bytes)
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct PublicResolver;

impl Resolver for PublicResolver {
    fn resolve(
        &self,
        uri: &ureq::http::Uri,
        _config: &ureq::config::Config,
        _timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let host = uri
            .host()
            .ok_or_else(|| ureq::Error::BadUri("URL has no host".to_owned()))?;
        if matches!(
            host.to_ascii_lowercase().as_str(),
            "metadata.google.internal" | "metadata.google.com"
        ) {
            return Err(blocked_io(format!(
                "blocked cloud metadata endpoint {host:?}"
            )));
        }
        let port = uri.port_u16().unwrap_or_else(|| {
            if uri.scheme_str() == Some("http") {
                80
            } else {
                443
            }
        });
        let mut resolved = self.empty();
        for address in (host, port).to_socket_addrs().map_err(ureq::Error::Io)? {
            if ip_is_blocked(address.ip()) {
                return Err(blocked_io(format!(
                    "blocked private or reserved address {} for {host:?}",
                    address.ip()
                )));
            }
            if resolved.len() < 16 {
                resolved.push(address);
            }
        }
        if resolved.is_empty() {
            return Err(ureq::Error::HostNotFound);
        }
        Ok(resolved)
    }
}

fn blocked_io(message: String) -> ureq::Error {
    ureq::Error::Io(std::io::Error::new(ErrorKind::PermissionDenied, message))
}

pub fn ingest(request: &IngestRequest<'_>) -> Result<IngestResult, IngestError> {
    fs::create_dir_all(request.target_dir).map_err(|source| IngestError::CreateDirectory {
        path: request.target_dir.to_path_buf(),
        source,
    })?;
    let kind = detect_url_type(request.url)?;
    if kind == UrlKind::Youtube {
        validate_public_url(request.url)
            .map_err(|error| IngestError::RejectedUrl(error.to_string()))?;
        let mut downloader = ManagedYtDlp::from_environment()
            .map_err(|error| IngestError::Youtube(error.to_string()))?;
        let path = downloader
            .download_audio(request.url, request.target_dir)
            .map_err(IngestError::Youtube)?;
        return Ok(IngestResult {
            message: format!(
                "Downloaded audio: {}",
                path.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default()
            ),
            path,
            kind,
        });
    }
    ingest_with(request, OffsetDateTime::now_utc(), &SafeHttpFetcher)
}

pub fn ingest_with(
    request: &IngestRequest<'_>,
    captured_at: OffsetDateTime,
    fetcher: &dyn Fetcher,
) -> Result<IngestResult, IngestError> {
    fs::create_dir_all(request.target_dir).map_err(|source| IngestError::CreateDirectory {
        path: request.target_dir.to_path_buf(),
        source,
    })?;
    let kind = detect_url_type(request.url)?;
    if kind == UrlKind::Youtube {
        return Err(IngestError::Youtube(
            "testable ingestion requires the managed downloader for YouTube URLs".to_owned(),
        ));
    }
    validate_url_syntax(request.url)?;

    if kind == UrlKind::Pdf || kind == UrlKind::Image {
        let bytes = fetcher
            .fetch(request.url, MAX_BINARY_BYTES, Duration::from_secs(30))
            .map_err(|message| IngestError::Fetch {
                url: request.url.to_owned(),
                message,
            })?;
        let suffix = if kind == UrlKind::Pdf {
            ".pdf".to_owned()
        } else {
            path_suffix(raw_path(request.url)).unwrap_or_else(|| ".jpg".to_owned())
        };
        let filename = safe_filename(request.url, &suffix);
        let path = request.target_dir.join(&filename);
        write_bytes_atomic(&path, &bytes).map_err(|error| IngestError::Persist {
            path: path.clone(),
            message: error.to_string(),
        })?;
        return Ok(IngestResult {
            message: format!("Downloaded {}: {filename}", kind.label()),
            path,
            kind,
        });
    }

    let (content, filename) = match kind {
        UrlKind::Tweet => fetch_tweet(request, captured_at, fetcher),
        UrlKind::Arxiv => fetch_arxiv(request, captured_at, fetcher),
        UrlKind::Github | UrlKind::Webpage => fetch_webpage(request, captured_at, fetcher),
        UrlKind::Youtube | UrlKind::Pdf | UrlKind::Image => {
            return Err(IngestError::Fetch {
                url: request.url.to_owned(),
                message: "internal URL classification mismatch".to_owned(),
            });
        }
    }?;
    let path = available_markdown_path(request.target_dir, &filename);
    write_text_atomic(&path, &content).map_err(|error| IngestError::Persist {
        path: path.clone(),
        message: error.to_string(),
    })?;
    Ok(IngestResult {
        message: format!(
            "Saved {}: {}",
            kind.label(),
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
        ),
        path,
        kind,
    })
}

pub fn detect_url_type(input: &str) -> Result<UrlKind, IngestError> {
    let parsed = Url::parse(input).map_err(|error| {
        let scheme = input
            .split_once(':')
            .map(|(scheme, _)| scheme)
            .filter(|scheme| {
                !scheme.is_empty()
                    && scheme.chars().enumerate().all(|(index, character)| {
                        if index == 0 {
                            character.is_ascii_alphabetic()
                        } else {
                            character.is_ascii_alphanumeric()
                                || matches!(character, '+' | '-' | '.')
                        }
                    })
            })
            .unwrap_or_default();
        if scheme.is_empty() {
            IngestError::RejectedUrl(format!(
                "Blocked URL scheme '' - only http and https are allowed. Got: '{input}'"
            ))
        } else {
            IngestError::RejectedUrl(error.to_string())
        }
    })?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(IngestError::RejectedUrl(format!(
            "Blocked URL scheme '{}' - only http and https are allowed. Got: '{}'",
            parsed.scheme(),
            input
        )));
    }
    let lower = input.to_lowercase();
    if lower.contains("twitter.com") || lower.contains("x.com") {
        return Ok(UrlKind::Tweet);
    }
    if lower.contains("arxiv.org") {
        return Ok(UrlKind::Arxiv);
    }
    if lower.contains("github.com") {
        return Ok(UrlKind::Github);
    }
    if lower.contains("youtube.com") || lower.contains("youtu.be") {
        return Ok(UrlKind::Youtube);
    }
    let path = parsed.path().to_lowercase();
    if path.ends_with(".pdf") {
        return Ok(UrlKind::Pdf);
    }
    if [".png", ".jpg", ".jpeg", ".webp", ".gif"]
        .iter()
        .any(|extension| path.ends_with(extension))
    {
        return Ok(UrlKind::Image);
    }
    Ok(UrlKind::Webpage)
}

fn fetch_tweet(
    request: &IngestRequest<'_>,
    captured_at: OffsetDateTime,
    fetcher: &dyn Fetcher,
) -> Result<(String, String), IngestError> {
    let normalized = request.url.replace("x.com", "twitter.com");
    let api = format!(
        "https://publish.twitter.com/oembed?url={}&omit_script=true",
        python_quote(&normalized)
    );
    let fetched = fetcher.fetch(&api, MAX_TEXT_BYTES, Duration::from_secs(15));
    let (text, author) = fetched
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|value| {
            let html = value.get("html")?.as_str()?;
            let text = TAG.replace_all(html, "").trim().to_owned();
            let author = value
                .get("author_name")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_owned();
            Some((text, author))
        })
        .unwrap_or_else(|| {
            (
                format!("Tweet at {} (could not fetch content)", request.url),
                "unknown".to_owned(),
            )
        });
    let contributor = request.contributor.or(request.author).unwrap_or("unknown");
    let timestamp = python_timestamp(captured_at);
    let content = format!(
        "---\nsource_url: \"{}\"\ntype: tweet\nauthor: \"{}\"\ncaptured_at: {timestamp}\ncontributor: \"{}\"\n---\n\n# Tweet by @{author}\n\n{text}\n\nSource: {}\n",
        yaml_str(request.url),
        yaml_str(&author),
        yaml_str(contributor),
        request.url
    );
    Ok((content, safe_filename(request.url, ".md")))
}

fn fetch_webpage(
    request: &IngestRequest<'_>,
    captured_at: OffsetDateTime,
    fetcher: &dyn Fetcher,
) -> Result<(String, String), IngestError> {
    let html = fetch_text(request.url, fetcher)?;
    let title = TITLE
        .captures(&html)
        .and_then(|captures| captures.get(1))
        .map_or_else(
            || request.url.to_owned(),
            |value| collapse_whitespace(value.as_str()),
        );
    let markdown = html_to_markdown(&html);
    let contributor = request.contributor.or(request.author).unwrap_or("unknown");
    let timestamp = python_timestamp(captured_at);
    let content = format!(
        "---\nsource_url: \"{}\"\ntype: webpage\ntitle: \"{}\"\ncaptured_at: {timestamp}\ncontributor: \"{}\"\n---\n\n# {title}\n\nSource: {}\n\n---\n\n{}\n",
        yaml_str(request.url),
        yaml_str(&title),
        yaml_str(contributor),
        request.url,
        truncate_chars(&markdown, 12_000)
    );
    Ok((content, safe_filename(request.url, ".md")))
}

fn fetch_arxiv(
    request: &IngestRequest<'_>,
    captured_at: OffsetDateTime,
    fetcher: &dyn Fetcher,
) -> Result<(String, String), IngestError> {
    let Some(identifier) = ARXIV_ID
        .captures(request.url)
        .and_then(|captures| captures.get(1))
        .map(|value| value.as_str().to_owned())
    else {
        return fetch_webpage(request, captured_at, fetcher);
    };
    let api = format!("https://export.arxiv.org/abs/{identifier}");
    let html = fetcher
        .fetch(&api, MAX_TEXT_BYTES, Duration::from_secs(15))
        .ok()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
    let (title, abstract_text, authors) = html.map_or_else(
        || (identifier.clone(), String::new(), String::new()),
        |html| {
            let abstract_text = capture_tag_text(&ARXIV_ABSTRACT, &html, "");
            let title = capture_tag_text(&ARXIV_TITLE, &html, " ");
            let authors = capture_tag_text(&ARXIV_AUTHORS, &html, "");
            let title = if title.is_empty() {
                identifier.clone()
            } else {
                title
            };
            (title, abstract_text, authors)
        },
    );
    let contributor = request.contributor.or(request.author).unwrap_or("unknown");
    let timestamp = python_timestamp(captured_at);
    let content = format!(
        "---\nsource_url: \"{}\"\narxiv_id: \"{}\"\ntype: paper\ntitle: \"{}\"\npaper_authors: \"{}\"\ncaptured_at: {timestamp}\ncontributor: \"{}\"\n---\n\n# {title}\n\n**Authors:** {authors}\n**arXiv:** {identifier}\n\n## Abstract\n\n{abstract_text}\n\nSource: {}\n",
        yaml_str(request.url),
        yaml_str(&identifier),
        yaml_str(&title),
        yaml_str(&authors),
        yaml_str(contributor),
        request.url
    );
    Ok((
        content,
        format!("arxiv_{}.md", identifier.replace('.', "_")),
    ))
}

fn fetch_text(url: &str, fetcher: &dyn Fetcher) -> Result<String, IngestError> {
    fetcher
        .fetch(url, MAX_TEXT_BYTES, Duration::from_secs(15))
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .map_err(|message| IngestError::Fetch {
            url: url.to_owned(),
            message,
        })
}

fn capture_tag_text(pattern: &Regex, html: &str, replacement: &str) -> String {
    pattern
        .captures(html)
        .and_then(|captures| captures.get(1))
        .map(|value| {
            TAG.replace_all(value.as_str(), replacement)
                .trim()
                .to_owned()
        })
        .unwrap_or_default()
}

fn validate_url_syntax(input: &str) -> Result<Url, IngestError> {
    let url = Url::parse(input).map_err(|error| IngestError::RejectedUrl(error.to_string()))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(IngestError::RejectedUrl(format!(
            "Blocked URL scheme '{}' - only http and https are allowed. Got: '{}'",
            url.scheme(),
            input
        )));
    }
    let host = url
        .host_str()
        .ok_or_else(|| IngestError::RejectedUrl("URL has no host".to_owned()))?;
    if matches!(
        host.to_ascii_lowercase().as_str(),
        "metadata.google.internal" | "metadata.google.com"
    ) {
        return Err(IngestError::RejectedUrl(format!(
            "blocked cloud metadata endpoint {host:?}"
        )));
    }
    if let Ok(address) = host.parse::<IpAddr>()
        && ip_is_blocked(address)
    {
        return Err(IngestError::RejectedUrl(format!(
            "Blocked private/internal IP {address} (resolved from '{host}'). Got: '{input}'"
        )));
    }
    Ok(url)
}

fn html_to_markdown(html: &str) -> String {
    let html = SCRIPT.replace_all(html, "");
    let html = STYLE.replace_all(&html, "");
    let text = TAG.replace_all(&html, " ");
    truncate_chars(&collapse_whitespace(&text), 8_000)
}

fn collapse_whitespace(value: &str) -> String {
    WHITESPACE.replace_all(value, " ").trim().to_owned()
}

fn truncate_chars(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

fn available_markdown_path(target_dir: &Path, filename: &str) -> PathBuf {
    let mut path = target_dir.join(filename);
    let stem = Path::new(filename)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    let mut counter = 1;
    while path.exists() && counter < 1_000 {
        path = target_dir.join(format!("{stem}_{counter}.md"));
        counter += 1;
    }
    path
}

fn safe_filename(url: &str, suffix: &str) -> String {
    let name = format!("{}{}", raw_authority(url), raw_path(url));
    let name = UNSAFE_FILENAME.replace_all(&name, "_");
    let name = name.trim_matches('_');
    let name = REPEATED_UNDERSCORE.replace_all(name, "_");
    format!("{}{suffix}", truncate_chars(&name, 80))
}

fn path_suffix(path: &str) -> Option<String> {
    let name = path.rsplit('/').next().unwrap_or_default();
    let index = name.rfind('.')?;
    (index > 0 && index + 1 < name.len()).then(|| name[index..].to_owned())
}

fn raw_authority(url: &str) -> &str {
    url.split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(url)
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
}

fn raw_path(url: &str) -> &str {
    let rest = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let after_authority = rest.find('/').map_or("", |index| &rest[index..]);
    after_authority.split(['?', '#']).next().unwrap_or_default()
}

fn yaml_str(value: &str) -> String {
    let mut output = String::new();
    for character in value.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            '\0' => output.push_str("\\0"),
            '\u{2028}' => output.push_str("\\L"),
            '\u{2029}' => output.push_str("\\P"),
            value if (value as u32) < 0x20 || value == '\u{7f}' => {
                output.push_str(&format!("\\x{:02x}", value as u32));
            }
            value => output.push(value),
        }
    }
    output
}

fn python_timestamp(value: OffsetDateTime) -> String {
    let value = value.to_offset(UtcOffset::UTC);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:06}+00:00",
        value.year(),
        u8::from(value.month()),
        value.day(),
        value.hour(),
        value.minute(),
        value.second(),
        value.nanosecond() / 1_000
    )
}

fn python_quote(value: &str) -> String {
    let mut output = String::new();
    for byte in value.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(*byte, b'_' | b'-' | b'.' | b'~' | b'/') {
            output.push(char::from(*byte));
        } else {
            output.push_str(&format!("%{byte:02X}"));
        }
    }
    output
}

fn ip_is_blocked(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => ipv4_is_blocked(address),
        IpAddr::V6(address) => {
            if let Some(mapped) = address.to_ipv4_mapped() {
                return ipv4_is_blocked(mapped);
            }
            if let Some(nat64) = nat64_embedded_ipv4(address) {
                return ipv4_is_blocked(nat64);
            }
            let segments = address.segments();
            address.is_unspecified()
                || address.is_loopback()
                || address.is_multicast()
                || (segments[0] & 0xfe00) == 0xfc00
                || (segments[0] & 0xffc0) == 0xfe80
                || (segments[0] == 0x2001 && segments[1] == 0x0db8)
        }
    }
}

fn ipv4_is_blocked(address: Ipv4Addr) -> bool {
    let [a, b, c, _] = address.octets();
    address.is_private()
        || address.is_loopback()
        || address.is_link_local()
        || address.is_broadcast()
        || address.is_unspecified()
        || address.is_multicast()
        || a == 0
        || a >= 240
        || (a == 100 && (64..=127).contains(&b))
        || (a == 192 && b == 0 && matches!(c, 0 | 2))
        || (a == 198 && matches!(b, 18 | 19))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113)
}

fn nat64_embedded_ipv4(address: Ipv6Addr) -> Option<Ipv4Addr> {
    let octets = address.octets();
    (octets[..12] == [0, 0x64, 0xff, 0x9b, 0, 0, 0, 0, 0, 0, 0, 0])
        .then(|| Ipv4Addr::new(octets[12], octets[13], octets[14], octets[15]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct FixtureFetcher(HashMap<String, Vec<u8>>);

    impl Fetcher for FixtureFetcher {
        fn fetch(&self, url: &str, _max_bytes: u64, _timeout: Duration) -> Result<Vec<u8>, String> {
            self.0
                .get(url)
                .cloned()
                .ok_or_else(|| format!("missing fixture for {url}"))
        }
    }

    fn captured_at() -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(1_700_000_000)
            .unwrap_or(OffsetDateTime::UNIX_EPOCH)
            .replace_nanosecond(123_456_000)
            .unwrap_or(OffsetDateTime::UNIX_EPOCH)
    }

    #[test]
    fn webpage_fallback_matches_python_contract() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let url = "https://example.com/a-page?q=1";
        let html = "<html><head><title>A  &amp;\n B</title><style>secret</style></head><body><h1>Hello</h1><script>bad()</script><p>world</p></body></html>";
        let fetcher = FixtureFetcher(HashMap::from([(url.to_owned(), html.as_bytes().to_vec())]));
        let result = ingest_with(
            &IngestRequest {
                url,
                target_dir: directory.path(),
                author: Some("Alice"),
                contributor: None,
            },
            captured_at(),
            &fetcher,
        )?;
        assert_eq!(result.message, "Saved webpage: example_com_a-page.md");
        assert_eq!(
            fs::read_to_string(result.path)?,
            "---\nsource_url: \"https://example.com/a-page?q=1\"\ntype: webpage\ntitle: \"A &amp; B\"\ncaptured_at: 2023-11-14T22:13:20.123456+00:00\ncontributor: \"Alice\"\n---\n\n# A &amp; B\n\nSource: https://example.com/a-page?q=1\n\n---\n\nA &amp; B Hello world\n"
        );
        Ok(())
    }

    #[test]
    fn binary_and_collision_paths_are_bounded_and_deterministic()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let url = "https://example.com/assets/picture.JPG";
        let fetcher = FixtureFetcher(HashMap::from([(url.to_owned(), vec![1, 2, 3])]));
        let request = IngestRequest {
            url,
            target_dir: directory.path(),
            author: None,
            contributor: None,
        };
        let first = ingest_with(&request, captured_at(), &fetcher)?;
        assert_eq!(
            first.message,
            "Downloaded image: example_com_assets_picture_JPG.JPG"
        );
        assert_eq!(fs::read(first.path)?, vec![1, 2, 3]);

        let page = "https://example.com/page";
        let pages = FixtureFetcher(HashMap::from([(page.to_owned(), b"hello".to_vec())]));
        let page_request = IngestRequest {
            url: page,
            ..request
        };
        let first = ingest_with(&page_request, captured_at(), &pages)?;
        let second = ingest_with(&page_request, captured_at(), &pages)?;
        assert_ne!(first.path, second.path);
        assert!(second.path.ends_with("example_com_page_1.md"));
        Ok(())
    }

    #[test]
    fn tweet_and_arxiv_documents_preserve_legacy_metadata() -> Result<(), Box<dyn std::error::Error>>
    {
        let directory = tempfile::tempdir()?;
        let tweet = "https://x.com/dev/status/123";
        let oembed = format!(
            "https://publish.twitter.com/oembed?url={}&omit_script=true",
            python_quote("https://twitter.com/dev/status/123")
        );
        let fetcher = FixtureFetcher(HashMap::from([(
            oembed,
            br#"{"html":"<blockquote>Hello <b>graph</b></blockquote>","author_name":"Dev"}"#
                .to_vec(),
        )]));
        let result = ingest_with(
            &IngestRequest {
                url: tweet,
                target_dir: directory.path(),
                author: None,
                contributor: Some("Team"),
            },
            captured_at(),
            &fetcher,
        )?;
        let content = fs::read_to_string(result.path)?;
        assert!(content.contains("type: tweet\nauthor: \"Dev\""));
        assert!(content.contains("# Tweet by @Dev\n\nHello graph"));
        assert!(content.contains("contributor: \"Team\""));

        let arxiv = "https://arxiv.org/abs/1706.03762";
        let api = "https://export.arxiv.org/abs/1706.03762";
        let html = r#"<h1 class="title mathjax">Title: Attention</h1><div class="authors"><a>Alice</a>, <a>Bob</a></div><blockquote class="abstract mathjax">Abstract: <span>Useful</span></blockquote>"#;
        let papers = FixtureFetcher(HashMap::from([(api.to_owned(), html.as_bytes().to_vec())]));
        let result = ingest_with(
            &IngestRequest {
                url: arxiv,
                target_dir: directory.path(),
                author: None,
                contributor: None,
            },
            captured_at(),
            &papers,
        )?;
        assert_eq!(
            result.path.file_name().and_then(|name| name.to_str()),
            Some("arxiv_1706_03762.md")
        );
        let content = fs::read_to_string(result.path)?;
        assert!(content.contains("title: \"Title: Attention\""));
        assert!(content.contains("paper_authors: \"Alice, Bob\""));
        assert!(content.contains("Abstract: Useful"));
        Ok(())
    }

    #[test]
    fn url_classification_and_yaml_escaping_match_python() {
        assert_eq!(
            detect_url_type("https://arxiv.org/abs/1706.03762").ok(),
            Some(UrlKind::Arxiv)
        );
        assert_eq!(
            detect_url_type("https://x.com/user/status/1").ok(),
            Some(UrlKind::Tweet)
        );
        assert_eq!(
            detect_url_type("https://youtu.be/id").ok(),
            Some(UrlKind::Youtube)
        );
        assert_eq!(
            yaml_str("a\t\0\u{2028}\u{2029}\"\\"),
            "a\\t\\0\\L\\P\\\"\\\\"
        );
        assert_eq!(python_quote("https://x.com/a b"), "https%3A//x.com/a%20b");
        assert_eq!(path_suffix("/dir.with-dot/file"), None);
        assert_eq!(path_suffix("/file.tar.gz"), Some(".gz".to_owned()));
        assert_eq!(path_suffix("/.hidden"), None);
    }

    #[test]
    fn invalid_ingest_still_creates_requested_directory_like_python() {
        let directory = tempfile::tempdir().ok();
        let Some(directory) = directory else {
            return;
        };
        let target = directory.path().join("raw");
        let result = ingest(&IngestRequest {
            url: "file:///etc/passwd",
            target_dir: &target,
            author: None,
            contributor: None,
        });
        assert!(result.is_err());
        assert!(target.is_dir());
    }

    #[test]
    fn private_address_policy_blocks_all_internal_families() {
        for address in [
            "127.0.0.1",
            "10.1.2.3",
            "100.64.0.1",
            "169.254.1.1",
            "192.0.2.1",
            "::1",
            "fc00::1",
            "64:ff9b::7f00:1",
        ] {
            assert!(
                address.parse::<IpAddr>().is_ok_and(ip_is_blocked),
                "{address}"
            );
        }
    }
}
