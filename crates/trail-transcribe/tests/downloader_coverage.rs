use std::error::Error;
use std::ffi::OsString;
use std::io::{Cursor, Read};
use std::path::Path;
use std::time::Duration;

use trail_transcribe::downloader::{
    DownloaderError, SystemToolRunner, ToolCache, ToolFetcher, ToolRunner, ToolSpec,
    validate_public_url, yt_dlp_arguments,
};

struct BytesFetcher(Result<Vec<u8>, String>);

impl ToolFetcher for BytesFetcher {
    fn fetch(&self, _url: &str, _max_bytes: u64) -> Result<Box<dyn Read>, String> {
        self.0
            .as_ref()
            .map(|bytes| Box::new(Cursor::new(bytes.clone())) as Box<dyn Read>)
            .map_err(Clone::clone)
    }
}

fn spec(size: u64, sha256: &'static str) -> ToolSpec {
    ToolSpec {
        version: "coverage",
        asset: "fixture-helper",
        installed_name: "fixture-helper",
        size,
        sha256,
    }
}

#[test]
fn tool_cache_rejects_fetch_size_and_digest_failures() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let cache = ToolCache::new(directory.path().to_path_buf());

    let fetch = cache.ensure(
        spec(1, "00"),
        &BytesFetcher(Err("offline fixture".to_owned())),
    );
    assert!(matches!(fetch, Err(DownloaderError::ToolDownload { .. })));

    let too_large = cache.ensure(spec(1, "00"), &BytesFetcher(Ok(vec![1, 2])));
    assert!(matches!(too_large, Err(DownloaderError::ToolSize { .. })));

    let too_small = cache.ensure(spec(2, "00"), &BytesFetcher(Ok(vec![1])));
    assert!(matches!(too_small, Err(DownloaderError::ToolSize { .. })));

    let wrong_digest = cache.ensure(spec(1, "00"), &BytesFetcher(Ok(vec![1])));
    assert!(matches!(wrong_digest, Err(DownloaderError::ToolDigest(_))));
    Ok(())
}

#[test]
fn public_url_and_argument_contracts_cover_rejections_and_exact_boundaries() {
    for rejected in [
        "not a url",
        "file:///etc/passwd",
        "https://metadata.google.com/computeMetadata/v1/",
        "http://127.0.0.1/",
        "http://[::1]/",
        "http://[64:ff9b::7f00:1]/",
    ] {
        assert!(validate_public_url(rejected).is_err(), "{rejected}");
    }

    let arguments = yt_dlp_arguments(
        Path::new("out/yt_fixture.%(ext)s"),
        1234,
        "https://93.184.216.34/watch?v=1",
    );
    assert_eq!(arguments.first(), Some(&OsString::from("--ignore-config")));
    assert!(
        arguments
            .windows(2)
            .any(|pair| { pair == [OsString::from("--max-filesize"), OsString::from("1234")] })
    );
    assert_eq!(
        arguments.last(),
        Some(&OsString::from("https://93.184.216.34/watch?v=1"))
    );
}

#[cfg(unix)]
fn executable_script(
    directory: &Path,
    name: &str,
    source: &str,
) -> Result<std::path::PathBuf, Box<dyn Error>> {
    use std::os::unix::fs::PermissionsExt as _;

    let path = directory.join(name);
    std::fs::write(&path, source)?;
    let mut permissions = std::fs::metadata(&path)?.permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&path, permissions)?;
    Ok(path)
}

#[cfg(unix)]
#[test]
fn system_runner_bounds_process_success_failure_output_and_time() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let success = executable_script(
        directory.path(),
        "success.sh",
        "#!/bin/sh\nprintf 'ready\\n'\nprintf 'ignored stderr\\n' >&2\n",
    )?;
    let output = SystemToolRunner.run(&success, &[], Duration::from_secs(2))?;
    assert_eq!(output.stdout, "ready\n");

    let failure = executable_script(
        directory.path(),
        "failure.sh",
        "#!/bin/sh\nprintf 'specific failure\\n' >&2\nexit 7\n",
    )?;
    let Err(error) = SystemToolRunner.run(&failure, &[], Duration::from_secs(2)) else {
        return Err("non-zero process unexpectedly succeeded".into());
    };
    assert!(error.contains("exited 7: specific failure"), "{error}");

    let overflow = executable_script(
        directory.path(),
        "overflow.sh",
        "#!/bin/sh\ni=0\nwhile [ \"$i\" -lt 70000 ]; do printf x; i=$((i + 1)); done\n",
    )?;
    let Err(error) = SystemToolRunner.run(&overflow, &[], Duration::from_secs(5)) else {
        return Err("oversized stdout unexpectedly succeeded".into());
    };
    assert!(error.contains("exceeded 64 KiB"), "{error}");

    let timeout = executable_script(directory.path(), "timeout.sh", "#!/bin/sh\nsleep 2\n")?;
    let Err(error) = SystemToolRunner.run(&timeout, &[], Duration::from_millis(20)) else {
        return Err("slow process unexpectedly completed".into());
    };
    assert!(error.contains("timed out"), "{error}");

    let missing = directory.path().join("missing-helper");
    let Err(error) = SystemToolRunner.run(&missing, &[], Duration::from_secs(1)) else {
        return Err("missing helper unexpectedly started".into());
    };
    assert!(error.contains("could not start verified helper"), "{error}");
    Ok(())
}
