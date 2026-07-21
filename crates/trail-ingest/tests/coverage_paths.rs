use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::time::Duration;

use time::OffsetDateTime;
use trail_ingest::{
    Fetcher, IngestRequest, SafeHttpFetcher, UrlKind, detect_url_type, ingest_with,
};

struct Fixtures(HashMap<String, Result<Vec<u8>, String>>);

impl Fetcher for Fixtures {
    fn fetch(&self, url: &str, _max_bytes: u64, _timeout: Duration) -> Result<Vec<u8>, String> {
        self.0
            .get(url)
            .cloned()
            .unwrap_or_else(|| Err(format!("missing {url}")))
    }
}

fn now() -> Result<OffsetDateTime, Box<dyn Error>> {
    Ok(OffsetDateTime::from_unix_timestamp(1_700_000_000)?)
}

#[test]
fn classification_labels_and_url_rejections_cover_every_public_kind() {
    for (url, kind, label) in [
        ("https://twitter.com/a/status/1", UrlKind::Tweet, "tweet"),
        ("https://arxiv.org/pdf/1234.56789", UrlKind::Arxiv, "arxiv"),
        ("https://github.com/o/r", UrlKind::Github, "github"),
        ("https://youtube.com/watch?v=x", UrlKind::Youtube, "youtube"),
        ("https://example.com/a.PDF", UrlKind::Pdf, "PDF"),
        ("https://example.com/a.webp", UrlKind::Image, "image"),
        ("https://example.com/", UrlKind::Webpage, "webpage"),
    ] {
        assert_eq!(detect_url_type(url).ok(), Some(kind), "{url}");
        assert_eq!(kind.label(), label);
    }
    for invalid in [
        "relative/path",
        "://broken",
        "file:///etc/passwd",
        "ftp://example.com/a",
    ] {
        assert!(detect_url_type(invalid).is_err(), "{invalid}");
    }
}

#[test]
fn offline_fallbacks_cover_tweet_arxiv_webpage_binary_and_youtube_errors()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let captured = now()?;

    let tweet = ingest_with(
        &IngestRequest {
            url: "https://x.com/dev/status/1",
            target_dir: directory.path(),
            author: None,
            contributor: None,
        },
        captured,
        &Fixtures(HashMap::new()),
    )?;
    assert!(fs::read_to_string(tweet.path)?.contains("could not fetch content"));

    let arxiv_url = "https://arxiv.org/abs/2401.12345";
    let arxiv = ingest_with(
        &IngestRequest {
            url: arxiv_url,
            target_dir: directory.path(),
            author: Some("author"),
            contributor: None,
        },
        captured,
        &Fixtures(HashMap::new()),
    )?;
    assert!(fs::read_to_string(arxiv.path)?.contains("# 2401.12345"));

    let arxiv_without_id = "https://arxiv.org/help";
    let fallback = ingest_with(
        &IngestRequest {
            url: arxiv_without_id,
            target_dir: directory.path(),
            author: None,
            contributor: None,
        },
        captured,
        &Fixtures(HashMap::from([(
            arxiv_without_id.to_owned(),
            Ok(b"<body>Help</body>".to_vec()),
        )])),
    )?;
    assert!(fs::read_to_string(fallback.path)?.contains("# https://arxiv.org/help"));

    for (url, bytes) in [
        ("https://example.com/file.pdf", b"pdf".as_slice()),
        ("https://example.com/image.gif", b"gif".as_slice()),
    ] {
        let result = ingest_with(
            &IngestRequest {
                url,
                target_dir: directory.path(),
                author: None,
                contributor: None,
            },
            captured,
            &Fixtures(HashMap::from([(url.to_owned(), Ok(bytes.to_vec()))])),
        )?;
        assert_eq!(fs::read(result.path)?, bytes);
    }

    let youtube = ingest_with(
        &IngestRequest {
            url: "https://youtu.be/x",
            target_dir: directory.path(),
            author: None,
            contributor: None,
        },
        captured,
        &Fixtures(HashMap::new()),
    );
    assert!(youtube.is_err());

    let failed = ingest_with(
        &IngestRequest {
            url: "https://example.com/fail",
            target_dir: directory.path(),
            author: None,
            contributor: None,
        },
        captured,
        &Fixtures(HashMap::from([(
            "https://example.com/fail".to_owned(),
            Err("offline".to_owned()),
        )])),
    );
    assert!(failed.is_err());
    Ok(())
}

#[test]
fn syntax_private_network_and_target_creation_failures_are_rejected_before_fetch()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let target_file = directory.path().join("not-a-directory");
    fs::write(&target_file, b"x")?;
    let request = |url| IngestRequest {
        url,
        target_dir: directory.path(),
        author: None,
        contributor: None,
    };
    for url in [
        "http://127.0.0.1/page",
        "http://[::1]/page",
        "https://metadata.google.internal/computeMetadata/v1/",
        "https://metadata.google.com/",
    ] {
        assert!(ingest_with(&request(url), now()?, &Fixtures(HashMap::new())).is_err());
        assert!(
            SafeHttpFetcher
                .fetch(url, 1, Duration::from_millis(1))
                .is_err()
        );
    }
    let creation = ingest_with(
        &IngestRequest {
            url: "https://example.com/",
            target_dir: &target_file,
            author: None,
            contributor: None,
        },
        now()?,
        &Fixtures(HashMap::new()),
    );
    assert!(creation.is_err());
    Ok(())
}
