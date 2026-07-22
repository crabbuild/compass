use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtractorKind {
    Generic,
    Markdown,
    JsonConfig,
    Terraform,
    PascalForm,
    LazarusPackage,
    DreamMaker,
    Solution,
    ProjectXml,
    Xaml,
    Template,
    PackageManifest,
    McpConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LanguageSpec {
    pub name: &'static str,
    pub grammar: Option<&'static str>,
    pub kind: ExtractorKind,
}

#[derive(Debug, Default)]
pub struct Registry;

impl Registry {
    #[must_use]
    pub fn resolve(path: &Path) -> Option<LanguageSpec> {
        let raw_name = path.file_name()?.to_str()?;
        let name = raw_name.to_ascii_lowercase();
        if matches!(
            raw_name,
            ".mcp.json" | "claude_desktop_config.json" | "mcp.json" | "mcp_servers.json"
        ) {
            return Some(LanguageSpec {
                name: "mcp-config",
                grammar: None,
                kind: ExtractorKind::McpConfig,
            });
        }
        if matches!(
            name.as_str(),
            "apm.yml" | "apm.yaml" | "pyproject.toml" | "go.mod" | "pom.xml"
        ) {
            return Some(LanguageSpec {
                name: "package-manifest",
                grammar: None,
                kind: ExtractorKind::PackageManifest,
            });
        }
        if name.ends_with(".blade.php") {
            return Some(spec("blade", "blade", ExtractorKind::Template));
        }
        let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
            return shebang_spec(path);
        };
        let extension_lower = extension.to_ascii_lowercase();
        let spec = match extension_lower.as_str() {
            "py" => spec("python", "python", ExtractorKind::Generic),
            "js" | "jsx" | "mjs" | "cjs" | "ejs" => {
                spec("javascript", "javascript", ExtractorKind::Generic)
            }
            "ts" | "mts" | "cts" | "ets" => {
                spec("typescript", "typescript", ExtractorKind::Generic)
            }
            "tsx" => spec("tsx", "tsx", ExtractorKind::Generic),
            "go" => spec("go", "go", ExtractorKind::Generic),
            "rs" => spec("rust", "rust", ExtractorKind::Generic),
            "java" => spec("java", "java", ExtractorKind::Generic),
            "groovy" | "gradle" => spec("groovy", "groovy", ExtractorKind::Generic),
            "c" => spec("c", "c", ExtractorKind::Generic),
            "h" => header_spec(path),
            "cpp" | "cc" | "cxx" | "hpp" | "cu" | "cuh" | "metal" => {
                spec("cpp", "cpp", ExtractorKind::Generic)
            }
            "rb" | "rake" => spec("ruby", "ruby", ExtractorKind::Generic),
            "cs" => spec("csharp", "csharp", ExtractorKind::Generic),
            "kt" | "kts" => spec("kotlin", "kotlin", ExtractorKind::Generic),
            "scala" => spec("scala", "scala", ExtractorKind::Generic),
            "php" => spec("php", "php", ExtractorKind::Generic),
            "swift" => spec("swift", "swift", ExtractorKind::Generic),
            "lua" | "luau" | "toc" => spec("lua", "lua", ExtractorKind::Generic),
            "zig" => spec("zig", "zig", ExtractorKind::Generic),
            "ps1" | "psm1" | "psd1" => spec("powershell", "powershell", ExtractorKind::Generic),
            "ex" | "exs" => spec("elixir", "elixir", ExtractorKind::Generic),
            "m" => objc_source_spec(path)?,
            "mm" => spec("objc", "objc", ExtractorKind::Generic),
            "jl" => spec("julia", "julia", ExtractorKind::Generic),
            "f" | "f90" | "f95" | "f03" | "f08" => {
                spec("fortran", "fortran", ExtractorKind::Generic)
            }
            "vue" => spec("vue", "vue", ExtractorKind::Template),
            "svelte" => spec("svelte", "svelte", ExtractorKind::Template),
            "astro" => spec("astro", "astro", ExtractorKind::Template),
            "dart" => spec("dart", "dart", ExtractorKind::Generic),
            "v" | "sv" | "svh" => spec("verilog", "verilog", ExtractorKind::Generic),
            "sql" => spec("sql", "sql", ExtractorKind::Generic),
            "md" | "mdx" | "qmd" | "skill" => LanguageSpec {
                name: "markdown",
                grammar: None,
                kind: ExtractorKind::Markdown,
            },
            "pas" | "pp" | "dpr" | "dpk" | "lpr" | "inc" => {
                spec("pascal", "pascal", ExtractorKind::Generic)
            }
            "dfm" | "lfm" => LanguageSpec {
                name: "pascal-form",
                grammar: None,
                kind: ExtractorKind::PascalForm,
            },
            "lpk" => LanguageSpec {
                name: "lazarus-package",
                grammar: None,
                kind: ExtractorKind::LazarusPackage,
            },
            "sh" | "bash" => spec("bash", "bash", ExtractorKind::Generic),
            "json" => spec("json", "json", ExtractorKind::JsonConfig),
            "tf" | "tfvars" | "hcl" => spec("terraform", "hcl", ExtractorKind::Terraform),
            "dm" | "dme" | "dmi" | "dmm" | "dmf" => LanguageSpec {
                name: "dreammaker",
                grammar: None,
                kind: ExtractorKind::DreamMaker,
            },
            "sln" | "slnx" => LanguageSpec {
                name: "solution",
                grammar: None,
                kind: ExtractorKind::Solution,
            },
            "csproj" | "fsproj" | "vbproj" => LanguageSpec {
                name: "project-xml",
                grammar: None,
                kind: ExtractorKind::ProjectXml,
            },
            "xaml" => LanguageSpec {
                name: "xaml",
                grammar: None,
                kind: ExtractorKind::Xaml,
            },
            "razor" | "cshtml" => spec("razor", "razor", ExtractorKind::Template),
            "cls" | "trigger" => spec("apex", "apex", ExtractorKind::Generic),
            _ => return None,
        };
        Some(spec)
    }

    #[must_use]
    pub fn supported_extensions() -> &'static [&'static str] {
        &[
            "py", "ts", "tsx", "js", "go", "rs", "java", "c", "cpp", "rb", "cs", "kt", "scala",
            "php", "swift", "lua", "zig", "ps1", "ex", "m", "jl", "f90", "vue", "svelte", "astro",
            "dart", "v", "sql", "md", "pas", "dfm", "sh", "json", "tf", "dm", "sln", "csproj",
            "xaml", "razor", "cls",
        ]
    }
}

fn shebang_spec(path: &Path) -> Option<LanguageSpec> {
    let source = std::fs::read(path).ok()?;
    let first_line = source.split(|byte| *byte == b'\n').next()?;
    let line = std::str::from_utf8(first_line)
        .ok()?
        .strip_prefix("#!")?
        .trim();
    let mut arguments = split_command_line(line)?;
    let first = arguments.first()?;
    let mut interpreter = Path::new(first).file_name()?.to_str()?.to_owned();
    if interpreter == "env" {
        arguments.remove(0);
        interpreter = env_interpreter(&arguments)?;
    }
    match interpreter.as_str() {
        "python" | "python2" | "python3" => Some(spec("python", "python", ExtractorKind::Generic)),
        "bash" | "sh" | "dash" | "zsh" | "ksh" => {
            Some(spec("bash", "bash", ExtractorKind::Generic))
        }
        "node" | "nodejs" => Some(spec("javascript", "javascript", ExtractorKind::Generic)),
        "ruby" => Some(spec("ruby", "ruby", ExtractorKind::Generic)),
        "lua" => Some(spec("lua", "lua", ExtractorKind::Generic)),
        "php" => Some(spec("php", "php", ExtractorKind::Generic)),
        "julia" => Some(spec("julia", "julia", ExtractorKind::Generic)),
        _ => None,
    }
}

fn env_interpreter(arguments: &[String]) -> Option<String> {
    let mut index = 0;
    while index < arguments.len() {
        let argument = &arguments[index];
        if argument == "-S" || argument == "--split-string" {
            let packed = arguments.get(index + 1)?;
            let split = split_command_line(packed)?;
            return split
                .first()
                .and_then(|value| Path::new(value).file_name())
                .and_then(|value| value.to_str())
                .map(str::to_owned);
        }
        if let Some(packed) = argument
            .strip_prefix("-S")
            .filter(|value| !value.is_empty())
            .or_else(|| argument.strip_prefix("--split-string="))
        {
            let split = split_command_line(packed)?;
            return split
                .first()
                .and_then(|value| Path::new(value).file_name())
                .and_then(|value| value.to_str())
                .map(str::to_owned);
        }
        if matches!(
            argument.as_str(),
            "-u" | "-C" | "-P" | "-a" | "--unset" | "--chdir"
        ) {
            index += 2;
            continue;
        }
        if argument.starts_with("--unset=")
            || argument.starts_with("--chdir=")
            || argument.starts_with("--argv0=")
            || (argument.starts_with("-u") && argument.len() > 2)
            || (argument.starts_with("-C") && argument.len() > 2)
            || (argument.starts_with("-P") && argument.len() > 2)
            || (argument.starts_with("-a") && argument.len() > 2)
            || matches!(
                argument.as_str(),
                "-i" | "-v" | "-0" | "--ignore-environment" | "--null" | "--debug"
            )
        {
            index += 1;
            continue;
        }
        if argument.contains('=') && !argument.starts_with('=') {
            index += 1;
            continue;
        }
        if argument.starts_with('-') {
            return None;
        }
        return Path::new(argument)
            .file_name()
            .and_then(|value| value.to_str())
            .map(str::to_owned);
    }
    None
}

fn split_command_line(line: &str) -> Option<Vec<String>> {
    let mut output = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;
    for character in line.chars() {
        if escaped {
            current.push(character);
            escaped = false;
            continue;
        }
        if character == '\\' && quote != Some('\'') {
            escaped = true;
            continue;
        }
        if let Some(active) = quote {
            if character == active {
                quote = None;
            } else {
                current.push(character);
            }
            continue;
        }
        if matches!(character, '\'' | '"') {
            quote = Some(character);
        } else if character.is_whitespace() {
            if !current.is_empty() {
                output.push(std::mem::take(&mut current));
            }
        } else {
            current.push(character);
        }
    }
    if escaped || quote.is_some() {
        return None;
    }
    if !current.is_empty() {
        output.push(current);
    }
    Some(output)
}

const fn spec(name: &'static str, grammar: &'static str, kind: ExtractorKind) -> LanguageSpec {
    LanguageSpec {
        name,
        grammar: Some(grammar),
        kind,
    }
}

fn header_spec(path: &Path) -> LanguageSpec {
    let source = std::fs::read(path).unwrap_or_default();
    if [
        b"@interface".as_slice(),
        b"@protocol",
        b"@implementation",
        b"@import",
        b"#import",
    ]
    .iter()
    .any(|marker| source.windows(marker.len()).any(|window| window == *marker))
    {
        spec("objc", "objc", ExtractorKind::Generic)
    } else if [
        b"class ".as_slice(),
        b"namespace ",
        b"template",
        b"::",
        b"public:",
        b"private:",
        b"protected:",
    ]
    .iter()
    .any(|marker| source.windows(marker.len()).any(|window| window == *marker))
    {
        spec("cpp", "cpp", ExtractorKind::Generic)
    } else {
        spec("c", "c", ExtractorKind::Generic)
    }
}

fn objc_source_spec(path: &Path) -> Option<LanguageSpec> {
    let source = std::fs::read(path).ok()?;
    [
        b"@interface".as_slice(),
        b"@protocol",
        b"@implementation",
        b"@import",
        b"#import",
    ]
    .iter()
    .any(|marker| source.windows(marker.len()).any(|window| window == *marker))
    .then(|| spec("objc", "objc", ExtractorKind::Generic))
}
