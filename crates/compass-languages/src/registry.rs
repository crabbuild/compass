use std::path::Path;

use crate::{AdapterProfile, AdapterRegistry};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtractorKind {
    Generic,
    Markdown,
    Html,
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
    FrameworkConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LanguageSpec {
    pub name: &'static str,
    pub grammar: Option<&'static str>,
    pub kind: ExtractorKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistryMatcher {
    ExactNames(&'static [&'static str]),
    LowerNames(&'static [&'static str]),
    Suffixes(&'static [&'static str]),
    ParentFile {
        file_name: &'static str,
        parent: &'static str,
    },
    Extensions(&'static [&'static str]),
    Header(&'static [&'static [u8]], bool),
    ObjectiveCSource,
    Include(bool),
    Shebang(&'static [&'static str]),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegistryCase {
    pub id: &'static str,
    pub matcher: RegistryMatcher,
    pub spec: LanguageSpec,
    pub fixture_path: &'static str,
    pub fixture_source: &'static str,
}

#[derive(Debug, Default)]
pub struct Registry;

impl Registry {
    #[must_use]
    pub fn resolve(path: &Path) -> Option<LanguageSpec> {
        REGISTRY_CASES
            .iter()
            .find(|case| matcher_matches(case.matcher, path))
            .map(|case| case.spec)
    }

    #[must_use]
    pub fn cases() -> &'static [RegistryCase] {
        REGISTRY_CASES
    }

    /// Resolve a path only when its language has hard-cut to universal evidence.
    #[must_use]
    pub fn universal_adapter(path: &Path) -> Option<&'static AdapterProfile> {
        Self::resolve(path).and_then(Self::universal_profile_for_spec)
    }

    /// Return the hard-cut adapter associated with a resolved language.
    ///
    /// TSX is a parser dialect of the canonical TypeScript universal adapter;
    /// keep the registry's `tsx` grammar name while publishing one stable
    /// semantic language identity for resolution and cache invalidation.
    #[must_use]
    pub fn universal_profile_for_spec(spec: LanguageSpec) -> Option<&'static AdapterProfile> {
        let language = match spec.name {
            "tsx" => "typescript",
            language => language,
        };
        AdapterRegistry::universal_profile(language)
    }

    #[must_use]
    pub fn supported_extensions() -> Vec<&'static str> {
        let mut extensions = REGISTRY_CASES
            .iter()
            .flat_map(|case| match case.matcher {
                RegistryMatcher::Extensions(values) => values.iter().copied(),
                RegistryMatcher::Header(_, _) => ["h"].iter().copied(),
                RegistryMatcher::ObjectiveCSource => ["m"].iter().copied(),
                RegistryMatcher::Include(_) => ["inc"].iter().copied(),
                _ => [].iter().copied(),
            })
            .collect::<Vec<_>>();
        extensions.sort_unstable();
        extensions.dedup();
        extensions
    }
}

const OBJC_MARKERS: &[&[u8]] = &[
    b"@interface",
    b"@protocol",
    b"@implementation",
    b"@import",
    b"#import",
];
const CPP_HEADER_MARKERS: &[&[u8]] = &[
    b"class ",
    b"namespace ",
    b"template",
    b"::",
    b"public:",
    b"private:",
    b"protected:",
];

macro_rules! registry_case {
    ($id:literal, $matcher:expr, $spec:expr, $path:literal, $source:literal) => {
        RegistryCase {
            id: $id,
            matcher: $matcher,
            spec: $spec,
            fixture_path: $path,
            fixture_source: $source,
        }
    };
}

const REGISTRY_CASES: &[RegistryCase] = &[
    registry_case!(
        "mcp-config",
        RegistryMatcher::ExactNames(&[
            ".mcp.json",
            "claude_desktop_config.json",
            "mcp.json",
            "mcp_servers.json"
        ]),
        LanguageSpec {
            name: "mcp-config",
            grammar: None,
            kind: ExtractorKind::McpConfig
        },
        "matrix/mcp.json",
        "{\"mcpServers\":{}}\n"
    ),
    registry_case!(
        "package-manifest",
        RegistryMatcher::LowerNames(&[
            "apm.yml",
            "apm.yaml",
            "pyproject.toml",
            "go.mod",
            "pom.xml"
        ]),
        LanguageSpec {
            name: "package-manifest",
            grammar: None,
            kind: ExtractorKind::PackageManifest
        },
        "matrix/pyproject.toml",
        "[project]\nname = \"matrix\"\n"
    ),
    registry_case!(
        "drupal-routing",
        RegistryMatcher::Suffixes(&[".routing.yml", ".routing.yaml"]),
        LanguageSpec {
            name: "drupal-routing",
            grammar: None,
            kind: ExtractorKind::FrameworkConfig
        },
        "matrix/example.routing.yml",
        "example.route:\n  path: /matrix\n  defaults:\n    _controller: 'Example::view'\n"
    ),
    registry_case!(
        "play-routes",
        RegistryMatcher::ParentFile {
            file_name: "routes",
            parent: "conf"
        },
        LanguageSpec {
            name: "play-routes",
            grammar: None,
            kind: ExtractorKind::FrameworkConfig
        },
        "matrix/conf/routes",
        "GET /matrix controllers.Example.view()\n"
    ),
    registry_case!(
        "drupal-php",
        RegistryMatcher::Extensions(&["module", "theme", "install"]),
        spec("php", "php", ExtractorKind::Generic),
        "matrix/example.module",
        "<?php function example_hook() {}\n"
    ),
    registry_case!(
        "blade",
        RegistryMatcher::Suffixes(&[".blade.php"]),
        spec("blade", "blade", ExtractorKind::Template),
        "matrix/example.blade.php",
        "@include('shared.header')\n"
    ),
    registry_case!(
        "header-objc",
        RegistryMatcher::Header(OBJC_MARKERS, false),
        spec("objc", "objc", ExtractorKind::Generic),
        "matrix/objc.h",
        "@interface Matrix\n@end\n"
    ),
    registry_case!(
        "header-cpp",
        RegistryMatcher::Header(CPP_HEADER_MARKERS, false),
        spec("cpp", "cpp", ExtractorKind::Generic),
        "matrix/cpp.h",
        "class Matrix {};\n"
    ),
    registry_case!(
        "header-c",
        RegistryMatcher::Header(&[], true),
        spec("c", "c", ExtractorKind::Generic),
        "matrix/c.h",
        "int matrix(void);\n"
    ),
    registry_case!(
        "objc-source",
        RegistryMatcher::ObjectiveCSource,
        spec("objc", "objc", ExtractorKind::Generic),
        "matrix/example.m",
        "@implementation Matrix\n@end\n"
    ),
    registry_case!(
        "include-php",
        RegistryMatcher::Include(true),
        spec("php", "php", ExtractorKind::Generic),
        "matrix/php.inc",
        "<?php function matrix() {}\n"
    ),
    registry_case!(
        "include-pascal",
        RegistryMatcher::Include(false),
        spec("pascal", "pascal", ExtractorKind::Generic),
        "matrix/pascal.inc",
        "procedure Matrix;\nbegin\nend;\n"
    ),
    registry_case!(
        "python",
        RegistryMatcher::Extensions(&["py"]),
        spec("python", "python", ExtractorKind::Generic),
        "matrix/sample.py",
        "def matrix():\n    return 1\n"
    ),
    registry_case!(
        "javascript",
        RegistryMatcher::Extensions(&["js", "jsx", "mjs", "cjs", "ejs"]),
        spec("javascript", "javascript", ExtractorKind::Generic),
        "matrix/sample.js",
        "export function matrix() {}\n"
    ),
    registry_case!(
        "typescript",
        RegistryMatcher::Extensions(&["ts", "mts", "cts", "ets"]),
        spec("typescript", "typescript", ExtractorKind::Generic),
        "matrix/sample.ts",
        "export function matrix(): void {}\n"
    ),
    registry_case!(
        "tsx",
        RegistryMatcher::Extensions(&["tsx"]),
        spec("tsx", "tsx", ExtractorKind::Generic),
        "matrix/sample.tsx",
        "export const Matrix = () => <div />;\n"
    ),
    registry_case!(
        "go",
        RegistryMatcher::Extensions(&["go"]),
        spec("go", "go", ExtractorKind::Generic),
        "matrix/sample.go",
        "package matrix\nfunc Example() {}\n"
    ),
    registry_case!(
        "rust",
        RegistryMatcher::Extensions(&["rs"]),
        spec("rust", "rust", ExtractorKind::Generic),
        "matrix/sample.rs",
        "pub fn matrix() {}\n"
    ),
    registry_case!(
        "java",
        RegistryMatcher::Extensions(&["java"]),
        spec("java", "java", ExtractorKind::Generic),
        "matrix/Sample.java",
        "class Sample {}\n"
    ),
    registry_case!(
        "groovy",
        RegistryMatcher::Extensions(&["groovy", "gradle"]),
        spec("groovy", "groovy", ExtractorKind::Generic),
        "matrix/sample.groovy",
        "class Sample {}\n"
    ),
    registry_case!(
        "c",
        RegistryMatcher::Extensions(&["c"]),
        spec("c", "c", ExtractorKind::Generic),
        "matrix/sample.c",
        "int matrix(void) { return 1; }\n"
    ),
    registry_case!(
        "cpp",
        RegistryMatcher::Extensions(&["cpp", "cc", "cxx", "hpp", "cu", "cuh", "metal"]),
        spec("cpp", "cpp", ExtractorKind::Generic),
        "matrix/sample.cpp",
        "class Matrix {};\n"
    ),
    registry_case!(
        "ruby",
        RegistryMatcher::Extensions(&["rb", "rake"]),
        spec("ruby", "ruby", ExtractorKind::Generic),
        "matrix/sample.rb",
        "def matrix\nend\n"
    ),
    registry_case!(
        "csharp",
        RegistryMatcher::Extensions(&["cs"]),
        spec("csharp", "csharp", ExtractorKind::Generic),
        "matrix/Sample.cs",
        "class Sample {}\n"
    ),
    registry_case!(
        "kotlin",
        RegistryMatcher::Extensions(&["kt", "kts"]),
        spec("kotlin", "kotlin", ExtractorKind::Generic),
        "matrix/sample.kt",
        "class Sample\n"
    ),
    registry_case!(
        "scala",
        RegistryMatcher::Extensions(&["scala"]),
        spec("scala", "scala", ExtractorKind::Generic),
        "matrix/Sample.scala",
        "class Sample\n"
    ),
    registry_case!(
        "php",
        RegistryMatcher::Extensions(&["php"]),
        spec("php", "php", ExtractorKind::Generic),
        "matrix/sample.php",
        "<?php function matrix() {}\n"
    ),
    registry_case!(
        "perl",
        RegistryMatcher::Extensions(&["pl", "pm"]),
        spec("perl", "perl", ExtractorKind::Generic),
        "matrix/sample.pl",
        "sub matrix { return 1; }\n"
    ),
    registry_case!(
        "swift",
        RegistryMatcher::Extensions(&["swift"]),
        spec("swift", "swift", ExtractorKind::Generic),
        "matrix/sample.swift",
        "func matrix() {}\n"
    ),
    registry_case!(
        "lua",
        RegistryMatcher::Extensions(&["lua", "luau", "toc"]),
        spec("lua", "lua", ExtractorKind::Generic),
        "matrix/sample.lua",
        "function matrix() end\n"
    ),
    registry_case!(
        "zig",
        RegistryMatcher::Extensions(&["zig"]),
        spec("zig", "zig", ExtractorKind::Generic),
        "matrix/sample.zig",
        "pub fn matrix() void {}\n"
    ),
    registry_case!(
        "powershell",
        RegistryMatcher::Extensions(&["ps1", "psm1", "psd1"]),
        spec("powershell", "powershell", ExtractorKind::Generic),
        "matrix/sample.ps1",
        "function Matrix {}\n"
    ),
    registry_case!(
        "elixir",
        RegistryMatcher::Extensions(&["ex", "exs"]),
        spec("elixir", "elixir", ExtractorKind::Generic),
        "matrix/sample.ex",
        "defmodule Matrix do\nend\n"
    ),
    registry_case!(
        "objc",
        RegistryMatcher::Extensions(&["mm"]),
        spec("objc", "objc", ExtractorKind::Generic),
        "matrix/sample.mm",
        "@implementation Matrix\n@end\n"
    ),
    registry_case!(
        "julia",
        RegistryMatcher::Extensions(&["jl"]),
        spec("julia", "julia", ExtractorKind::Generic),
        "matrix/sample.jl",
        "function matrix()\nend\n"
    ),
    registry_case!(
        "fortran",
        RegistryMatcher::Extensions(&["f", "f90", "f95", "f03", "f08"]),
        spec("fortran", "fortran", ExtractorKind::Generic),
        "matrix/sample.f90",
        "subroutine matrix()\nend subroutine\n"
    ),
    registry_case!(
        "vue",
        RegistryMatcher::Extensions(&["vue"]),
        spec("vue", "vue", ExtractorKind::Template),
        "matrix/Sample.vue",
        "<template><div /></template>\n"
    ),
    registry_case!(
        "svelte",
        RegistryMatcher::Extensions(&["svelte"]),
        spec("svelte", "svelte", ExtractorKind::Template),
        "matrix/Sample.svelte",
        "<div>matrix</div>\n"
    ),
    registry_case!(
        "astro",
        RegistryMatcher::Extensions(&["astro"]),
        spec("astro", "astro", ExtractorKind::Template),
        "matrix/sample.astro",
        "---\nconst value = 1;\n---\n<div>{value}</div>\n"
    ),
    registry_case!(
        "dart",
        RegistryMatcher::Extensions(&["dart"]),
        spec("dart", "dart", ExtractorKind::Generic),
        "matrix/sample.dart",
        "void matrix() {}\n"
    ),
    registry_case!(
        "verilog",
        RegistryMatcher::Extensions(&["v", "sv", "svh"]),
        spec("verilog", "verilog", ExtractorKind::Generic),
        "matrix/sample.sv",
        "module matrix; endmodule\n"
    ),
    registry_case!(
        "sql",
        RegistryMatcher::Extensions(&["sql"]),
        spec("sql", "sql", ExtractorKind::Generic),
        "matrix/sample.sql",
        "CREATE TABLE matrix (id INTEGER);\n"
    ),
    registry_case!(
        "r",
        RegistryMatcher::Extensions(&["r"]),
        LanguageSpec {
            name: "r",
            grammar: None,
            kind: ExtractorKind::Generic
        },
        "matrix/sample.r",
        "matrix_fn <- function() 1\n"
    ),
    registry_case!(
        "markdown",
        RegistryMatcher::Extensions(&["md", "markdown", "mdx", "qmd", "skill"]),
        LanguageSpec {
            name: "markdown",
            grammar: None,
            kind: ExtractorKind::Markdown
        },
        "matrix/sample.md",
        "# Matrix\n"
    ),
    registry_case!(
        "html",
        RegistryMatcher::Extensions(&["html", "htm"]),
        LanguageSpec {
            name: "html",
            grammar: Some("html"),
            kind: ExtractorKind::Html
        },
        "matrix/sample.html",
        "<main><h1>Matrix</h1></main>\n"
    ),
    registry_case!(
        "pascal",
        RegistryMatcher::Extensions(&["pas", "pp", "dpr", "dpk", "lpr"]),
        spec("pascal", "pascal", ExtractorKind::Generic),
        "matrix/sample.pas",
        "program Matrix;\nbegin\nend.\n"
    ),
    registry_case!(
        "pascal-form",
        RegistryMatcher::Extensions(&["dfm", "lfm"]),
        LanguageSpec {
            name: "pascal-form",
            grammar: None,
            kind: ExtractorKind::PascalForm
        },
        "matrix/sample.dfm",
        "object Form1: TForm1\nend\n"
    ),
    registry_case!(
        "lazarus-package",
        RegistryMatcher::Extensions(&["lpk"]),
        LanguageSpec {
            name: "lazarus-package",
            grammar: None,
            kind: ExtractorKind::LazarusPackage
        },
        "matrix/sample.lpk",
        "<CONFIG></CONFIG>\n"
    ),
    registry_case!(
        "bash",
        RegistryMatcher::Extensions(&["sh", "bash"]),
        spec("bash", "bash", ExtractorKind::Generic),
        "matrix/sample.sh",
        "matrix() { :; }\n"
    ),
    registry_case!(
        "json",
        RegistryMatcher::Extensions(&["json"]),
        spec("json", "json", ExtractorKind::JsonConfig),
        "matrix/package.json",
        "{\"name\":\"matrix\",\"dependencies\":{\"example\":\"1.0.0\"}}\n"
    ),
    registry_case!(
        "terraform",
        RegistryMatcher::Extensions(&["tf", "tfvars", "hcl"]),
        spec("terraform", "hcl", ExtractorKind::Terraform),
        "matrix/sample.tf",
        "variable \"matrix\" {}\n"
    ),
    registry_case!(
        "dreammaker",
        RegistryMatcher::Extensions(&["dm", "dme", "dmi", "dmm", "dmf"]),
        LanguageSpec {
            name: "dreammaker",
            grammar: None,
            kind: ExtractorKind::DreamMaker
        },
        "matrix/sample.dm",
        "world\n    name = \"matrix\"\n"
    ),
    registry_case!(
        "solution",
        RegistryMatcher::Extensions(&["sln", "slnx"]),
        LanguageSpec {
            name: "solution",
            grammar: None,
            kind: ExtractorKind::Solution
        },
        "matrix/sample.sln",
        "Microsoft Visual Studio Solution File, Format Version 12.00\n"
    ),
    registry_case!(
        "project-xml",
        RegistryMatcher::Extensions(&["csproj", "fsproj", "vbproj"]),
        LanguageSpec {
            name: "project-xml",
            grammar: None,
            kind: ExtractorKind::ProjectXml
        },
        "matrix/sample.csproj",
        "<Project />\n"
    ),
    registry_case!(
        "xaml",
        RegistryMatcher::Extensions(&["xaml"]),
        LanguageSpec {
            name: "xaml",
            grammar: None,
            kind: ExtractorKind::Xaml
        },
        "matrix/sample.xaml",
        "<Page />\n"
    ),
    registry_case!(
        "razor",
        RegistryMatcher::Extensions(&["razor", "cshtml"]),
        spec("razor", "razor", ExtractorKind::Template),
        "matrix/sample.razor",
        "<div>matrix</div>\n"
    ),
    registry_case!(
        "apex",
        RegistryMatcher::Extensions(&["cls", "trigger"]),
        spec("apex", "apex", ExtractorKind::Generic),
        "matrix/Sample.cls",
        "class Sample {}\n"
    ),
    registry_case!(
        "shebang-python",
        RegistryMatcher::Shebang(&["python", "python2", "python3"]),
        spec("python", "python", ExtractorKind::Generic),
        "matrix/bin/python-tool",
        "#!/usr/bin/env python3\ndef matrix():\n    return 1\n"
    ),
    registry_case!(
        "shebang-bash",
        RegistryMatcher::Shebang(&["bash", "sh", "dash", "zsh", "ksh"]),
        spec("bash", "bash", ExtractorKind::Generic),
        "matrix/bin/shell-tool",
        "#!/usr/bin/env bash\nmatrix() { :; }\n"
    ),
    registry_case!(
        "shebang-node",
        RegistryMatcher::Shebang(&["node", "nodejs"]),
        spec("javascript", "javascript", ExtractorKind::Generic),
        "matrix/bin/node-tool",
        "#!/usr/bin/env node\nfunction matrix() {}\n"
    ),
    registry_case!(
        "shebang-ruby",
        RegistryMatcher::Shebang(&["ruby"]),
        spec("ruby", "ruby", ExtractorKind::Generic),
        "matrix/bin/ruby-tool",
        "#!/usr/bin/env ruby\ndef matrix\nend\n"
    ),
    registry_case!(
        "shebang-perl",
        RegistryMatcher::Shebang(&["perl", "perl5", "perl6"]),
        spec("perl", "perl", ExtractorKind::Generic),
        "matrix/bin/perl-tool",
        "#!/usr/bin/env perl\nsub matrix { 1 }\n"
    ),
    registry_case!(
        "shebang-lua",
        RegistryMatcher::Shebang(&["lua"]),
        spec("lua", "lua", ExtractorKind::Generic),
        "matrix/bin/lua-tool",
        "#!/usr/bin/env lua\nfunction matrix() end\n"
    ),
    registry_case!(
        "shebang-php",
        RegistryMatcher::Shebang(&["php"]),
        spec("php", "php", ExtractorKind::Generic),
        "matrix/bin/php-tool",
        "#!/usr/bin/env php\n<?php function matrix() {}\n"
    ),
    registry_case!(
        "shebang-julia",
        RegistryMatcher::Shebang(&["julia"]),
        spec("julia", "julia", ExtractorKind::Generic),
        "matrix/bin/julia-tool",
        "#!/usr/bin/env julia\nfunction matrix()\nend\n"
    ),
    registry_case!(
        "shebang-r",
        RegistryMatcher::Shebang(&["Rscript"]),
        LanguageSpec {
            name: "r",
            grammar: None,
            kind: ExtractorKind::Generic
        },
        "matrix/bin/r-tool",
        "#!/usr/bin/env Rscript\nmatrix_fn <- function() 1\n"
    ),
];

fn matcher_matches(matcher: RegistryMatcher, path: &Path) -> bool {
    let Some(raw_name) = path.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    let name = raw_name.to_ascii_lowercase();
    let extension = extension_lower(path);
    match matcher {
        RegistryMatcher::ExactNames(names) => names.contains(&raw_name),
        RegistryMatcher::LowerNames(names) => names.contains(&name.as_str()),
        RegistryMatcher::Suffixes(suffixes) => suffixes.iter().any(|suffix| name.ends_with(suffix)),
        RegistryMatcher::ParentFile { file_name, parent } => {
            name == file_name
                && path
                    .parent()
                    .and_then(Path::file_name)
                    .and_then(|value| value.to_str())
                    .is_some_and(|value| value.eq_ignore_ascii_case(parent))
        }
        RegistryMatcher::Extensions(extensions) => extension
            .as_deref()
            .is_some_and(|value| extensions.contains(&value)),
        RegistryMatcher::Header(markers, fallback) => {
            if extension.as_deref() != Some("h") {
                return false;
            }
            let source = std::fs::read(path).unwrap_or_default();
            let matched = markers
                .iter()
                .any(|marker| source.windows(marker.len()).any(|window| window == *marker));
            if fallback {
                !OBJC_MARKERS
                    .iter()
                    .chain(CPP_HEADER_MARKERS)
                    .any(|marker| source.windows(marker.len()).any(|window| window == *marker))
            } else {
                matched
            }
        }
        RegistryMatcher::ObjectiveCSource => {
            extension.as_deref() == Some("m") && objc_source_spec(path).is_some()
        }
        RegistryMatcher::Include(php) => {
            extension.as_deref() == Some("inc") && looks_like_php(path) == php
        }
        RegistryMatcher::Shebang(interpreters) => {
            extension.is_none()
                && shebang_interpreter(path)
                    .as_deref()
                    .is_some_and(|value| interpreters.contains(&value))
        }
    }
}

fn extension_lower(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
}

fn looks_like_php(path: &Path) -> bool {
    std::fs::read(path)
        .ok()
        .is_some_and(|source| source.windows(5).any(|window| window == b"<?php"))
}

fn shebang_interpreter(path: &Path) -> Option<String> {
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
    Some(interpreter)
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
