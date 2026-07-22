use std::error::Error;
use std::ffi::OsString;
use std::time::Duration;

use compass_transcribe::AudioDownloader;
use compass_transcribe::audio::{AudioLimits, decode_audio};
use compass_transcribe::downloader::{
    HttpsToolFetcher, ManagedYtDlp, SystemToolRunner, ToolCache, ToolRunner, platform_tool_spec,
};

#[test]
#[ignore = "downloads and executes the pinned self-contained yt-dlp helper"]
fn verified_yt_dlp_helper_executes() -> Result<(), Box<dyn Error>> {
    let spec = platform_tool_spec().ok_or("current platform has no pinned yt-dlp artifact")?;
    let path = ToolCache::from_environment()?.ensure(spec, &HttpsToolFetcher::default())?;
    let output = SystemToolRunner
        .run(
            &path,
            &[
                OsString::from("--ignore-config"),
                OsString::from("--version"),
            ],
            Duration::from_secs(30),
        )
        .map_err(std::io::Error::other)?;
    assert_eq!(output.stdout.trim(), spec.version);
    Ok(())
}

#[test]
#[ignore = "downloads real public audio through the verified yt-dlp helper"]
fn verified_yt_dlp_downloads_decodable_audio() -> Result<(), Box<dyn Error>> {
    let url = std::env::var("COMPASS_YTDLP_ACCEPTANCE_URL")
        .map_err(|_| "set COMPASS_YTDLP_ACCEPTANCE_URL to a public audio or video URL")?;
    let directory = tempfile::tempdir()?;
    let mut downloader = ManagedYtDlp::from_environment()?;
    let path = downloader
        .download_audio(&url, directory.path())
        .map_err(std::io::Error::other)?;
    assert!(path.is_file());
    assert!(
        !decode_audio(&path, AudioLimits::default())?
            .samples
            .is_empty()
    );
    Ok(())
}
