use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::{FileError, StatHashIndex, io_error};

const CORPUS_WARN_THRESHOLD: u64 = 50_000;
const CORPUS_UPPER_THRESHOLD: u64 = 500_000;
const FILE_COUNT_UPPER: usize = 500;

const CODE_EXTENSIONS: &[&str] = &[
    "py", "ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs", "ejs", "ets", "go", "rs", "java",
    "groovy", "gradle", "cpp", "cc", "cxx", "c", "h", "hpp", "cu", "cuh", "metal", "rb", "rake",
    "swift", "kt", "kts", "cs", "scala", "php", "lua", "luau", "toc", "zig", "ps1", "psm1", "psd1",
    "ex", "exs", "m", "mm", "jl", "vue", "svelte", "astro", "dart", "v", "sv", "svh", "sql", "r",
    "f", "f90", "f95", "f03", "f08", "pas", "pp", "dpr", "dpk", "lpr", "inc", "dfm", "lfm", "lpk",
    "sh", "bash", "json", "tf", "tfvars", "hcl", "dm", "dme", "dmi", "dmm", "dmf", "sln", "slnx",
    "csproj", "fsproj", "vbproj", "xaml", "razor", "cshtml", "cls", "trigger",
];
const DOCUMENT_EXTENSIONS: &[&str] = &[
    "md", "mdx", "qmd", "skill", "txt", "rst", "html", "yaml", "yml", "docx", "xlsx", "gdoc",
    "gsheet", "gslides",
];
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "svg"];
const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "mov", "webm", "mkv", "avi", "m4v", "mp3", "wav", "m4a", "ogg",
];
const SECRET_PRONE_EXTENSIONS: &[&str] = &[
    "json",
    "yaml",
    "yml",
    "toml",
    "ini",
    "cfg",
    "conf",
    "config",
    "xml",
    "properties",
    "env",
    "txt",
    "tfvars",
];
const SKIP_FILES: &[&str] = &[
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "Cargo.lock",
    "poetry.lock",
    "Gemfile.lock",
    "composer.lock",
    "go.sum",
    "go.work.sum",
];
const SKIP_DIRS: &[&str] = &[
    "venv",
    ".venv",
    "env",
    ".env",
    "node_modules",
    "__pycache__",
    ".git",
    "dist",
    "build",
    "target",
    "out",
    "site-packages",
    "lib64",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    ".tox",
    ".nox",
    ".eggs",
    "compass-out",
    "coverage",
    "lcov-report",
    "visual-tests",
    "visual-test",
    "__snapshots__",
    "storybook-static",
    "dist-protected",
    ".next",
    ".nuxt",
    ".turbo",
    ".angular",
    ".idea",
    ".cache",
    ".parcel-cache",
    ".svelte-kit",
    ".terraform",
    ".serverless",
    ".graphify",
    ".worktrees",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileType {
    Code,
    Document,
    Paper,
    Image,
    Video,
}

/// Controls whether mutable caller-local Git excludes participate in source discovery.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum IgnorePolicy {
    /// Current-checkout behavior, including `.git/info/exclude`.
    #[default]
    CurrentCheckout,
    /// Reproducible historical behavior using only committed ignore files and explicit excludes.
    HistoricalCommit,
}

impl FileType {
    const ALL: [Self; 5] = [
        Self::Code,
        Self::Document,
        Self::Paper,
        Self::Image,
        Self::Video,
    ];

    fn key(self) -> &'static str {
        match self {
            Self::Code => "code",
            Self::Document => "document",
            Self::Paper => "paper",
            Self::Image => "image",
            Self::Video => "video",
        }
    }
}

#[derive(Debug, Clone)]
pub struct DetectOptions {
    pub scan_filesystem: bool,
    pub follow_symlinks: bool,
    pub gitignore: bool,
    pub ignore_policy: IgnorePolicy,
    pub extra_excludes: Vec<String>,
    pub output_name: String,
    pub cache_root: Option<PathBuf>,
    pub google_workspace: bool,
    pub additional_files: Vec<PathBuf>,
}

impl Default for DetectOptions {
    fn default() -> Self {
        Self {
            scan_filesystem: true,
            follow_symlinks: false,
            gitignore: true,
            ignore_policy: IgnorePolicy::CurrentCheckout,
            extra_excludes: Vec::new(),
            output_name: std::env::var("COMPASS_OUT").unwrap_or_else(|_| "compass-out".to_owned()),
            cache_root: None,
            google_workspace: false,
            additional_files: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Detection {
    pub files: BTreeMap<String, Vec<String>>,
    pub total_files: usize,
    pub total_words: u64,
    pub needs_graph: bool,
    pub warning: Option<String>,
    pub skipped_sensitive: Vec<String>,
    pub unclassified: Vec<String>,
    pub walk_errors: Vec<String>,
    pub ignored: Vec<String>,
    pub graphifyignore_patterns: usize,
    pub scan_root: String,
    #[serde(skip)]
    pub google_workspace_shortcuts: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
struct IgnorePattern {
    anchor: PathBuf,
    negated: bool,
    directory_only: bool,
    path_relative: bool,
    matcher: Regex,
    component_matcher: Regex,
}

/// Immutable event filter for filesystem watchers.
///
/// Ignore files are intentionally loaded once at watcher startup. This keeps
/// high-volume editor and filesystem event streams cheap while ensuring the
/// watched corpus uses the same root ignore rules as a normal detection pass.
#[derive(Debug)]
pub struct WatchPathFilter {
    root: PathBuf,
    lexical_root: PathBuf,
    output_name: String,
    patterns: Vec<IgnorePattern>,
}

impl WatchPathFilter {
    pub fn new(root: &Path, options: &DetectOptions) -> Result<Self, FileError> {
        let lexical_root = if root.is_absolute() {
            root.to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(|source| io_error(root, source))?
                .join(root)
        };
        let root = fs::canonicalize(root).map_err(|source| io_error(root, source))?;
        let mut patterns = initial_ignore_patterns(&root, options.gitignore, options.ignore_policy);
        patterns.extend(options.extra_excludes.iter().filter_map(|raw| {
            parse_ignore_line(raw).and_then(|line| IgnorePattern::new(root.clone(), &line))
        }));
        Ok(Self {
            root,
            lexical_root,
            output_name: options.output_name.clone(),
            patterns,
        })
    }

    #[must_use]
    pub fn allows(&self, path: &Path) -> bool {
        let lexical = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.lexical_root.join(path)
        };
        let absolute = if lexical.starts_with(&self.root) {
            lexical
        } else if let Ok(relative) = lexical.strip_prefix(&self.lexical_root) {
            self.root.join(relative)
        } else {
            return false;
        };
        let Ok(relative) = absolute.strip_prefix(&self.root) else {
            return false;
        };
        if relative.as_os_str().is_empty()
            || relative.components().any(|component| {
                let value = component.as_os_str().to_string_lossy();
                value.starts_with('.')
                    || value == self.output_name
                    || Path::new(&self.output_name)
                        .file_name()
                        .is_some_and(|name| name == component.as_os_str())
            })
        {
            return false;
        }
        classify_file(&absolute).is_some() && !ignored(&absolute, &self.root, &self.patterns)
    }
}

impl IgnorePattern {
    fn new(anchor: PathBuf, line: &str) -> Option<Self> {
        let negated = line.starts_with('!');
        let raw = line.strip_prefix('!').unwrap_or(line);
        let directory_only = raw.ends_with('/');
        let path_relative = raw.trim_end_matches('/').contains('/');
        let raw = raw.trim_matches('/');
        if raw.is_empty() {
            return None;
        }
        Some(Self {
            anchor,
            negated,
            directory_only,
            path_relative,
            matcher: Regex::new(&glob_regex(raw, !path_relative)).ok()?,
            component_matcher: Regex::new(&glob_regex(raw, false)).ok()?,
        })
    }

    fn matches(&self, target: &Path) -> bool {
        let Ok(relative) = target.strip_prefix(&self.anchor) else {
            return false;
        };
        if relative.as_os_str().is_empty() || (self.directory_only && !target.is_dir()) {
            return false;
        }
        let relative = relative.to_string_lossy().replace('\\', "/");
        if self.path_relative {
            return self.matcher.is_match(&relative);
        }
        if self.matcher.is_match(&relative)
            || target
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| self.component_matcher.is_match(name))
        {
            return true;
        }
        let mut prefix = String::new();
        for component in relative.split('/') {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(component);
            if self.component_matcher.is_match(component) || self.matcher.is_match(&prefix) {
                return true;
            }
        }
        false
    }
}

fn glob_regex(pattern: &str, star_crosses_slash: bool) -> String {
    let mut output = String::from("^");
    let chars = pattern.chars().collect::<Vec<_>>();
    let mut index = 0;
    while index < chars.len() {
        match chars[index] {
            '*' if index + 1 < chars.len() && chars[index + 1] == '*' => {
                output.push_str(".*");
                index += 2;
            }
            '*' => {
                output.push_str(if star_crosses_slash { ".*" } else { "[^/]*" });
                index += 1;
            }
            '?' => {
                output.push_str(if star_crosses_slash { "." } else { "[^/]" });
                index += 1;
            }
            '[' => {
                let start = index;
                index += 1;
                while index < chars.len() && chars[index] != ']' {
                    index += 1;
                }
                if index < chars.len() {
                    output.extend(chars[start..=index].iter());
                    index += 1;
                } else {
                    output.push_str("\\[");
                }
            }
            value => {
                output.push_str(&regex::escape(&value.to_string()));
                index += 1;
            }
        }
    }
    output.push('$');
    output
}

fn parse_ignore_line(raw: &str) -> Option<String> {
    let mut line = raw.trim_start().trim_end_matches(['\r', '\n']).to_owned();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    if let Some(index) = line
        .char_indices()
        .find(|&(index, value)| {
            value == '#' && index > 0 && line[..index].ends_with(char::is_whitespace)
        })
        .map(|(index, _)| index)
    {
        line.truncate(index);
    }
    line = line.replace("\\#", "#");
    while line.ends_with(' ') && !line.ends_with("\\ ") {
        line.pop();
    }
    (!line.is_empty()).then_some(line)
}

fn load_own_ignore(directory: &Path, gitignore: bool) -> Vec<IgnorePattern> {
    let names: &[&str] = if gitignore {
        &[".gitignore", ".graphifyignore"]
    } else {
        &[".graphifyignore"]
    };
    names
        .iter()
        .flat_map(|name| {
            fs::read_to_string(directory.join(name))
                .unwrap_or_default()
                .lines()
                .filter_map(parse_ignore_line)
                .filter_map(|line| IgnorePattern::new(directory.to_path_buf(), &line))
                .collect::<Vec<_>>()
        })
        .collect()
}

fn vcs_root(root: &Path) -> Option<PathBuf> {
    let mut current = root.to_path_buf();
    loop {
        if [".git", ".hg", ".svn", "_darcs", ".fossil"]
            .iter()
            .any(|marker| current.join(marker).exists())
        {
            return Some(current);
        }
        let parent = current.parent()?;
        if parent == current {
            return None;
        }
        current = parent.to_path_buf();
    }
}

fn initial_ignore_patterns(
    root: &Path,
    gitignore: bool,
    policy: IgnorePolicy,
) -> Vec<IgnorePattern> {
    let ceiling = vcs_root(root).unwrap_or_else(|| root.to_path_buf());
    let mut directories = vec![root.to_path_buf()];
    while directories
        .last()
        .is_some_and(|directory| directory != &ceiling)
    {
        let Some(parent) = directories.last().and_then(|directory| directory.parent()) else {
            break;
        };
        directories.push(parent.to_path_buf());
    }
    directories.reverse();
    let mut patterns = Vec::new();
    if gitignore && policy == IgnorePolicy::CurrentCheckout {
        let exclude = ceiling.join(".git/info/exclude");
        if let Ok(text) = fs::read_to_string(exclude) {
            patterns.extend(
                text.lines()
                    .filter_map(parse_ignore_line)
                    .filter_map(|line| IgnorePattern::new(ceiling.clone(), &line)),
            );
        }
    }
    for directory in directories {
        patterns.extend(load_own_ignore(&directory, gitignore));
    }
    patterns
}

fn ignored(path: &Path, root: &Path, patterns: &[IgnorePattern]) -> bool {
    let evaluate = |target: &Path| {
        let mut result = false;
        for pattern in patterns {
            if pattern.matches(target) {
                result = !pattern.negated;
            }
        }
        result
    };
    if let Ok(relative) = path.strip_prefix(root) {
        let mut ancestor = root.to_path_buf();
        let components = relative.components().collect::<Vec<_>>();
        for component in components.iter().take(components.len().saturating_sub(1)) {
            ancestor.push(component);
            if evaluate(&ancestor) {
                return true;
            }
        }
    }
    evaluate(path)
}

fn is_noise_dir(path: &Path, output_name: &str) -> bool {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if SKIP_DIRS.contains(&name)
        || Path::new(output_name)
            .file_name()
            .is_some_and(|value| value == name)
        || name.ends_with("_venv")
        || name.ends_with("_env")
        || name.ends_with(".egg-info")
    {
        return true;
    }
    if name == "worktrees" {
        return path
            .parent()
            .and_then(Path::file_name)
            .and_then(|value| value.to_str())
            .is_some_and(|parent| parent.starts_with('.'));
    }
    if name == "snapshots" {
        let js_root = path
            .parent()
            .and_then(Path::file_name)
            .and_then(|value| value.to_str())
            .is_some_and(|parent| matches!(parent, "__tests__" | "__test__"));
        let has_snap = fs::read_dir(path).is_ok_and(|entries| {
            entries
                .flatten()
                .any(|entry| entry.path().extension().is_some_and(|ext| ext == "snap"))
        });
        return js_root || has_snap;
    }
    false
}

fn path_is_under(path: &Path, root: &Path) -> bool {
    fs::canonicalize(path)
        .and_then(|path| {
            path.strip_prefix(root).map(Path::to_path_buf).map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::PermissionDenied, "outside root")
            })
        })
        .is_ok()
}

fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn is_package_manifest(path: &Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|name| {
            matches!(
                name.to_ascii_lowercase().as_str(),
                "apm.yml" | "apm.yaml" | "pyproject.toml" | "go.mod" | "pom.xml"
            )
        })
}

fn shebang_is_code(path: &Path) -> bool {
    let Ok(bytes) = fs::read(path) else {
        return false;
    };
    let first = &bytes[..bytes.len().min(256)];
    if !first.starts_with(b"#!") {
        return false;
    }
    let decoded = String::from_utf8_lossy(first);
    let line = decoded
        .lines()
        .next()
        .unwrap_or_default()
        .trim_start_matches("#!")
        .trim();
    let mut words = line.split_whitespace().collect::<Vec<_>>();
    if words.is_empty() {
        return false;
    }
    let mut interpreter = Path::new(words[0])
        .file_name()
        .and_then(|value| value.to_str());
    if interpreter == Some("env") {
        words.remove(0);
        while words
            .first()
            .is_some_and(|word| word.starts_with('-') || word.contains('='))
        {
            let option = words.remove(0);
            if matches!(option, "-u" | "-C" | "-P" | "-a") && !words.is_empty() {
                words.remove(0);
            }
            if matches!(option, "-S" | "-vS" | "--split-string") {
                break;
            }
        }
        interpreter = words
            .first()
            .and_then(|word| Path::new(word).file_name().and_then(|value| value.to_str()));
    }
    interpreter.is_some_and(|value| {
        matches!(
            value,
            "python"
                | "python3"
                | "python2"
                | "ruby"
                | "perl"
                | "node"
                | "nodejs"
                | "bash"
                | "sh"
                | "dash"
                | "zsh"
                | "fish"
                | "ksh"
                | "tcsh"
                | "lua"
                | "php"
                | "julia"
                | "Rscript"
        )
    })
}

fn looks_like_paper(path: &Path) -> bool {
    static SIGNALS: OnceLock<Vec<Regex>> = OnceLock::new();
    let signals = SIGNALS.get_or_init(|| {
        [
            r"(?i)\barxiv\b",
            r"(?i)\bdoi\s*:",
            r"(?i)\babstract\b",
            r"(?i)\bproceedings\b",
            r"(?i)\bjournal\b",
            r"(?i)\bpreprint\b",
            r"\\cite\{",
            r"\[\d+\]",
            r"\[\n\d+\n\]",
            r"(?i)eq\.\s*\d+|equation\s+\d+",
            r"\d{4}\.\d{4,5}",
            r"(?i)\bwe propose\b",
            r"(?i)\bliterature\b",
        ]
        .into_iter()
        .filter_map(|pattern| Regex::new(pattern).ok())
        .collect()
    });
    let text = fs::read(path).unwrap_or_default();
    let text = String::from_utf8_lossy(&text[..text.len().min(3000)]);
    signals
        .iter()
        .filter(|signal| signal.is_match(&text))
        .count()
        >= 3
}

pub fn classify_file(path: &Path) -> Option<FileType> {
    if is_package_manifest(path)
        || path
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|name| name.to_ascii_lowercase().ends_with(".blade.php"))
    {
        return Some(FileType::Code);
    }
    let ext = extension(path);
    if ext.is_empty() {
        return shebang_is_code(path).then_some(FileType::Code);
    }
    if CODE_EXTENSIONS.contains(&ext.as_str()) {
        return Some(FileType::Code);
    }
    if ext == "pdf" {
        if path.components().any(|component| {
            let value = component.as_os_str().to_string_lossy().to_ascii_lowercase();
            [
                ".imageset",
                ".xcassets",
                ".appiconset",
                ".colorset",
                ".launchimage",
            ]
            .iter()
            .any(|marker| value.ends_with(marker))
        }) {
            return None;
        }
        return Some(FileType::Paper);
    }
    if IMAGE_EXTENSIONS.contains(&ext.as_str()) {
        return Some(FileType::Image);
    }
    if DOCUMENT_EXTENSIONS.contains(&ext.as_str()) {
        return Some(if looks_like_paper(path) {
            FileType::Paper
        } else {
            FileType::Document
        });
    }
    if VIDEO_EXTENSIONS.contains(&ext.as_str()) {
        return Some(FileType::Video);
    }
    None
}

fn graphable_source(path: &Path) -> bool {
    classify_file(path) == Some(FileType::Code)
        && !SECRET_PRONE_EXTENSIONS.contains(&extension(path).as_str())
}

fn generic_keyword_hit(name: &str) -> bool {
    static KEYWORDS: OnceLock<Regex> = OnceLock::new();
    let stem = name
        .trim_start_matches('.')
        .split('.')
        .next()
        .unwrap_or_default();
    let words = stem
        .split(['-', '_', ' '])
        .filter(|word| !word.is_empty())
        .count();
    let pattern = KEYWORDS.get_or_init(|| {
        Regex::new(r"(?i)(?:^|[^a-z0-9])(?:credentials?|secrets?|passwds?|passwords?|private_keys?|tokens?)(?:$|[^a-z])")
            .unwrap_or_else(|error| unreachable!("static regex failed: {error}"))
    });
    pattern.find_iter(stem).any(|hit| hit.end() == stem.len())
        || (words <= 2 && pattern.is_match(stem))
}

fn sensitive(path: &Path) -> bool {
    let parents = path
        .components()
        .take(path.components().count().saturating_sub(1));
    let mut ambiguous = false;
    for parent in parents {
        let value = parent.as_os_str().to_string_lossy();
        if matches!(value.as_ref(), ".ssh" | ".gnupg" | ".aws" | ".gcloud") {
            return true;
        }
        if matches!(value.as_ref(), "secrets" | ".secrets" | "credentials") {
            ambiguous = true;
        }
    }
    if ambiguous && !graphable_source(path) {
        return true;
    }
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    let lower = name.to_ascii_lowercase();
    if lower == ".netrc"
        || lower == ".pgpass"
        || lower == ".htpasswd"
        || lower.starts_with(".env")
        || lower.starts_with(".envrc")
        || ["pem", "key", "p12", "pfx", "cert", "crt", "der", "p8"]
            .contains(&extension(path).as_str())
        || ["id_rsa", "id_dsa", "id_ecdsa", "id_ed25519"]
            .iter()
            .any(|candidate| lower == *candidate || lower == format!("{candidate}.pub"))
        || ["aws_credentials", "gcloud_credentials", "service.account"]
            .iter()
            .any(|candidate| lower.contains(candidate))
    {
        return true;
    }
    generic_keyword_hit(name) && !graphable_source(path)
}

fn count_words(path: &Path) -> u64 {
    if matches!(extension(path).as_str(), "pdf" | "docx" | "xlsx") {
        return 0;
    }
    fs::read(path).map_or(0, |bytes| {
        String::from_utf8_lossy(&bytes).split_whitespace().count() as u64
    })
}

struct WalkState<'a> {
    root: &'a Path,
    options: &'a DetectOptions,
    patterns: Vec<IgnorePattern>,
    all_files: Vec<PathBuf>,
    ignored: Vec<String>,
    walk_errors: Vec<String>,
    skipped_sensitive: Vec<String>,
}

impl WalkState<'_> {
    fn walk(&mut self, directory: &Path, ancestors: &mut Vec<PathBuf>) {
        if directory != self.root {
            self.patterns
                .extend(load_own_ignore(directory, self.options.gitignore));
        }
        let entries = match fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(error) => {
                self.walk_errors
                    .push(format!("{}: {error}", directory.display()));
                return;
            }
        };
        let mut entries = entries.flatten().collect::<Vec<_>>();
        entries.sort_by_key(fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let is_directory = file_type.is_dir() || (file_type.is_symlink() && path.is_dir());
            if is_directory {
                if is_noise_dir(&path, &self.options.output_name) {
                    continue;
                }
                if ignored(&path, self.root, &self.patterns) {
                    self.ignored.push(format!(
                        "{}{sep}",
                        path.display(),
                        sep = std::path::MAIN_SEPARATOR
                    ));
                    continue;
                }
                if file_type.is_symlink() && !self.options.follow_symlinks {
                    continue;
                }
                let Ok(canonical) = fs::canonicalize(&path) else {
                    continue;
                };
                if !canonical.starts_with(self.root) {
                    self.skipped_sensitive.push(format!(
                        "{} [symlink target outside scan root]",
                        path.display()
                    ));
                    continue;
                }
                if ancestors.contains(&canonical) {
                    continue;
                }
                ancestors.push(canonical);
                self.walk(&path, ancestors);
                ancestors.pop();
            } else if !SKIP_FILES.contains(
                &path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default(),
            ) {
                self.all_files.push(path);
            }
        }
    }
}

pub fn detect(root: &Path, options: &DetectOptions) -> Result<Detection, FileError> {
    let root = fs::canonicalize(root).map_err(|source| io_error(root, source))?;
    let mut patterns = Vec::new();
    if options.scan_filesystem {
        patterns = initial_ignore_patterns(&root, options.gitignore, options.ignore_policy);
        patterns.extend(options.extra_excludes.iter().filter_map(|raw| {
            parse_ignore_line(raw).and_then(|line| IgnorePattern::new(root.clone(), &line))
        }));
    }
    let mut state = WalkState {
        root: &root,
        options,
        patterns,
        all_files: Vec::new(),
        ignored: Vec::new(),
        walk_errors: Vec::new(),
        skipped_sensitive: Vec::new(),
    };
    let memory = root.join(&options.output_name).join("memory");
    if options.scan_filesystem {
        let mut ancestors = vec![root.clone()];
        state.walk(&root, &mut ancestors);
        if memory.is_dir() {
            collect_memory_files(&memory, &mut state.all_files, &mut state.walk_errors);
        }
    }
    state.all_files.sort();
    state.all_files.dedup();

    let mut files = FileType::ALL
        .into_iter()
        .map(|kind| (kind.key().to_owned(), Vec::new()))
        .collect::<BTreeMap<_, _>>();
    let mut unclassified = Vec::new();
    let mut google_workspace_shortcuts = Vec::new();
    let mut total_words = 0;
    let cache_root = options.cache_root.as_deref().unwrap_or(&root);
    let mut stat_index = StatHashIndex::load(cache_root, &options.output_name);
    for path in state.all_files {
        let in_memory = path.starts_with(&memory);
        if !in_memory && ignored(&path, &root, &state.patterns) {
            state.ignored.push(path.to_string_lossy().into_owned());
            continue;
        }
        if !path_is_under(&path, &root) {
            state.skipped_sensitive.push(format!(
                "{} [symlink target outside scan root]",
                path.display()
            ));
            continue;
        }
        if sensitive(&path) {
            state
                .skipped_sensitive
                .push(path.to_string_lossy().into_owned());
            continue;
        }
        let Some(file_type) = classify_file(&path) else {
            unclassified.push(path.to_string_lossy().into_owned());
            continue;
        };
        if is_google_workspace_shortcut(&path) {
            google_workspace_shortcuts.push(path.clone());
            if !options.google_workspace {
                state.skipped_sensitive.push(format!(
                    "{} [Google Workspace shortcut skipped - pass --google-workspace or set GRAPHIFY_GOOGLE_WORKSPACE=1]",
                    path.display()
                ));
            }
            continue;
        }
        if file_type != FileType::Video {
            total_words += stat_index.word_count(&path, count_words);
        }
        if let Some(bucket) = files.get_mut(file_type.key()) {
            bucket.push(path.to_string_lossy().into_owned());
        }
    }
    for path in &options.additional_files {
        if !path_is_under(path, &root)
            || has_noise_ancestor(path, &root, &options.output_name)
            || ignored(path, &root, &state.patterns)
            || sensitive(path)
        {
            continue;
        }
        let Some(file_type) = classify_file(path) else {
            continue;
        };
        if file_type != FileType::Video {
            total_words += stat_index.word_count(path, count_words);
        }
        if let Some(bucket) = files.get_mut(file_type.key()) {
            let value = path.to_string_lossy().into_owned();
            if !bucket.contains(&value) {
                bucket.push(value);
            }
        }
    }
    for bucket in files.values_mut() {
        bucket.sort();
    }
    let total_files = files.values().map(Vec::len).sum();
    let needs_graph = total_words >= CORPUS_WARN_THRESHOLD;
    let word_count = grouped_number(total_words);
    let warning = if !needs_graph {
        Some(format!(
            "Corpus is ~{word_count} words - fits in a single context window. You may not need a graph."
        ))
    } else if total_words >= CORPUS_UPPER_THRESHOLD || total_files >= FILE_COUNT_UPPER {
        Some(format!(
            "Large corpus: {total_files} files · ~{word_count} words. Semantic extraction will be expensive (many Claude tokens). Consider running on a subfolder."
        ))
    } else {
        None
    };
    unclassified.sort();
    state.ignored.sort();
    let graphifyignore_patterns = state.patterns.len();
    stat_index.flush()?;
    Ok(Detection {
        files,
        total_files,
        total_words,
        needs_graph,
        warning,
        skipped_sensitive: state.skipped_sensitive,
        unclassified,
        walk_errors: state.walk_errors,
        ignored: state.ignored,
        graphifyignore_patterns,
        scan_root: root.to_string_lossy().into_owned(),
        google_workspace_shortcuts,
    })
}

fn is_google_workspace_shortcut(path: &Path) -> bool {
    matches!(extension(path).as_str(), "gdoc" | "gsheet" | "gslides")
}

fn has_noise_ancestor(path: &Path, root: &Path, output_name: &str) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return true;
    };
    let mut ancestor = root.to_path_buf();
    let components = relative.components().collect::<Vec<_>>();
    components
        .iter()
        .take(components.len().saturating_sub(1))
        .any(|component| {
            ancestor.push(component);
            is_noise_dir(&ancestor, output_name)
        })
}

fn collect_memory_files(directory: &Path, files: &mut Vec<PathBuf>, errors: &mut Vec<String>) {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => {
            errors.push(format!("{}: {error}", directory.display()));
            return;
        }
    };
    let mut entries = entries.flatten().collect::<Vec<_>>();
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_memory_files(&path, files, errors);
        } else {
            files.push(path);
        }
    }
}

fn grouped_number(value: u64) -> String {
    let digits = value.to_string();
    let mut output = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            output.push(',');
        }
        output.push(digit);
    }
    output
}
