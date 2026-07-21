use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::time::Duration;

use time::OffsetDateTime;
use trail_ingest::{
    Fetcher, IngestRequest, SafeHttpFetcher, UrlKind, detect_url_type, ingest, ingest_with,
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

#[test]
fn rich_tweet_arxiv_and_webpage_fixtures_escape_metadata_and_extract_html()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let captured = now()?;
    let tweet = "https://x.com/dev/status/42";
    let tweet_api = "https://publish.twitter.com/oembed?url=https%3A//twitter.com/dev/status/42&omit_script=true";
    let tweet_result = ingest_with(
        &IngestRequest {
            url: tweet,
            target_dir: directory.path(),
            author: Some("fallback"),
            contributor: Some("line\nreturn\rcontrol\u{1f}\t\0\u{2028}\u{2029}\\\""),
        },
        captured,
        &Fixtures(HashMap::from([(
            tweet_api.to_owned(),
            Ok(br#"{"html":"<blockquote><p>Hello <b>graph</b></p></blockquote>","author_name":"Dev"}"#.to_vec()),
        )])),
    )?;
    let tweet_text = fs::read_to_string(tweet_result.path)?;
    assert!(tweet_text.contains("# Tweet by @Dev"));
    assert!(tweet_text.contains("Hello graph"));
    assert!(tweet_text.contains("line\\nreturn\\rcontrol\\x1f\\t\\0\\L\\P\\\\\\\""));

    let arxiv = "https://arxiv.org/abs/2401.54321";
    let arxiv_api = "https://export.arxiv.org/abs/2401.54321";
    let arxiv_result = ingest_with(
        &IngestRequest {
            url: arxiv,
            target_dir: directory.path(),
            author: None,
            contributor: Some("researcher"),
        },
        captured,
        &Fixtures(HashMap::from([(
            arxiv_api.to_owned(),
            Ok(br#"<h1 class="title mathjax">Title: <span>Trail Graphs</span></h1><div class="authors"><a>Ada</a>, <a>Lin</a></div><blockquote class="abstract mathjax">Abstract: <b>Fast</b> graphs.</blockquote>"#.to_vec()),
        )])),
    )?;
    let arxiv_text = fs::read_to_string(arxiv_result.path)?;
    assert!(arxiv_text.contains("# Title:"));
    assert!(arxiv_text.contains("Trail Graphs"));
    assert!(arxiv_text.contains("**Authors:** Ada, Lin"));
    assert!(arxiv_text.contains("Abstract: Fast graphs."));

    let page = "https://github.com/org/repo";
    let page_result = ingest_with(
        &IngestRequest {
            url: page,
            target_dir: directory.path(),
            author: None,
            contributor: None,
        },
        captured,
        &Fixtures(HashMap::from([(
            page.to_owned(),
            Ok(b"<style>hidden</style><script>bad()</script><main>Repository body</main>".to_vec()),
        )])),
    )?;
    let page_text = fs::read_to_string(page_result.path)?;
    assert!(page_text.contains("Repository body"));
    assert!(!page_text.contains("hidden"));
    assert!(!page_text.contains("bad()"));
    Ok(())
}

#[test]
fn binary_fetch_persist_and_reserved_address_failures_are_explicit() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let captured = now()?;
    let pdf = "https://example.com/file.pdf";
    let fetch_failure = ingest_with(
        &IngestRequest {
            url: pdf,
            target_dir: directory.path(),
            author: None,
            contributor: None,
        },
        captured,
        &Fixtures(HashMap::from([(
            pdf.to_owned(),
            Err("too large".to_owned()),
        )])),
    );
    assert!(fetch_failure.is_err());

    let image = "https://example.com/image";
    let image_url = format!("{image}.png");
    fs::create_dir(directory.path().join("example_com_image_png.png"))?;
    let persist_failure = ingest_with(
        &IngestRequest {
            url: &image_url,
            target_dir: directory.path(),
            author: None,
            contributor: None,
        },
        captured,
        &Fixtures(HashMap::from([(image_url.clone(), Ok(vec![1, 2, 3]))])),
    );
    assert!(persist_failure.is_err());

    for invalid in [
        "http://[::ffff:127.0.0.1]/",
        "http://[2001:db8::1]/",
        "http://198.18.0.1/",
        "http://198.51.100.2/",
        "http://203.0.113.3/",
    ] {
        assert!(
            ingest_with(
                &IngestRequest {
                    url: invalid,
                    target_dir: directory.path(),
                    author: None,
                    contributor: None,
                },
                captured,
                &Fixtures(HashMap::new()),
            )
            .is_err(),
            "{invalid}"
        );
    }
    assert!(detect_url_type("git+ssh:").is_err());
    assert!(detect_url_type("a+b:c").is_err());

    let blocked_youtube = IngestRequest {
        url: "http://127.0.0.1/youtube.com/watch?v=x",
        target_dir: directory.path(),
        author: None,
        contributor: None,
    };
    assert!(ingest(&blocked_youtube).is_err());
    Ok(())
}
