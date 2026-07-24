use std::error::Error;
use std::fs;
use std::path::Path;

use compass_languages::{Registry, file_stem, make_id, normalize_id};

#[test]
fn registry_covers_every_python_dispatch_extension() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let ordinary = [
        "py", "js", "jsx", "mjs", "cjs", "ts", "tsx", "mts", "cts", "go", "rs", "java", "groovy",
        "gradle", "c", "cpp", "cc", "cxx", "hpp", "cu", "cuh", "metal", "rb", "rake", "cs", "kt",
        "kts", "scala", "php", "swift", "lua", "luau", "toc", "zig", "ps1", "psm1", "psd1", "ex",
        "exs", "mm", "jl", "f", "F", "f90", "F90", "f95", "F95", "f03", "F03", "f08", "F08", "vue",
        "svelte", "astro", "dart", "v", "sv", "svh", "sql", "md", "mdx", "qmd", "skill", "pas",
        "pp", "dpr", "dpk", "lpr", "inc", "dfm", "lfm", "lpk", "sh", "bash", "json", "tf",
        "tfvars", "hcl", "dm", "dme", "dmi", "dmm", "dmf", "sln", "slnx", "csproj", "fsproj",
        "vbproj", "xaml", "razor", "cshtml", "cls", "trigger", "pl", "pm",
    ];
    for extension in ordinary {
        let path = directory.path().join(format!("sample.{extension}"));
        fs::write(&path, "")?;
        assert!(Registry::resolve(&path).is_some(), "missing .{extension}");
    }
    let objc = directory.path().join("sample.m");
    fs::write(&objc, "@implementation Compass\n@end\n")?;
    assert_eq!(Registry::resolve(&objc).map(|spec| spec.name), Some("objc"));
    let header = directory.path().join("sample.h");
    fs::write(&header, "class Compass {};")?;
    assert_eq!(
        Registry::resolve(&header).map(|spec| spec.name),
        Some("cpp")
    );
    Ok(())
}

#[test]
fn every_declared_grammar_is_statically_available() {
    for grammar in [
        "apex",
        "astro",
        "bash",
        "blade",
        "c",
        "cpp",
        "csharp",
        "dart",
        "elixir",
        "fortran",
        "go",
        "groovy",
        "hcl",
        "java",
        "javascript",
        "json",
        "julia",
        "kotlin",
        "lua",
        "objc",
        "pascal",
        "perl",
        "php",
        "powershell",
        "python",
        "razor",
        "ruby",
        "rust",
        "scala",
        "sql",
        "svelte",
        "swift",
        "tsx",
        "typescript",
        "verilog",
        "vue",
        "zig",
    ] {
        assert!(
            tree_sitter_language_pack::get_language(grammar).is_ok(),
            "grammar {grammar} is not linked"
        );
    }
}

#[test]
fn ids_match_python_unicode_casefold_contract() {
    assert_eq!(normalize_id("Straße / API"), "strasse_api");
    assert_eq!(normalize_id("ＡＰＩ café"), "api_café");
    assert_eq!(normalize_id("用户/服务"), "用户_服务");
    assert_eq!(normalize_id("बनाया इतिहास"), "बन_य_इत_ह_स");
    assert_eq!(normalize_id("การติดตั้ง"), "การต_ดต_ง");
    assert_eq!(normalize_id("ref_@scope//package"), "ref_scope_package");
    assert_eq!(normalize_id("a___b"), "a_b");
    assert_eq!(
        make_id(&["src/auth/session.py", "ValidateToken"]),
        "src_auth_session_py_validatetoken"
    );
    assert_eq!(
        normalize_id(normalize_id("Straße / API").as_str()),
        "strasse_api"
    );
    assert_eq!(
        file_stem(Path::new("src/auth/session.py")),
        "src/auth/session"
    );
    assert_eq!(file_stem(Path::new("README")), "README");
    assert_eq!(file_stem(Path::new("")), "");
    assert!(Registry::resolve(Path::new("archive.zip")).is_none());
}
