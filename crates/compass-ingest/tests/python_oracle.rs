use std::collections::HashMap;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use compass_ingest::{Fetcher, IngestRequest, ingest_with};
use serde_json::Value;
use time::OffsetDateTime;

struct FixtureFetcher(HashMap<String, Vec<u8>>);

impl Fetcher for FixtureFetcher {
    fn fetch(&self, url: &str, _max_bytes: u64, _timeout: Duration) -> Result<Vec<u8>, String> {
        self.0
            .get(url)
            .cloned()
            .ok_or_else(|| format!("missing fixture for {url}"))
    }
}

fn repository_root() -> PathBuf {
    if let Some(root) = std::env::var_os("GRAPHIFY_REPO_ROOT") {
        return PathBuf::from(root);
    }
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .map_or_else(|| PathBuf::from("../.."), Path::to_path_buf)
}

fn captured_at() -> Result<OffsetDateTime, Box<dyn Error>> {
    Ok(OffsetDateTime::from_unix_timestamp(1_700_000_000)?.replace_nanosecond(123_456_000)?)
}

fn python_artifact(kind: &str, url: &str, payload: &str) -> Result<Value, Box<dyn Error>> {
    let repo = repository_root();
    let python = if cfg!(windows) {
        repo.join(".venv/Scripts/python.exe")
    } else {
        repo.join(".venv/bin/python")
    };
    let script = r#"
import json, os
from datetime import datetime as RealDateTime, timezone
import graphify.ingest as ingest

class FrozenDateTime:
    @classmethod
    def now(cls, tz=None):
        return RealDateTime(2023, 11, 14, 22, 13, 20, 123456, tzinfo=timezone.utc)

ingest.datetime = FrozenDateTime
kind = os.environ['ORACLE_KIND']
url = os.environ['ORACLE_URL']
payload = os.environ['ORACLE_PAYLOAD']
ingest._fetch_html = lambda _url: payload
ingest.safe_fetch_text = lambda _url: payload
if kind == 'webpage':
    content, filename = ingest._fetch_webpage(url, 'Alice', None)
elif kind == 'tweet':
    content, filename = ingest._fetch_tweet(url, None, 'Team')
else:
    content, filename = ingest._fetch_arxiv(url, None, None)
print(json.dumps({'content': content, 'filename': filename}, ensure_ascii=False))
"#;
    let output = Command::new(python)
        .args(["-c", script])
        .current_dir(&repo)
        .env("PYTHONPATH", &repo)
        .env("ORACLE_KIND", kind)
        .env("ORACLE_URL", url)
        .env("ORACLE_PAYLOAD", payload)
        .output()?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned().into());
    }
    Ok(serde_json::from_slice(&output.stdout)?)
}

fn compare(
    kind: &str,
    url: &str,
    payload_url: &str,
    payload: &str,
    author: Option<&str>,
    contributor: Option<&str>,
) -> Result<(), Box<dyn Error>> {
    let expected = python_artifact(kind, url, payload)?;
    let directory = tempfile::tempdir()?;
    let fetcher = FixtureFetcher(HashMap::from([(
        payload_url.to_owned(),
        payload.as_bytes().to_vec(),
    )]));
    let result = ingest_with(
        &IngestRequest {
            url,
            target_dir: directory.path(),
            author,
            contributor,
        },
        captured_at()?,
        &fetcher,
    )?;
    assert_eq!(
        result.path.file_name().and_then(|name| name.to_str()),
        expected.get("filename").and_then(Value::as_str),
        "{kind} filename"
    );
    assert_eq!(
        std::fs::read_to_string(result.path)?,
        expected
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        "{kind} content"
    );
    Ok(())
}

#[test]
fn generated_markdown_matches_python_oracle_bytes() -> Result<(), Box<dyn Error>> {
    let webpage = "https://example.com/a-page?q=1";
    compare(
        "webpage",
        webpage,
        webpage,
        "<html><head><title>A  &amp;\n B</title><style>secret</style></head><body><h1>Hello</h1><script>bad()</script><p>world</p></body></html>",
        Some("Alice"),
        None,
    )?;

    let tweet = "https://x.com/dev/status/123";
    compare(
        "tweet",
        tweet,
        "https://publish.twitter.com/oembed?url=https%3A//twitter.com/dev/status/123&omit_script=true",
        r#"{"html":"<blockquote>Hello <b>graph</b></blockquote>","author_name":"Dev"}"#,
        None,
        Some("Team"),
    )?;

    let arxiv = "https://arxiv.org/abs/1706.03762";
    compare(
        "arxiv",
        arxiv,
        "https://export.arxiv.org/abs/1706.03762",
        r#"<h1 class="title mathjax">Title: Attention</h1><div class="authors"><a>Alice</a>, <a>Bob</a></div><blockquote class="abstract mathjax">Abstract: <span>Useful</span></blockquote>"#,
        None,
        None,
    )?;
    Ok(())
}
