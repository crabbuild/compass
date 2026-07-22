//! Verified, self-contained URL audio acquisition through a pinned yt-dlp helper.

use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use sha2::{Digest, Sha256};
use url::Url;
use wait_timeout::ChildExt;

use crate::audio::DEFAULT_MAX_SOURCE_BYTES;
use crate::{AudioDownloader, audio_cache_key};

const YT_DLP_VERSION: &str = "2026.06.09";
const TOOL_USER_AGENT: &str = "compass/0.1 tool-fetch";
const DEFAULT_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const PROCESS_OUTPUT_LIMIT: u64 = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolSpec {
    pub version: &'static str,
    pub asset: &'static str,
    pub installed_name: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

impl ToolSpec {
    #[must_use]
    pub fn download_url(self) -> String {
        format!(
            "https://github.com/yt-dlp/yt-dlp/releases/download/{}/{}",
            self.version, self.asset
        )
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const PLATFORM_TOOL: Option<ToolSpec> = Some(ToolSpec {
    version: YT_DLP_VERSION,
    asset: "yt-dlp_linux",
    installed_name: "yt-dlp",
    size: 39_875_976,
    sha256: "bf8aac79b72287a6d2043074415132558b43743a8f9461a22b0141e90f16ce66",
});

#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
const PLATFORM_TOOL: Option<ToolSpec> = Some(ToolSpec {
    version: YT_DLP_VERSION,
    asset: "yt-dlp_linux_aarch64",
    installed_name: "yt-dlp",
    size: 39_628_336,
    sha256: "cabd246445bdfde0eda0dfe68bbe90354be83f3fdbbf077df11a2ea55f41cdbd",
});

#[cfg(all(
    target_os = "macos",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
const PLATFORM_TOOL: Option<ToolSpec> = Some(ToolSpec {
    version: YT_DLP_VERSION,
    asset: "yt-dlp_macos",
    installed_name: "yt-dlp",
    size: 36_478_448,
    sha256: "b82c3626952e6c14eaf654cc565866775ffd0b9ffb7021628ac59b42c2f4f244",
});

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
const PLATFORM_TOOL: Option<ToolSpec> = Some(ToolSpec {
    version: YT_DLP_VERSION,
    asset: "yt-dlp.exe",
    installed_name: "yt-dlp.exe",
    size: 18_202_192,
    sha256: "3a48cb955d55c8821b60ccbdbbc6f61bc958f2f3d3b7ad5eaf3d83a543293a27",
});

#[cfg(all(target_os = "windows", target_arch = "aarch64"))]
const PLATFORM_TOOL: Option<ToolSpec> = Some(ToolSpec {
    version: YT_DLP_VERSION,
    asset: "yt-dlp_arm64.exe",
    installed_name: "yt-dlp.exe",
    size: 22_204_855,
    sha256: "847583f91bb6d26479c1dc9643c2f4b8857a90b40d619da97b0cfabccb9138d0",
});

#[cfg(not(any(
    all(target_os = "linux", target_arch = "x86_64"),
    all(target_os = "linux", target_arch = "aarch64"),
    all(target_os = "macos", target_arch = "x86_64"),
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "windows", target_arch = "x86_64"),
    all(target_os = "windows", target_arch = "aarch64")
)))]
const PLATFORM_TOOL: Option<ToolSpec> = None;

#[derive(Debug, thiserror::Error)]
pub enum DownloaderError {
    #[error("yt-dlp is not packaged for this operating system and architecture")]
    UnsupportedPlatform,
    #[error("could not determine Compass's tool cache directory")]
    MissingCacheDirectory,
    #[error("URL rejected: {0}")]
    RejectedUrl(String),
    #[error("could not {action} {path}: {source}")]
    Io {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("tool download failed for {url}: {message}")]
    ToolDownload { url: String, message: String },
    #[error("tool artifact {path} has size {actual}; expected {expected}")]
    ToolSize {
        path: PathBuf,
        actual: u64,
        expected: u64,
    },
    #[error("tool artifact {0} failed SHA-256 verification")]
    ToolDigest(PathBuf),
    #[error("yt-dlp failed: {0}")]
    Process(String),
    #[error("yt-dlp did not produce a safe bounded audio file in {0}")]
    MissingOutput(PathBuf),
}

#[must_use]
pub fn platform_tool_spec() -> Option<ToolSpec> {
    PLATFORM_TOOL
}

pub trait ToolFetcher {
    fn fetch(&self, url: &str, max_bytes: u64) -> Result<Box<dyn Read>, String>;
}

#[derive(Clone)]
pub struct HttpsToolFetcher {
    agent: ureq::Agent,
}

impl Default for HttpsToolFetcher {
    fn default() -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(5 * 60)))
            .max_redirects(5)
            .build();
        Self {
            agent: config.into(),
        }
    }
}

impl ToolFetcher for HttpsToolFetcher {
    fn fetch(&self, url: &str, max_bytes: u64) -> Result<Box<dyn Read>, String> {
        let limit = max_bytes
            .checked_add(1)
            .ok_or_else(|| "tool size limit overflowed".to_owned())?;
        let response = self
            .agent
            .get(url)
            .header("User-Agent", TOOL_USER_AGENT)
            .call()
            .map_err(|error| error.to_string())?;
        Ok(Box::new(
            response
                .into_body()
                .into_with_config()
                .limit(limit)
                .reader(),
        ))
    }
}

#[derive(Debug, Clone)]
pub struct ToolCache {
    root: PathBuf,
}

impl ToolCache {
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn from_environment() -> Result<Self, DownloaderError> {
        if let Some(root) = std::env::var_os("COMPASS_CACHE_DIR") {
            return Ok(Self::new(PathBuf::from(root).join("tools")));
        }
        if let Some(root) = std::env::var_os("XDG_CACHE_HOME") {
            return Ok(Self::new(PathBuf::from(root).join("compass/tools")));
        }
        if cfg!(windows)
            && let Some(root) = std::env::var_os("LOCALAPPDATA")
        {
            return Ok(Self::new(PathBuf::from(root).join("compass/tools")));
        }
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|root| Self::new(root.join(".cache/compass/tools")))
            .ok_or(DownloaderError::MissingCacheDirectory)
    }

    pub fn ensure(
        &self,
        spec: ToolSpec,
        fetcher: &dyn ToolFetcher,
    ) -> Result<PathBuf, DownloaderError> {
        let directory = self.root.join("yt-dlp").join(spec.version);
        fs::create_dir_all(&directory).map_err(|source| DownloaderError::Io {
            action: "create tool cache directory",
            path: directory.clone(),
            source,
        })?;
        let destination = directory.join(spec.installed_name);
        if verify_tool(&destination, spec)? {
            make_executable(&destination)?;
            return Ok(destination);
        }

        let url = spec.download_url();
        let mut input =
            fetcher
                .fetch(&url, spec.size)
                .map_err(|message| DownloaderError::ToolDownload {
                    url: url.clone(),
                    message,
                })?;
        let mut temporary =
            tempfile::NamedTempFile::new_in(&directory).map_err(|source| DownloaderError::Io {
                action: "create temporary tool artifact",
                path: directory.clone(),
                source,
            })?;
        let mut hasher = Sha256::new();
        let mut total = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let count = input
                .read(&mut buffer)
                .map_err(|source| DownloaderError::Io {
                    action: "download tool artifact",
                    path: destination.clone(),
                    source,
                })?;
            if count == 0 {
                break;
            }
            total = total.saturating_add(count as u64);
            if total > spec.size {
                return Err(DownloaderError::ToolSize {
                    path: destination,
                    actual: total,
                    expected: spec.size,
                });
            }
            hasher.update(&buffer[..count]);
            temporary
                .write_all(&buffer[..count])
                .map_err(|source| DownloaderError::Io {
                    action: "write tool artifact",
                    path: destination.clone(),
                    source,
                })?;
        }
        if total != spec.size {
            return Err(DownloaderError::ToolSize {
                path: destination,
                actual: total,
                expected: spec.size,
            });
        }
        if format!("{:x}", hasher.finalize()) != spec.sha256 {
            return Err(DownloaderError::ToolDigest(destination));
        }
        temporary
            .as_file()
            .sync_all()
            .map_err(|source| DownloaderError::Io {
                action: "sync tool artifact",
                path: destination.clone(),
                source,
            })?;
        temporary
            .persist(&destination)
            .map_err(|error| DownloaderError::Io {
                action: "publish tool artifact",
                path: destination.clone(),
                source: error.error,
            })?;
        make_executable(&destination)?;
        Ok(destination)
    }
}

fn verify_tool(path: &Path, spec: ToolSpec) -> Result<bool, DownloaderError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(source) => {
            return Err(DownloaderError::Io {
                action: "inspect tool artifact",
                path: path.to_path_buf(),
                source,
            });
        }
    };
    if !metadata.file_type().is_file() || metadata.len() != spec.size {
        return Ok(false);
    }
    let mut file = File::open(path).map_err(|source| DownloaderError::Io {
        action: "verify tool artifact",
        path: path.to_path_buf(),
        source,
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|source| DownloaderError::Io {
                action: "verify tool artifact",
                path: path.to_path_buf(),
                source,
            })?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()) == spec.sha256)
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<(), DownloaderError> {
    use std::os::unix::fs::PermissionsExt as _;
    let mut permissions = fs::metadata(path)
        .map_err(|source| DownloaderError::Io {
            action: "inspect tool permissions",
            path: path.to_path_buf(),
            source,
        })?
        .permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(path, permissions).map_err(|source| DownloaderError::Io {
        action: "set tool permissions on",
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<(), DownloaderError> {
    Ok(())
}

trait HostResolver {
    fn resolve(&self, host: &str, port: u16) -> Result<Vec<IpAddr>, String>;
}

#[derive(Debug, Default)]
struct SystemResolver;

impl HostResolver for SystemResolver {
    fn resolve(&self, host: &str, port: u16) -> Result<Vec<IpAddr>, String> {
        (host, port)
            .to_socket_addrs()
            .map(|addresses| addresses.map(|address| address.ip()).collect())
            .map_err(|error| error.to_string())
    }
}

pub fn validate_public_url(input: &str) -> Result<Url, DownloaderError> {
    validate_public_url_with(input, &SystemResolver)
}

fn validate_public_url_with(
    input: &str,
    resolver: &dyn HostResolver,
) -> Result<Url, DownloaderError> {
    let url = Url::parse(input).map_err(|error| DownloaderError::RejectedUrl(error.to_string()))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(DownloaderError::RejectedUrl(format!(
            "blocked URL scheme {:?}; only http and https are allowed",
            url.scheme()
        )));
    }
    let host = url
        .host_str()
        .ok_or_else(|| DownloaderError::RejectedUrl("URL has no host".to_owned()))?;
    if matches!(
        host.to_ascii_lowercase().as_str(),
        "metadata.google.internal" | "metadata.google.com"
    ) {
        return Err(DownloaderError::RejectedUrl(format!(
            "blocked cloud metadata endpoint {host:?}"
        )));
    }
    let addresses = if let Ok(address) = host.parse::<IpAddr>() {
        vec![address]
    } else {
        resolver
            .resolve(host, url.port_or_known_default().unwrap_or(443))
            .map_err(|error| {
                DownloaderError::RejectedUrl(format!("DNS resolution failed for {host:?}: {error}"))
            })?
    };
    if addresses.is_empty() {
        return Err(DownloaderError::RejectedUrl(format!(
            "DNS resolution returned no addresses for {host:?}"
        )));
    }
    if let Some(address) = addresses
        .into_iter()
        .find(|address| ip_is_blocked(*address))
    {
        return Err(DownloaderError::RejectedUrl(format!(
            "blocked private or reserved address {address} for {host:?}"
        )));
    }
    Ok(url)
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
        || (a == 192 && b == 0 && c == 0)
        || (a == 192 && b == 0 && c == 2)
        || (a == 198 && matches!(b, 18 | 19))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113)
}

fn nat64_embedded_ipv4(address: Ipv6Addr) -> Option<Ipv4Addr> {
    let octets = address.octets();
    (octets[..12] == [0, 0x64, 0xff, 0x9b, 0, 0, 0, 0, 0, 0, 0, 0])
        .then(|| Ipv4Addr::new(octets[12], octets[13], octets[14], octets[15]))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessOutput {
    pub stdout: String,
}

pub trait ToolRunner {
    fn run(
        &mut self,
        program: &Path,
        arguments: &[OsString],
        timeout: Duration,
    ) -> Result<ProcessOutput, String>;
}

#[derive(Debug, Default)]
pub struct SystemToolRunner;

impl ToolRunner for SystemToolRunner {
    fn run(
        &mut self,
        program: &Path,
        arguments: &[OsString],
        timeout: Duration,
    ) -> Result<ProcessOutput, String> {
        let mut stdout = tempfile::tempfile().map_err(|error| error.to_string())?;
        let mut stderr = tempfile::tempfile().map_err(|error| error.to_string())?;
        let mut command = Command::new(program);
        command
            .args(arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::from(
                stdout.try_clone().map_err(|error| error.to_string())?,
            ))
            .stderr(Stdio::from(
                stderr.try_clone().map_err(|error| error.to_string())?,
            ))
            .env_remove("PYTHONHOME")
            .env_remove("PYTHONPATH")
            .env_remove("PYTHONSTARTUP");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt as _;
            command.creation_flags(0x0800_0000);
        }
        let mut child = command.spawn().map_err(|error| {
            format!(
                "could not start verified helper {}: {error}",
                program.display()
            )
        })?;
        let status = match child.wait_timeout(timeout) {
            Ok(Some(status)) => status,
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "verified helper timed out after {:.3} seconds",
                    timeout.as_secs_f64()
                ));
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("could not wait for verified helper: {error}"));
            }
        };
        let (stdout_bytes, stdout_overflowed) = read_capture(&mut stdout)?;
        let (stderr_bytes, _) = read_capture(&mut stderr)?;
        if stdout_overflowed {
            return Err("verified helper output exceeded 64 KiB".to_owned());
        }
        if !status.success() {
            let message = String::from_utf8_lossy(&stderr_bytes)
                .chars()
                .take(500)
                .collect::<String>();
            return Err(format!(
                "verified helper exited {}: {}",
                status
                    .code()
                    .map_or_else(|| "without a status".to_owned(), |code| code.to_string()),
                message.trim()
            ));
        }
        Ok(ProcessOutput {
            stdout: String::from_utf8_lossy(&stdout_bytes).into_owned(),
        })
    }
}

fn read_capture(file: &mut File) -> Result<(Vec<u8>, bool), String> {
    file.seek(SeekFrom::Start(0))
        .map_err(|error| error.to_string())?;
    let mut output = Vec::new();
    file.take(PROCESS_OUTPUT_LIMIT + 1)
        .read_to_end(&mut output)
        .map_err(|error| error.to_string())?;
    let overflowed = output.len() as u64 > PROCESS_OUTPUT_LIMIT;
    output.truncate(PROCESS_OUTPUT_LIMIT as usize);
    Ok((output, overflowed))
}

pub struct ManagedYtDlp<F = HttpsToolFetcher, R = SystemToolRunner> {
    cache: ToolCache,
    spec: ToolSpec,
    fetcher: F,
    runner: R,
    timeout: Duration,
    max_source_bytes: u64,
}

impl ManagedYtDlp {
    pub fn from_environment() -> Result<Self, DownloaderError> {
        Ok(Self {
            cache: ToolCache::from_environment()?,
            spec: platform_tool_spec().ok_or(DownloaderError::UnsupportedPlatform)?,
            fetcher: HttpsToolFetcher::default(),
            runner: SystemToolRunner,
            timeout: DEFAULT_DOWNLOAD_TIMEOUT,
            max_source_bytes: DEFAULT_MAX_SOURCE_BYTES,
        })
    }
}

impl<F, R> ManagedYtDlp<F, R> {
    #[must_use]
    pub fn new(cache: ToolCache, spec: ToolSpec, fetcher: F, runner: R) -> Self {
        Self {
            cache,
            spec,
            fetcher,
            runner,
            timeout: DEFAULT_DOWNLOAD_TIMEOUT,
            max_source_bytes: DEFAULT_MAX_SOURCE_BYTES,
        }
    }
}

impl<F: ToolFetcher, R: ToolRunner> AudioDownloader for ManagedYtDlp<F, R> {
    fn download_audio(&mut self, input: &str, output_dir: &Path) -> Result<PathBuf, String> {
        validate_public_url(input).map_err(|error| error.to_string())?;
        fs::create_dir_all(output_dir).map_err(|error| {
            format!(
                "could not create download directory {}: {error}",
                output_dir.display()
            )
        })?;
        if let Some(cached) = safe_cached_audio_path(input, output_dir, self.max_source_bytes) {
            return Ok(cached);
        }
        let program = self
            .cache
            .ensure(self.spec, &self.fetcher)
            .map_err(|error| error.to_string())?;
        let key = audio_cache_key(input);
        let template = output_dir.join(format!("yt_{key}.%(ext)s"));
        let arguments = yt_dlp_arguments(&template, self.max_source_bytes, input);
        let output = self
            .runner
            .run(&program, &arguments, self.timeout)
            .map_err(|error| DownloaderError::Process(error).to_string())?;
        resolve_downloaded_audio(&output.stdout, output_dir, &key, self.max_source_bytes)
            .map_err(|error| error.to_string())
    }
}

#[must_use]
pub fn yt_dlp_arguments(template: &Path, max_bytes: u64, url: &str) -> Vec<OsString> {
    [
        OsString::from("--ignore-config"),
        OsString::from("--no-plugin-dirs"),
        OsString::from("--quiet"),
        OsString::from("--no-warnings"),
        OsString::from("--no-playlist"),
        OsString::from("--format"),
        OsString::from("bestaudio[ext=m4a]/bestaudio/best"),
        OsString::from("--output"),
        template.as_os_str().to_owned(),
        OsString::from("--print"),
        OsString::from("after_move:filepath"),
        OsString::from("--max-filesize"),
        OsString::from(max_bytes.to_string()),
        OsString::from("--"),
        OsString::from(url),
    ]
    .into_iter()
    .collect()
}

fn safe_cached_audio_path(input: &str, output_dir: &Path, max_bytes: u64) -> Option<PathBuf> {
    let key = audio_cache_key(input);
    super::CACHED_AUDIO_EXTENSIONS
        .iter()
        .map(|extension| output_dir.join(format!("yt_{key}.{extension}")))
        .find(|candidate| safe_output_metadata(candidate, max_bytes).is_some())
}

fn resolve_downloaded_audio(
    stdout: &str,
    output_dir: &Path,
    key: &str,
    max_bytes: u64,
) -> Result<PathBuf, DownloaderError> {
    for line in stdout
        .lines()
        .rev()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let candidate = PathBuf::from(line);
        let candidate = if candidate.is_absolute() {
            candidate
        } else {
            output_dir.join(candidate)
        };
        if safe_named_output(&candidate, output_dir, key, max_bytes) {
            return Ok(candidate);
        }
    }
    let mut candidates = fs::read_dir(output_dir)
        .map_err(|source| DownloaderError::Io {
            action: "scan audio downloads in",
            path: output_dir.to_path_buf(),
            source,
        })?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| safe_named_output(path, output_dir, key, max_bytes))
        .collect::<Vec<_>>();
    candidates.sort();
    candidates
        .into_iter()
        .next()
        .ok_or_else(|| DownloaderError::MissingOutput(output_dir.to_path_buf()))
}

fn safe_named_output(path: &Path, output_dir: &Path, key: &str, max_bytes: u64) -> bool {
    let expected_prefix = format!("yt_{key}.");
    path.parent() == Some(output_dir)
        && path
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| name.starts_with(&expected_prefix))
        && safe_output_metadata(path, max_bytes).is_some()
}

fn safe_output_metadata(path: &Path, max_bytes: u64) -> Option<fs::Metadata> {
    let metadata = fs::symlink_metadata(path).ok()?;
    (metadata.file_type().is_file() && metadata.len() <= max_bytes).then_some(metadata)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::io::Cursor;

    const TEST_SPEC: ToolSpec = ToolSpec {
        version: "test",
        asset: "test-helper",
        installed_name: "yt-dlp-test",
        size: 11,
        sha256: "4654b19066a760b87db054d08c230e18f39519145a62ef35103a5e6f166d5871",
    };

    struct StaticResolver(Vec<IpAddr>);

    impl HostResolver for StaticResolver {
        fn resolve(&self, _host: &str, _port: u16) -> Result<Vec<IpAddr>, String> {
            Ok(self.0.clone())
        }
    }

    struct StaticFetcher {
        calls: Cell<usize>,
        bytes: &'static [u8],
    }

    impl ToolFetcher for StaticFetcher {
        fn fetch(&self, _url: &str, _max_bytes: u64) -> Result<Box<dyn Read>, String> {
            self.calls.set(self.calls.get() + 1);
            Ok(Box::new(Cursor::new(self.bytes)))
        }
    }

    #[derive(Default)]
    struct FakeRunner {
        calls: usize,
        create_output: bool,
        observed: Vec<OsString>,
    }

    impl ToolRunner for FakeRunner {
        fn run(
            &mut self,
            _program: &Path,
            arguments: &[OsString],
            _timeout: Duration,
        ) -> Result<ProcessOutput, String> {
            self.calls += 1;
            self.observed = arguments.to_vec();
            let template = arguments
                .windows(2)
                .find(|pair| pair[0] == OsStr::new("--output"))
                .map(|pair| pair[1].clone())
                .ok_or_else(|| "missing output template".to_owned())?;
            let path = PathBuf::from(template.to_string_lossy().replace("%(ext)s", "m4a"));
            if self.create_output {
                fs::write(&path, b"audio").map_err(|error| error.to_string())?;
            }
            Ok(ProcessOutput {
                stdout: format!("{}\n", path.display()),
            })
        }
    }

    #[test]
    fn release_catalog_covers_the_current_target() {
        let spec = platform_tool_spec();
        if matches!(
            (std::env::consts::OS, std::env::consts::ARCH),
            ("linux" | "macos" | "windows", "x86_64" | "aarch64")
        ) {
            assert!(spec.is_some());
        }
        if let Some(spec) = spec {
            assert_eq!(spec.version, YT_DLP_VERSION);
            assert_eq!(spec.sha256.len(), 64);
            assert!(spec.size > 10_000_000);
        }
    }

    #[test]
    fn url_policy_rejects_private_reserved_and_metadata_targets() {
        let public = StaticResolver(vec![
            "93.184.216.34"
                .parse()
                .unwrap_or(IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34))),
        ]);
        assert!(validate_public_url_with("https://example.com/watch", &public).is_ok());
        let private = StaticResolver(vec![IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))]);
        assert!(validate_public_url_with("https://example.com/watch", &private).is_err());
        assert!(validate_public_url("http://169.254.169.254/latest/meta-data").is_err());
        assert!(validate_public_url("file:///etc/passwd").is_err());
        assert!(validate_public_url("https://metadata.google.internal/path").is_err());
    }

    #[test]
    fn tool_cache_verifies_every_warm_use_and_replaces_corruption()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let cache = ToolCache::new(directory.path().to_path_buf());
        let fetcher = StaticFetcher {
            calls: Cell::new(0),
            bytes: b"test helper",
        };
        let path = cache.ensure(TEST_SPEC, &fetcher)?;
        assert_eq!(fs::read(&path)?, b"test helper");
        assert_eq!(fetcher.calls.get(), 1);
        assert_eq!(cache.ensure(TEST_SPEC, &fetcher)?, path);
        assert_eq!(fetcher.calls.get(), 1);
        fs::write(&path, b"bad helper")?;
        assert_eq!(cache.ensure(TEST_SPEC, &fetcher)?, path);
        assert_eq!(fetcher.calls.get(), 2);
        assert_eq!(fs::read(path)?, b"test helper");
        Ok(())
    }

    #[test]
    fn managed_downloader_uses_cache_before_running_helper()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let downloads = directory.path().join("downloads");
        fs::create_dir_all(&downloads)?;
        let input = "https://93.184.216.34/watch?v=42";
        let cached = downloads.join(format!("yt_{}.m4a", audio_cache_key(input)));
        fs::write(&cached, b"cached")?;
        let fetcher = StaticFetcher {
            calls: Cell::new(0),
            bytes: b"test helper",
        };
        let runner = FakeRunner::default();
        let mut downloader = ManagedYtDlp::new(
            ToolCache::new(directory.path().join("tools")),
            TEST_SPEC,
            fetcher,
            runner,
        );
        assert_eq!(downloader.download_audio(input, &downloads)?, cached);
        assert_eq!(downloader.runner.calls, 0);
        assert_eq!(downloader.fetcher.calls.get(), 0);
        Ok(())
    }

    #[test]
    fn managed_downloader_builds_guarded_compatible_command()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let downloads = directory.path().join("downloads");
        let input = "https://93.184.216.34/watch?v=43";
        let fetcher = StaticFetcher {
            calls: Cell::new(0),
            bytes: b"test helper",
        };
        let runner = FakeRunner {
            create_output: true,
            ..FakeRunner::default()
        };
        let mut downloader = ManagedYtDlp::new(
            ToolCache::new(directory.path().join("tools")),
            TEST_SPEC,
            fetcher,
            runner,
        );
        let result = downloader.download_audio(input, &downloads)?;
        assert_eq!(result.extension(), Some(OsStr::new("m4a")));
        assert!(
            downloader
                .runner
                .observed
                .contains(&OsString::from("--ignore-config"))
        );
        assert!(
            downloader
                .runner
                .observed
                .contains(&OsString::from("--no-plugin-dirs"))
        );
        assert!(
            downloader
                .runner
                .observed
                .contains(&OsString::from("--no-playlist"))
        );
        assert_eq!(
            downloader.runner.observed.last(),
            Some(&OsString::from(input))
        );
        Ok(())
    }

    #[test]
    fn ipv6_url_policy_rejects_embedded_private_and_reserved_networks() {
        let cases = [
            "::",
            "::1",
            "::ffff:127.0.0.1",
            "64:ff9b::a9fe:a9fe",
            "ff02::1",
            "fc00::1",
            "fe80::1",
            "2001:db8::1",
        ];
        for address in cases {
            let parsed = address
                .parse::<IpAddr>()
                .unwrap_or(IpAddr::V6(Ipv6Addr::UNSPECIFIED));
            assert!(ip_is_blocked(parsed), "{address} should be blocked");
        }
        let public = "2606:4700:4700::1111"
            .parse::<IpAddr>()
            .unwrap_or(IpAddr::V6(Ipv6Addr::UNSPECIFIED));
        assert!(!ip_is_blocked(public));
        assert_eq!(
            nat64_embedded_ipv4(
                "64:ff9b::c000:201"
                    .parse::<Ipv6Addr>()
                    .unwrap_or(Ipv6Addr::UNSPECIFIED)
            ),
            Some(Ipv4Addr::new(192, 0, 2, 1))
        );
        assert!(
            validate_public_url_with("https://example.com", &StaticResolver(Vec::new())).is_err()
        );
    }

    #[test]
    fn downloaded_audio_resolution_accepts_safe_stdout_then_sorted_fallback()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let key = "abc";
        let first = directory.path().join("yt_abc.wav");
        let second = directory.path().join("yt_abc.m4a");
        fs::write(&first, b"wav")?;
        fs::write(&second, b"m4a")?;

        assert_eq!(
            resolve_downloaded_audio("unsafe.txt\nyt_abc.wav\n", directory.path(), key, 10)?,
            first
        );
        assert_eq!(
            resolve_downloaded_audio("", directory.path(), key, 10)?,
            second
        );
        assert!(resolve_downloaded_audio("", directory.path(), "missing", 10).is_err());
        assert!(resolve_downloaded_audio("", &directory.path().join("absent"), key, 10).is_err());
        assert!(!safe_named_output(
            &directory.path().join("other.wav"),
            directory.path(),
            key,
            10
        ));
        assert!(!safe_named_output(&first, directory.path(), key, 2));
        Ok(())
    }

    #[test]
    fn environment_factories_and_https_overflow_guard_are_total() {
        assert!(ToolCache::from_environment().is_ok());
        assert!(ManagedYtDlp::from_environment().is_ok());
        let fetcher = HttpsToolFetcher::default();
        assert_eq!(
            fetcher.fetch("https://example.com", u64::MAX).err(),
            Some("tool size limit overflowed".to_owned())
        );
    }
}
