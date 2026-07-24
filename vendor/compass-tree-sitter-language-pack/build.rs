use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Deserialize)]
struct LanguageDefinition {
    #[allow(dead_code)]
    repo: String,
    #[allow(dead_code)]
    rev: Option<String>,
    #[allow(dead_code)]
    branch: Option<String>,
    #[allow(dead_code)]
    directory: Option<String>,
    #[allow(dead_code)]
    generate: Option<bool>,
    #[allow(dead_code)]
    abi_version: Option<u32>,
    #[serde(default)]
    extensions: Vec<String>,
    #[serde(default)]
    ambiguous: BTreeMap<String, Vec<String>>,
    /// Override for the C symbol name when it differs from the language name.
    /// E.g. language "csharp" exports `tree_sitter_c_sharp()`.
    #[serde(default)]
    c_symbol: Option<String>,
}

fn find_project_root() -> PathBuf {
    if let Ok(root) = env::var("PROJECT_ROOT") {
        return PathBuf::from(root);
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());

    let mut dir = manifest_dir.as_path();
    loop {
        if dir.join("sources/language_definitions.json").exists() && dir.join("parsers").exists() {
            return dir.to_path_buf();
        }
        match dir.parent() {
            Some(parent) => dir = parent,
            None => break,
        }
    }

    // ~keep Published crates include crate-local language_definitions.json but not workspace sources/parsers.
    manifest_dir
}

const COMPASS_STATIC_LANGUAGES: &[&str] = &[
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
];

fn selected_languages(definitions: &BTreeMap<String, LanguageDefinition>) -> Vec<String> {
    COMPASS_STATIC_LANGUAGES
        .iter()
        .map(|name| {
            assert!(
                definitions.contains_key(*name),
                "Compass grammar {name} is absent from the pinned language definitions"
            );
            (*name).to_owned()
        })
        .collect()
}

/// Get the target OS, using CARGO_CFG_TARGET_OS for cross-compilation correctness.
fn target_os() -> String {
    env::var("CARGO_CFG_TARGET_OS").unwrap_or_else(|_| {
        if cfg!(target_os = "macos") {
            "macos".to_string()
        } else if cfg!(target_os = "windows") {
            "windows".to_string()
        } else {
            "linux".to_string()
        }
    })
}

/// Get shared library filename components for the target OS.
fn shared_lib_components(target_os: &str) -> (&'static str, &'static str) {
    match target_os {
        "macos" | "ios" => ("lib", "dylib"),
        "windows" => ("", "dll"),
        _ => ("lib", "so"),
    }
}

/// Compile a single language parser into a shared library (.so/.dylib/.dll).
///
/// When a language has a `c_symbol` override (e.g. "csharp" → "c_sharp"),
/// the library is named using the c_symbol so the runtime loader can find it.
fn compile_parser_dynamic(
    name: &str,
    c_symbol: Option<&str>,
    parser_dir: &Path,
    output_dir: &Path,
) -> bool {
    let src_dir = parser_dir.join("src");
    let parser_c = src_dir.join("parser.c");

    if !parser_c.exists() {
        println!(
            "cargo:warning=Skipping language '{}': parser.c not found at {}",
            name,
            parser_c.display()
        );
        return false;
    }

    let mut c_sources = vec![parser_c];
    let scanner_c = src_dir.join("scanner.c");
    if scanner_c.exists() {
        c_sources.push(scanner_c);
    }

    let scanner_cc = src_dir.join("scanner.cc");

    let mut includes = vec![src_dir.clone()];
    let common_dir = parser_dir.join("common");
    if common_dir.exists() {
        includes.push(common_dir);
    }

    let sym = c_symbol.unwrap_or(name);
    let lib_name = format!("tree_sitter_{sym}");
    let os = target_os();
    let (prefix, ext) = shared_lib_components(&os);
    let output_path = output_dir.join(format!("{prefix}{lib_name}.{ext}"));

    let compiler = cc::Build::new().get_compiler();
    let is_msvc = compiler.is_like_msvc();

    let mut cmd = compiler.to_command();

    if is_msvc {
        // ~keep Use per-file MSVC language flags; global `/TC`/`/TP` breaks parser.c C99 initializers.
        cmd.arg("/std:c11");
        cmd.arg("/utf-8");
        cmd.arg("/O2");
        cmd.arg("/wd4244");
        cmd.arg("/wd4566");
        cmd.arg("/wd4819");
        for inc in &includes {
            cmd.arg(format!("/I{}", inc.display()));
        }
        for src in &c_sources {
            cmd.arg(format!("/Tc{}", src.display()));
        }
        if scanner_cc.exists() {
            cmd.arg(format!("/Tp{}", scanner_cc.display()));
        }
        cmd.arg("/LD");
        cmd.arg(format!("/Fe:{}", output_path.display()));
    } else {
        cmd.arg("-std=c11");
        cmd.arg("-O2");
        cmd.arg("-fno-strict-aliasing");
        cmd.arg("-fPIC");
        for inc in &includes {
            cmd.arg(format!("-I{}", inc.display()));
        }
        for src in &c_sources {
            cmd.arg(src);
        }
        if scanner_cc.exists() {
            // ~keep Mixed C/C++ shared libs compile scanner.cc to an object first, then link everything.
            let scanner_obj = output_dir.join(format!("{name}_scanner.o"));
            let cpp_compiler = cc::Build::new().cpp(true).get_compiler();
            let mut cpp_cmd = cpp_compiler.to_command();
            cpp_cmd.arg("-c");
            cpp_cmd.arg("-fPIC");
            cpp_cmd.arg("-O2");
            cpp_cmd.arg("-fno-strict-aliasing");
            for inc in &includes {
                cpp_cmd.arg(format!("-I{}", inc.display()));
            }
            cpp_cmd.arg(&scanner_cc);
            cpp_cmd.arg("-o");
            cpp_cmd.arg(&scanner_obj);
            let cpp_status = cpp_cmd.status();
            match cpp_status {
                Ok(s) if s.success() => {
                    cmd.arg(&scanner_obj);
                }
                Ok(s) => {
                    println!(
                        "cargo:warning=Failed to compile C++ scanner for '{}': exit code {:?}",
                        name,
                        s.code()
                    );
                    return false;
                }
                Err(e) => {
                    println!(
                        "cargo:warning=Failed to run C++ compiler for '{}': {}",
                        name, e
                    );
                    return false;
                }
            }
        }

        if os == "macos" || os == "ios" {
            cmd.arg("-dynamiclib");
        } else {
            cmd.arg("-shared");
        }
        // ~keep C++ scanners require the C++ standard library for std:: and ABI symbols.
        if scanner_cc.exists() {
            if os == "macos" || os == "ios" {
                cmd.arg("-lc++");
            } else {
                cmd.arg("-lstdc++");
            }
        }
        cmd.arg("-o");
        cmd.arg(&output_path);
    }

    let status = cmd.status();
    match status {
        Ok(s) if s.success() => {
            // ~keep Some compilers return success without a dylib after resource exhaustion; verify output exists.
            if output_path.exists() {
                true
            } else {
                println!(
                    "cargo:warning=Failed to compile shared library for '{}': compiler succeeded but output file was not created at {}",
                    name,
                    output_path.display()
                );
                false
            }
        }
        Ok(s) => {
            println!(
                "cargo:warning=Failed to compile shared library for '{}': exit code {:?}",
                name,
                s.code()
            );
            false
        }
        Err(e) => {
            println!("cargo:warning=Failed to run compiler for '{}': {}", name, e);
            false
        }
    }
}

/// Find the wasi-sysroot include path for wasm32 cross-compilation.
/// Checks WASI_SYSROOT env, then common Homebrew/system paths.
fn find_wasi_sysroot() -> Option<PathBuf> {
    if let Ok(sysroot) = env::var("WASI_SYSROOT") {
        let p = PathBuf::from(sysroot);
        if p.exists() {
            return Some(p);
        }
    }

    let candidates = [
        "/opt/homebrew/share/wasi-sysroot",
        "/usr/local/share/wasi-sysroot",
        "/opt/wasi-sdk/share/wasi-sysroot",
    ];
    for candidate in &candidates {
        let p = PathBuf::from(candidate);
        if p.exists() {
            return Some(p);
        }
    }

    if let Ok(entries) = fs::read_dir("/opt/homebrew/Cellar/wasi-libc") {
        for entry in entries.flatten() {
            let sysroot = entry.path().join("share/wasi-sysroot");
            if sysroot.exists() {
                return Some(sysroot);
            }
        }
    }

    None
}

/// Optimize cc::Build for wasm32 targets to reduce memory usage on CI runners.
/// Disables debug info to reduce object file sizes.
fn apply_wasm32_optimizations(build: &mut cc::Build) {
    if env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default() == "wasm32" {
        build.cargo_warnings(false);
        build.debug(false);
        build.opt_level(2);
    }
}

/// Default upper bound (bytes) for a grammar's `parser.c` when compiling to wasm32.
///
/// A handful of grammars ship pathologically large *generated* `parser.c` files (e.g. `abl` at
/// ~130 MB). Compiling one of those to wasm32 needs 18-25 GB＋ of clang RAM at *any* optimization
/// level (the cost is in parsing/IR-building the giant single-function source, not optimization),
/// which OOMs standard ≤16 GB CI runners — `CARGO_BUILD_JOBS=1` cannot help because a single file
/// already exceeds the budget. 40 MB keeps every common language (including the ~40 MB `sql`
/// grammar) while excluding only the unbuildable outliers.
const DEFAULT_WASM_MAX_PARSER_BYTES: u64 = 40 * 1024 * 1024;

/// Resolve the wasm32 `parser.c` size limit. Returns `None` (gate disabled) only when
/// `TSLP_WASM_MAX_PARSER_BYTES=0`. Any unparsable value falls back to the default.
fn wasm_parser_size_limit() -> Option<u64> {
    match env::var("TSLP_WASM_MAX_PARSER_BYTES") {
        Ok(raw) => match raw.trim().parse::<u64>() {
            Ok(0) => None,
            Ok(limit) => Some(limit),
            Err(_) => Some(DEFAULT_WASM_MAX_PARSER_BYTES),
        },
        Err(_) => Some(DEFAULT_WASM_MAX_PARSER_BYTES),
    }
}

/// Grammars whose external scanners cannot be compiled/linked for
/// `wasm32-unknown-unknown` and are skipped on that target by default.
///
/// Each depends on host libc/libc++ facilities that wasi-libc does not provide
/// freestanding:
/// - `gitcommit`, `perl` — `<wctype.h>` (`iswcntrl`/`iswspace`) needs locale
///   tables absent on wasm32; `perl` also calls `fprintf(stderr, …)` in a DEBUG
///   macro (no stderr on wasm32).
/// - `mojo`, `nim` — C++ `<cwctype>` (`std::iswspace`) fails to link against
///   wasi-libc's C++ wctype/locale layer.
/// - `norg` — C++ `<regex>` + `<locale>` + `<iostream>`, none available on wasm32.
/// - `tmux` — its ~30 MB generated `parser.c` is under the size gate but still
///   overruns the wasm32 clang backend during codegen (fails to compile), so it
///   is skipped explicitly rather than by the byte-size heuristic.
///
/// These are niche grammars; dropping them from the wasm bundle degrades
/// gracefully (absent from `STATIC_LANGUAGES`, no dangling FFI symbol). Override
/// the set with `TSLP_WASM_SKIP_GRAMMARS` (comma-separated; empty disables the
/// skip). Mirrors the `TSLP_WASM_MAX_PARSER_BYTES` size-gate pattern.
const DEFAULT_WASM_SKIP_GRAMMARS: [&str; 6] = ["gitcommit", "mojo", "nim", "norg", "perl", "tmux"];

/// Resolve the wasm32 grammar skip-list. Returns the default set unless
/// `TSLP_WASM_SKIP_GRAMMARS` is set, in which case its comma-separated entries
/// replace the default (an empty/whitespace value disables the skip entirely).
fn wasm_skip_grammars() -> Vec<String> {
    match env::var("TSLP_WASM_SKIP_GRAMMARS") {
        Ok(raw) => raw
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        Err(_) => DEFAULT_WASM_SKIP_GRAMMARS
            .iter()
            .map(|s| s.to_string())
            .collect(),
    }
}

/// Apply wasi-sysroot includes to a cc::Build for wasm32 targets.
///
/// Use `-isystem` to add the wasm32-wasi include dir which has stdlib.h etc.
/// Avoid `--sysroot` which pulls in wasi/api.h through stdio.h and fails
/// for wasm32-unknown-unknown targets.
fn apply_wasm32_sysroot(build: &mut cc::Build) {
    if env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default() != "wasm32" {
        return;
    }

    if let Some(sysroot) = find_wasi_sysroot() {
        let wasi_include = sysroot.join("include/wasm32-wasi");
        if wasi_include.exists() {
            // ~keep Define __wasi__ only for parser C compilation so wasi/api.h guards pass.
            build.define("__wasi__", None);
            build.flag(format!("-isystem{}", wasi_include.display()));
        }
    } else {
        eprintln!(
            "wasm32 target detected but no wasi-sysroot found. \
                  Install wasi-libc (brew install wasi-libc) or set WASI_SYSROOT env var."
        );
    }
}

/// Compile a parser statically and link it into the main binary.
///
/// Compiles parser.c and scanner.c/cc separately to avoid symbol collisions
/// when statically linking multiple grammars. Scanner functions (`scan`,
/// `deserialize`, `serialize`, `scan_comment`) are prefixed with the language
/// name via C preprocessor defines.
fn compile_parser_static(name: &str, parser_dir: &Path) -> bool {
    let src_dir = parser_dir.join("src");
    let parser_c = src_dir.join("parser.c");
    let common_dir = parser_dir.join("common");

    let mut build = cc::Build::new();
    build
        .include(&src_dir)
        .file(&parser_c)
        .define("TREE_SITTER_HIDE_SYMBOLS", None)
        .flag_if_supported("-fvisibility=hidden")
        .flag_if_supported("-fno-strict-aliasing")
        .warnings(false);
    build.std("c11");
    apply_wasm32_sysroot(&mut build);
    apply_wasm32_optimizations(&mut build);
    if common_dir.exists() {
        build.include(&common_dir);
    }

    if let Err(e) = build.try_compile(&format!("tree_sitter_{name}_parser")) {
        println!(
            "cargo:warning=Failed to compile parser for '{}': {}",
            name, e
        );
        return false;
    }

    // ~keep Prefix scanner symbols to avoid collisions when multiple grammars are statically linked.
    let scanner_c = src_dir.join("scanner.c");
    if scanner_c.exists() {
        let mut scanner_build = cc::Build::new();
        scanner_build
            .include(&src_dir)
            .file(&scanner_c)
            .define("TREE_SITTER_HIDE_SYMBOLS", None)
            .define("scan", &*format!("tree_sitter_{name}_ext_scan"))
            .define(
                "deserialize",
                &*format!("tree_sitter_{name}_ext_deserialize"),
            )
            .define("serialize", &*format!("tree_sitter_{name}_ext_serialize"))
            .define(
                "scan_comment",
                &*format!("tree_sitter_{name}_ext_scan_comment"),
            )
            .flag_if_supported("-fvisibility=hidden")
            .flag_if_supported("-fno-strict-aliasing")
            .warnings(false);
        scanner_build.std("c11");
        apply_wasm32_sysroot(&mut scanner_build);
        apply_wasm32_optimizations(&mut scanner_build);
        if common_dir.exists() {
            scanner_build.include(&common_dir);
        }
        if let Err(e) = scanner_build.try_compile(&format!("tree_sitter_{name}_scanner")) {
            println!(
                "cargo:warning=Failed to compile C scanner for '{}': {}",
                name, e
            );
            return false;
        }
    }

    // ~keep C++ scanners also need separate compilation with the same symbol prefixing.
    let scanner_cc = src_dir.join("scanner.cc");
    if scanner_cc.exists() {
        let mut cpp_build = cc::Build::new();
        cpp_build
            .include(&src_dir)
            .file(&scanner_cc)
            .define("TREE_SITTER_HIDE_SYMBOLS", None)
            .define("scan", &*format!("tree_sitter_{name}_ext_scan"))
            .define(
                "deserialize",
                &*format!("tree_sitter_{name}_ext_deserialize"),
            )
            .define("serialize", &*format!("tree_sitter_{name}_ext_serialize"))
            .flag_if_supported("-fvisibility=hidden")
            .flag_if_supported("-fno-strict-aliasing")
            .warnings(false)
            .cpp(true);
        apply_wasm32_sysroot(&mut cpp_build);
        apply_wasm32_optimizations(&mut cpp_build);
        if common_dir.exists() {
            cpp_build.include(&common_dir);
        }
        if let Err(e) = cpp_build.try_compile(&format!("tree_sitter_{name}_scanner_cpp")) {
            println!(
                "cargo:warning=Failed to compile C++ scanner for '{}': {}",
                name, e
            );
            return false;
        }
    }

    true
}

fn generate_registry(
    static_langs: &[String],
    dynamic_langs: &[String],
    definitions: &BTreeMap<String, LanguageDefinition>,
    libs_dir: &Path,
    out_dir: &Path,
) {
    let path = out_dir.join("registry_generated.rs");
    let mut f = fs::File::create(&path).expect("Failed to create registry_generated.rs");

    writeln!(f, "// Auto-generated by build.rs — do not edit").unwrap();
    writeln!(
        f,
        "// This file is include!()'d into registry.rs which imports tree_sitter::Language"
    )
    .unwrap();
    writeln!(f).unwrap();

    let c_sym = |name: &str| -> String {
        definitions
            .get(name)
            .and_then(|d| d.c_symbol.as_deref())
            .unwrap_or(name)
            .to_string()
    };

    if !static_langs.is_empty() {
        for name in static_langs {
            let sym = c_sym(name);
            writeln!(f, "unsafe extern \"C\" {{").unwrap();
            writeln!(
                f,
                "    fn tree_sitter_{sym}() -> *const tree_sitter::ffi::TSLanguage;"
            )
            .unwrap();
            writeln!(f, "}}").unwrap();
        }
        writeln!(f).unwrap();
    }

    writeln!(
        f,
        "#[allow(unused, clippy::type_complexity)]\npub(crate) static STATIC_LANGUAGES: &[(&str, fn() -> Language)] = &["
    )
    .unwrap();
    for name in static_langs {
        let sym = c_sym(name);
        writeln!(
            f,
            "    (\"{name}\", || unsafe {{ Language::from_raw(tree_sitter_{sym}()) }}),",
        )
        .unwrap();
    }
    writeln!(f, "];").unwrap();
    writeln!(f).unwrap();

    writeln!(
        f,
        "#[allow(unused)]\npub(crate) static DYNAMIC_LANGUAGE_NAMES: &[&str] = &["
    )
    .unwrap();
    for name in dynamic_langs {
        writeln!(f, "    \"{name}\",").unwrap();
    }
    writeln!(f, "];").unwrap();
    writeln!(f).unwrap();

    writeln!(
        f,
        "#[allow(unused)]\npub(crate) static LIBS_DIR: &str = {:?};",
        libs_dir.display().to_string()
    )
    .unwrap();
    writeln!(f).unwrap();

    // ~keep Download/runtime paths need C symbols for all definitions, not just compiled languages.
    writeln!(
        f,
        "#[allow(unused)]\npub(crate) static C_SYMBOL_OVERRIDES: &[(&str, &str)] = &["
    )
    .unwrap();
    for (name, def) in definitions.iter() {
        if let Some(c_sym) = &def.c_symbol {
            writeln!(f, "    (\"{name}\", \"{c_sym}\"),").unwrap();
        }
    }
    writeln!(f, "];").unwrap();
    writeln!(f).unwrap();

    // ~keep This is the canonical known-language list, independent of static or dynamic compilation.
    // ~keep Download callers use it to probe a language before triggering on-demand download.
    writeln!(
        f,
        "#[allow(unused)]\npub(crate) static KNOWN_LANGUAGES: &[&str] = &["
    )
    .unwrap();
    for name in definitions.keys() {
        writeln!(f, "    \"{name}\",").unwrap();
    }
    writeln!(f, "];").unwrap();
}

/// Emit rerun-if-changed for specific source files in a parser directory.
fn emit_rerun_if_changed(parser_dir: &Path) {
    let src_dir = parser_dir.join("src");
    for file in &["parser.c", "scanner.c", "scanner.cc"] {
        let path = src_dir.join(file);
        if path.exists() {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
}

/// Generate extension-to-language lookup code from language_definitions.json.
///
/// Reads extensions from the JSON definitions and generates a match expression
/// that maps file extensions to language names. Validates strictly:
/// - No duplicate extensions across languages
/// - Extensions must be ASCII, lowercase, non-empty
/// - Extensions must not contain dots, whitespace, or special characters
fn generate_extensions_lookup(definitions: &BTreeMap<String, LanguageDefinition>, out_dir: &Path) {
    let mut ext_to_lang: BTreeMap<String, String> = BTreeMap::new();

    for (lang_name, def) in definitions {
        for ext in &def.extensions {
            if ext.is_empty() {
                panic!("Empty extension for language '{lang_name}' in language_definitions.json");
            }
            if !ext.is_ascii() {
                panic!(
                    "Non-ASCII extension '{ext}' for language '{lang_name}' in language_definitions.json"
                );
            }
            if ext != &ext.to_ascii_lowercase() {
                panic!(
                    "Extension '{ext}' for language '{lang_name}' must be lowercase in language_definitions.json"
                );
            }
            if ext.contains('.') {
                panic!(
                    "Extension '{ext}' for language '{lang_name}' must not contain dots (use 'py' not '.py') in language_definitions.json"
                );
            }
            if !ext.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                panic!(
                    "Extension '{ext}' for language '{lang_name}' contains invalid characters in language_definitions.json. Only alphanumeric and underscore allowed."
                );
            }
            if let Some(existing) = ext_to_lang.get(ext) {
                panic!(
                    "Duplicate extension '{ext}' in language_definitions.json: mapped to both '{existing}' and '{lang_name}'"
                );
            }
            ext_to_lang.insert(ext.clone(), lang_name.clone());
        }
    }

    let mut lang_to_exts: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (ext, lang) in &ext_to_lang {
        lang_to_exts
            .entry(lang.as_str())
            .or_default()
            .push(ext.as_str());
    }

    let path = out_dir.join("extensions_generated.rs");
    let mut f = fs::File::create(&path).expect("Failed to create extensions_generated.rs");

    writeln!(
        f,
        "// Auto-generated by build.rs from sources/language_definitions.json — do not edit"
    )
    .unwrap();
    writeln!(f, "{{").unwrap();
    writeln!(f, "    let mut buf = [0u8; 32];").unwrap();
    writeln!(
        f,
        "    let ext_lower = if ext.len() <= buf.len() && ext.is_ascii() {{"
    )
    .unwrap();
    writeln!(f, "        for (i, b) in ext.bytes().enumerate() {{").unwrap();
    writeln!(f, "            buf[i] = b.to_ascii_lowercase();").unwrap();
    writeln!(f, "        }}").unwrap();
    writeln!(f, "        match std::str::from_utf8(&buf[..ext.len()]) {{").unwrap();
    writeln!(f, "            Ok(s) => s,").unwrap();
    writeln!(f, "            Err(_) => return None,").unwrap();
    writeln!(f, "        }}").unwrap();
    writeln!(f, "    }} else {{").unwrap();
    writeln!(f, "        return None;").unwrap();
    writeln!(f, "    }};").unwrap();
    writeln!(f).unwrap();
    writeln!(f, "    match ext_lower {{").unwrap();

    for (lang, exts) in &lang_to_exts {
        let patterns: Vec<String> = exts.iter().map(|e| format!("\"{e}\"")).collect();
        writeln!(f, "        {} => Some(\"{lang}\"),", patterns.join(" | ")).unwrap();
    }

    writeln!(f, "        _ => None,").unwrap();
    writeln!(f, "    }}").unwrap();
    writeln!(f, "}}").unwrap();

    let mut ambiguities: BTreeMap<String, (String, Vec<String>)> = BTreeMap::new();
    for (lang_name, def) in definitions {
        for (ext, alternatives) in &def.ambiguous {
            if !def.extensions.contains(ext) {
                panic!(
                    "Ambiguous extension '{ext}' for language '{lang_name}' is not in its extensions list"
                );
            }
            for alt in alternatives {
                if !definitions.contains_key(alt) {
                    panic!(
                        "Ambiguous alternative '{alt}' for extension '{ext}' (language '{lang_name}') does not exist in language_definitions.json"
                    );
                }
            }
            ambiguities.insert(ext.clone(), (lang_name.clone(), alternatives.clone()));
        }
    }

    let ambig_path = out_dir.join("ambiguities_generated.rs");
    let mut af = fs::File::create(&ambig_path).expect("Failed to create ambiguities_generated.rs");

    writeln!(
        af,
        "// Auto-generated by build.rs from sources/language_definitions.json — do not edit"
    )
    .unwrap();
    writeln!(af, "match ext_lower {{").unwrap();
    for (ext, (assigned, alternatives)) in &ambiguities {
        let alt_strs: Vec<String> = alternatives.iter().map(|a| format!("\"{a}\"")).collect();
        writeln!(
            af,
            "    \"{ext}\" => Some((\"{assigned}\", &[{}])),",
            alt_strs.join(", ")
        )
        .unwrap();
    }
    writeln!(af, "    _ => None,").unwrap();
    writeln!(af, "}}").unwrap();
}

/// Generate bundled highlight/injection/locals/tags/indents/folds query functions from
/// parsers/{lang}/queries/*.scm.
///
/// Scans the parsers directory for query files and generates a Rust source file with six
/// match functions: `get_highlights_query_impl`, `get_injections_query_impl`,
/// `get_locals_query_impl`, `get_tags_query_impl`, `get_indents_query_impl`, and
/// `get_folds_query_impl`. Only languages that actually have the relevant .scm file on disk
/// at build time are included.
///
/// Query overlay files in `query-overlays/{lang}/{file}` (relative to the project root) take
/// precedence over the vendored `parsers/{lang}/queries/{file}` files. This allows adding or
/// replacing queries for upstream grammars without modifying the gitignored `parsers/` tree.
fn generate_queries_registry(
    definitions: &BTreeMap<String, LanguageDefinition>,
    parsers_dir: &Path,
    out_dir: &Path,
) {
    let path = out_dir.join("queries_generated.rs");
    let mut f = fs::File::create(&path).expect("Failed to create queries_generated.rs");

    writeln!(f, "// Auto-generated by build.rs — do not edit").unwrap();
    writeln!(f).unwrap();

    // ~keep Resolve project root for `query-overlays`, which is a sibling of `crates/`.
    let overlays_dir = {
        let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
        // ~keep Walk up to the directory containing `query-overlays` or `sources/language_definitions.json`.
        let mut candidate = manifest_dir.as_path();
        loop {
            if candidate.join("query-overlays").exists()
                || candidate.join("sources/language_definitions.json").exists()
            {
                break;
            }
            match candidate.parent() {
                Some(p) => candidate = p,
                None => {
                    candidate = &manifest_dir;
                    break;
                }
            }
        }
        candidate.join("query-overlays")
    };

    // ~keep Query overlay files intentionally win over vendored query files.
    let effective_query_path = |lang: &str, file: &str| -> Option<PathBuf> {
        let overlay = overlays_dir.join(lang).join(file);
        if overlay.exists() {
            return Some(overlay);
        }
        let vendored = parsers_dir.join(lang).join("queries").join(file);
        if vendored.exists() {
            return Some(vendored);
        }
        None
    };

    let mut highlights: Vec<String> = Vec::new();
    let mut injections: Vec<String> = Vec::new();
    let mut locals: Vec<String> = Vec::new();
    let mut tags: Vec<String> = Vec::new();
    let mut indents: Vec<String> = Vec::new();
    let mut folds: Vec<String> = Vec::new();

    for lang in definitions.keys() {
        if let Some(p) = effective_query_path(lang, "highlights.scm") {
            highlights.push(lang.clone());
            println!("cargo:rerun-if-changed={}", p.display());
        }
        if let Some(p) = effective_query_path(lang, "injections.scm") {
            injections.push(lang.clone());
            println!("cargo:rerun-if-changed={}", p.display());
        }
        if let Some(p) = effective_query_path(lang, "locals.scm") {
            locals.push(lang.clone());
            println!("cargo:rerun-if-changed={}", p.display());
        }
        if let Some(p) = effective_query_path(lang, "tags.scm") {
            tags.push(lang.clone());
            println!("cargo:rerun-if-changed={}", p.display());
        }
        if let Some(p) = effective_query_path(lang, "indents.scm") {
            indents.push(lang.clone());
            println!("cargo:rerun-if-changed={}", p.display());
        }
        if let Some(p) = effective_query_path(lang, "folds.scm") {
            folds.push(lang.clone());
            println!("cargo:rerun-if-changed={}", p.display());
        }
    }

    // ~keep Rerun on overlay directory changes so newly added overlay files rebuild immediately.
    if overlays_dir.exists() {
        println!("cargo:rerun-if-changed={}", overlays_dir.display());
    }

    // ~keep Embed query contents instead of include_str! because packaged crates break parser-relative paths.
    let gen_query_fn = |f: &mut fs::File, name: &str, langs: &[String], query_file: &str| {
        writeln!(
            f,
            "pub(crate) fn {name}(lang: &str) -> Option<&'static str> {{"
        )
        .unwrap();
        if langs.is_empty() {
            writeln!(f, "    let _ = lang;").unwrap();
            writeln!(f, "    None").unwrap();
        } else {
            writeln!(f, "    match lang {{").unwrap();
            for lang in langs {
                let query_path = effective_query_path(lang, query_file).unwrap_or_else(|| {
                    panic!(
                        "query file {query_file} for {lang} disappeared between scan and codegen"
                    )
                });
                let contents = fs::read_to_string(&query_path)
                    .unwrap_or_else(|e| panic!("Failed to read {}: {e}", query_path.display()));
                let escaped = contents
                    .replace('\\', "\\\\")
                    .replace('"', "\\\"")
                    .replace('\r', "")
                    .replace('\n', "\\n");
                writeln!(f, "        \"{lang}\" => Some(\"{escaped}\"),",).unwrap();
            }
            writeln!(f, "        _ => None,").unwrap();
            writeln!(f, "    }}").unwrap();
        }
        writeln!(f, "}}").unwrap();
        writeln!(f).unwrap();
    };

    gen_query_fn(
        &mut f,
        "get_highlights_query_impl",
        &highlights,
        "highlights.scm",
    );
    gen_query_fn(
        &mut f,
        "get_injections_query_impl",
        &injections,
        "injections.scm",
    );
    gen_query_fn(&mut f, "get_locals_query_impl", &locals, "locals.scm");
    gen_query_fn(&mut f, "get_tags_query_impl", &tags, "tags.scm");
    gen_query_fn(&mut f, "get_indents_query_impl", &indents, "indents.scm");
    gen_query_fn(&mut f, "get_folds_query_impl", &folds, "folds.scm");
}

/// Probe whether the parsers tree at `root` looks populated for the requested
/// selection. Uses the same heuristic in both the workspace and OUT_DIR cache
/// locations.
fn parsers_root_populated(root: &Path, selected: &[String]) -> bool {
    match selected.first() {
        Some(first) => root.join(first).join("src/parser.c").exists(),
        None => root.join("python").join("src/parser.c").exists(),
    }
}

/// Try to populate the workspace `parsers/` tree by invoking
/// `scripts/clone_vendors.py` from `project_root`. Returns true if the script
/// ran successfully AND the tree is populated afterwards. Returns false if no
/// script is present, no runner is available, or the script failed.
///
/// This is the local-development fallback: a fresh workspace clone has
/// `parsers/` gitignored, and a rc whose release hasn't been published yet has
/// no GH tarball — so neither the workspace nor the remote path works. Cloning
/// upstream grammars via the script bridges the gap.
fn try_clone_vendors_locally(project_root: &Path, parsers_dir: &Path, selected: &[String]) -> bool {
    let clone_script = project_root.join("scripts/clone_vendors.py");
    if !clone_script.exists() {
        return false;
    }

    println!(
        "cargo:warning=parsers/ tree is empty; running scripts/clone_vendors.py to populate from upstream grammars"
    );

    let runners: &[&[&str]] = &[
        &["uv", "run", "--no-sync", "scripts/clone_vendors.py"],
        &["uv", "run", "scripts/clone_vendors.py"],
        &["python3", "scripts/clone_vendors.py"],
        &["python", "scripts/clone_vendors.py"],
    ];
    for cmd_args in runners {
        let mut cmd = std::process::Command::new(cmd_args[0]);
        cmd.args(&cmd_args[1..]);
        cmd.current_dir(project_root);
        match cmd.status() {
            Ok(s) if s.success() => {
                if parsers_root_populated(parsers_dir, selected) {
                    return true;
                }
                println!(
                    "cargo:warning=clone_vendors.py ({}) succeeded but parsers/ is still not populated",
                    cmd_args.join(" ")
                );
                return false;
            }
            Ok(s) => {
                println!(
                    "cargo:warning=clone_vendors.py ({}) exited with {:?}",
                    cmd_args.join(" "),
                    s.code()
                );
            }
            Err(e) => {
                println!(
                    "cargo:warning=could not run clone_vendors.py via '{}': {e}",
                    cmd_args[0]
                );
            }
        }
    }
    false
}

/// In-place fixups for vendored grammar sources that don't compile under MSVC.
///
/// MSVC rejects two C99/GCC-isms tree-sitter scanners commonly use:
///   * Variable-length arrays (crystal scanner.c:1307).
///   * `__attribute__((unused))` parameter annotations (sml scanner.c).
///
/// The substitutions below are equivalent on every compiler — the array bound
/// `MAX_HEREDOC_WORD_SIZE + 4` is the same compile-time constant the runtime
/// already caps `max_word_size` at (lines 1303-1305), and dropping the unused-
/// attribute keeps the parameter signatures intact.
///
/// A sentinel comment is written on the first run so subsequent build.rs
/// invocations short-circuit without re-reading or re-writing the source.
fn apply_msvc_compat_patches(parsers_dir: &Path) {
    #[allow(clippy::type_complexity)]
    let patches: &[(&str, &str, &[(&str, &str)])] = &[
        (
            "crystal",
            "src/scanner.c",
            &[(
                "uint8_t word[max_word_size + 4];",
                "uint8_t word[MAX_HEREDOC_WORD_SIZE + 4]; /* TSLP_MSVC_PATCH: VLA → fixed bound */",
            )],
        ),
        (
            "sml",
            "src/scanner.c",
            &[(
                "__attribute__ ((unused))",
                "/* TSLP_MSVC_PATCH: dropped __attribute__((unused)) for MSVC */",
            )],
        ),
    ];

    for (lang, rel_path, subs) in patches {
        let path = parsers_dir.join(lang).join(rel_path);
        let Ok(original) = fs::read_to_string(&path) else {
            continue;
        };
        if original.contains("TSLP_MSVC_PATCH") {
            continue;
        }
        let mut patched = original.clone();
        for (from, to) in *subs {
            patched = patched.replace(from, to);
        }
        if patched != original
            && let Err(e) = fs::write(&path, patched)
        {
            println!(
                "cargo:warning=Failed to apply MSVC compat patch to {} ({lang}): {e}",
                path.display()
            );
        }
    }
}

/// Resolve grammar sources for the build. Order of preference:
/// 1. Workspace `parsers/` tree is already populated → use it as-is.
/// 2. `TSLP_OFFLINE=1` → return the empty workspace dir; downstream code falls
///    into the "missing parser, skipping" warning path.
/// 3. OUT_DIR cache already populated from a prior run → reuse it.
/// 4. Workspace tree exists (`scripts/clone_vendors.py` is present) → invoke
///    it to clone upstream grammars locally. Bridges the gap for fresh
///    workspace clones and unpublished rc builds where the remote release
///    tarball doesn't exist yet.
/// 5. Otherwise (sdist install or step 4 failed) → download the
///    `parser-sources-{version}.tar.zst` release asset and unpack it under
///    OUT_DIR. `TSLP_SOURCE_BUNDLE_URL` overrides the URL (also accepts
///    `file://` for local-bundle smoke testing).
///
/// Returns the directory to use as the parsers root.
fn ensure_parser_sources(parsers_dir: &Path, selected: &[String], out_dir: &Path) -> PathBuf {
    if parsers_root_populated(parsers_dir, selected) {
        return parsers_dir.to_path_buf();
    }
    if env::var("TSLP_OFFLINE").is_ok_and(|v| !v.is_empty() && v != "0") {
        println!("cargo:warning=TSLP_OFFLINE set; refusing to download parser sources");
        return parsers_dir.to_path_buf();
    }

    let cache_dir = out_dir.join("_parsers");
    // ~keep Same two-probe strategy for the OUT_DIR parser-source cache.
    let cache_populated = match selected.first() {
        Some(first) => cache_dir.join(first).join("src/parser.c").exists(),
        None => cache_dir
            .join("parsers")
            .join("python")
            .join("src/parser.c")
            .exists(),
    };
    if cache_populated {
        let inner = cache_dir.join("parsers");
        return if inner.is_dir() { inner } else { cache_dir };
    }

    // ~keep Dev checkouts clone upstream grammars before falling back to release tarballs.
    // ~keep This covers fresh workspaces and unpublished rc builds.
    let project_root = parsers_dir.parent().unwrap_or(parsers_dir).to_path_buf();
    println!("cargo:rerun-if-env-changed=TSLP_OFFLINE");
    println!("cargo:rerun-if-env-changed=TSLP_SOURCE_BUNDLE_URL");
    if try_clone_vendors_locally(&project_root, parsers_dir, selected) {
        return parsers_dir.to_path_buf();
    }

    let version = env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "unknown".to_string());
    let default_url = format!(
        "https://github.com/xberg-io/tree-sitter-language-pack/releases/download/v{version}/parser-sources-{version}.tar.zst"
    );
    let url = env::var("TSLP_SOURCE_BUNDLE_URL").unwrap_or(default_url);
    let sha_url = format!("{url}.sha256");

    println!("cargo:warning=Downloading parser sources from {url}");

    let body = fetch_bytes(&url).unwrap_or_else(|e| {
        panic!(
            "Failed to download parser sources from {url}: {e}. Set TSLP_OFFLINE=1 to skip, or ensure network access and that v{version} is published with a parser-sources-{version}.tar.zst asset."
        )
    });
    let sha_text = fetch_text(&sha_url)
        .unwrap_or_else(|error| panic!("Failed to download checksum from {sha_url}: {error}"));
    let expected = sha_text
        .split_whitespace()
        .next()
        .filter(|digest| digest.len() == 64 && digest.chars().all(|byte| byte.is_ascii_hexdigit()))
        .unwrap_or_else(|| panic!("Invalid SHA-256 response from {sha_url}"))
        .to_lowercase();
    let digest = Sha256::digest(&body);
    let actual: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    assert_eq!(
        expected, actual,
        "SHA-256 mismatch for {url}: expected {expected}, got {actual}"
    );

    fs::create_dir_all(&cache_dir).expect("create OUT_DIR/_parsers");
    let cursor = std::io::Cursor::new(body);
    let decoder = zstd::stream::read::Decoder::with_buffer(cursor).expect("zstd decoder");
    let mut archive = tar::Archive::new(decoder);
    archive
        .unpack(&cache_dir)
        .expect("extract parser-sources tarball");

    // ~keep Release tarballs contain top-level `parsers/` and `sources/`, nested under OUT_DIR extraction.
    let inner = cache_dir.join("parsers");
    if inner.is_dir() { inner } else { cache_dir }
}

fn fetch_bytes(url: &str) -> Result<Vec<u8>, String> {
    if let Some(path) = url.strip_prefix("file://") {
        return fs::read(path).map_err(|e| format!("read {path}: {e}"));
    }

    // ~keep Retry transport errors, including intermittent GitHub release CDN 504s during publish verification.
    let max_attempts = 6u32;
    let mut last_err = String::new();
    for attempt in 1..=max_attempts {
        let call_result = ureq::get(url)
            .config()
            .timeout_global(Some(Duration::from_secs(300)))
            .build()
            .call();

        match call_result {
            Ok(response) => {
                let mut reader = response.into_body().into_reader();
                let mut buf = Vec::new();
                match std::io::Read::read_to_end(&mut reader, &mut buf) {
                    Ok(_) => return Ok(buf),
                    Err(e) => last_err = format!("read body for {url}: {e}"),
                }
            }
            Err(e) => last_err = format!("GET {url}: {e}"),
        }

        if attempt < max_attempts {
            let backoff = Duration::from_secs(2u64.saturating_pow(attempt));
            println!(
                "cargo:warning={last_err}; retrying in {}s (attempt {}/{})",
                backoff.as_secs(),
                attempt + 1,
                max_attempts
            );
            std::thread::sleep(backoff);
        }
    }
    Err(last_err)
}

fn fetch_text(url: &str) -> Result<String, String> {
    let bytes = fetch_bytes(url)?;
    String::from_utf8(bytes).map_err(|e| format!("decode utf8: {e}"))
}

fn main() {
    println!("cargo:rerun-if-env-changed=TSLP_LANGUAGES");
    println!("cargo:rerun-if-env-changed=PROJECT_ROOT");
    println!("cargo:rerun-if-env-changed=TSLP_LINK_MODE");
    println!("cargo:rerun-if-env-changed=TSLP_WASM_MAX_PARSER_BYTES");

    let project_root = find_project_root();

    // ~keep Definitions live in workspace sources during development and crate-local JSON after publish.
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let crate_local = manifest_dir.join("language_definitions.json");
    let workspace_path = project_root.join("sources/language_definitions.json");
    // ~keep Prefer workspace definitions and fall back to crate-local copy for crates.io packages.
    let definitions_path = if workspace_path.exists() {
        // ~keep Keep the crate-local copy in sync so `cargo publish` includes it.
        if crate_local.parent().is_some_and(|p| p.exists()) {
            let _ = fs::copy(&workspace_path, &crate_local);
        }
        workspace_path
    } else {
        crate_local
    };

    let definitions: BTreeMap<String, LanguageDefinition> = if definitions_path.exists() {
        println!("cargo:rerun-if-changed={}", definitions_path.display());
        let definitions_json = fs::read_to_string(&definitions_path)
            .unwrap_or_else(|e| panic!("Failed to read {}: {e}", definitions_path.display()));
        serde_json::from_str(&definitions_json).expect("Failed to parse language_definitions.json")
    } else {
        BTreeMap::new()
    };
    let workspace_parsers_dir = project_root.join("parsers");

    let selected = selected_languages(&definitions);

    // Compass always uses statically linked grammars on every supported target.
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let link_mode = "static".to_owned();

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let libs_dir = out_dir.join("libs");
    fs::create_dir_all(&libs_dir).expect("Failed to create libs directory");

    // ~keep sdist installs lack local parser sources, so fetch release artifacts instead.
    let parsers_dir = ensure_parser_sources(&workspace_parsers_dir, &selected, &out_dir);
    apply_msvc_compat_patches(&parsers_dir);

    let mut static_compiled = Vec::new();
    let mut dynamic_compiled = Vec::new();
    let mut failed = Vec::new();
    let mut skipped_wasm: Vec<String> = Vec::new();

    // ~keep On wasm32, skip oversized parser.c files before compile to avoid OOM and dangling FFI symbols.
    let wasm_size_limit = if target_arch == "wasm32" {
        wasm_parser_size_limit()
    } else {
        None
    };

    // ~keep On wasm32, skip scanners incompatible with wasi-libc to avoid dangling FFI symbols.
    let wasm_skip = if target_arch == "wasm32" {
        wasm_skip_grammars()
    } else {
        Vec::new()
    };

    for name in &selected {
        let parser_dir = parsers_dir.join(name);
        let parser_c = parser_dir.join("src/parser.c");
        if !parser_c.exists() {
            println!(
                "cargo:warning=Parser sources not found for '{}', skipping",
                name
            );
            failed.push(name.clone());
            continue;
        }

        if wasm_skip.iter().any(|g| g == name) {
            println!(
                "cargo:warning=wasm32: skipping grammar '{}' — its external scanner is not wasm32-compatible (wctype/locale or C++ stdlib). Override with TSLP_WASM_SKIP_GRAMMARS.",
                name,
            );
            skipped_wasm.push(name.clone());
            continue;
        }

        if let Some(limit) = wasm_size_limit
            && let Ok(size) = fs::metadata(&parser_c).map(|m| m.len())
            && size > limit
        {
            println!(
                "cargo:warning=wasm32: skipping grammar '{}' — parser.c is {} MB (limit {} MB); too large to compile to wasm32 within runner memory. Override with TSLP_WASM_MAX_PARSER_BYTES (0 disables the gate).",
                name,
                size / (1024 * 1024),
                limit / (1024 * 1024),
            );
            skipped_wasm.push(name.clone());
            continue;
        }

        let out_dir = env::var("OUT_DIR").unwrap_or_default();
        if !parser_dir.starts_with(&out_dir) {
            emit_rerun_if_changed(&parser_dir);
        }

        let ok = match link_mode.as_str() {
            "static" => {
                let ok = compile_parser_static(name, &parser_dir);
                if ok {
                    static_compiled.push(name.clone());
                }
                ok
            }
            "dynamic" => {
                let c_sym = definitions
                    .get(name.as_str())
                    .and_then(|d| d.c_symbol.as_deref());
                let ok = compile_parser_dynamic(name, c_sym, &parser_dir, &libs_dir);
                if ok {
                    dynamic_compiled.push(name.clone());
                }
                ok
            }
            "both" => {
                let ok_s = compile_parser_static(name, &parser_dir);
                if ok_s {
                    static_compiled.push(name.clone());
                }
                let c_sym = definitions
                    .get(name.as_str())
                    .and_then(|d| d.c_symbol.as_deref());
                let ok_d = compile_parser_dynamic(name, c_sym, &parser_dir, &libs_dir);
                if ok_d {
                    dynamic_compiled.push(name.clone());
                }
                ok_s || ok_d
            }
            _ => {
                println!(
                    "cargo:warning=Unknown TSLP_LINK_MODE '{}', defaulting to dynamic",
                    link_mode
                );
                let c_sym = definitions
                    .get(name.as_str())
                    .and_then(|d| d.c_symbol.as_deref());
                let ok = compile_parser_dynamic(name, c_sym, &parser_dir, &libs_dir);
                if ok {
                    dynamic_compiled.push(name.clone());
                }
                ok
            }
        };
        if !ok {
            failed.push(name.clone());
        }
    }

    if !skipped_wasm.is_empty() {
        println!(
            "cargo:warning=wasm32: skipped {} oversized grammar(s) (excluded from the wasm build): {}. Set TSLP_WASM_MAX_PARSER_BYTES=0 to force-compile them on a high-memory builder.",
            skipped_wasm.len(),
            skipped_wasm.join(", "),
        );
    }

    if !failed.is_empty() {
        // ~keep Persist build failures so CI tooling can inspect them after the panic.
        let _ = fs::create_dir_all(&out_dir);
        let failed_path = out_dir.join("failed_languages.txt");
        if let Err(e) = fs::write(&failed_path, failed.join("\n") + "\n") {
            eprintln!(
                "WARNING: Failed to write {} for CI inspection: {}",
                failed_path.display(),
                e
            );
        }
        let message = format!(
            "FAILED to compile {} Compass grammar(s): {}. Compass refuses to build an artifact with advertised-but-missing parsers.",
            failed.len(),
            failed.join(", "),
        );
        panic!("{message}");
    }

    generate_registry(
        &static_compiled,
        &dynamic_compiled,
        &definitions,
        &libs_dir,
        &out_dir,
    );
    generate_extensions_lookup(&definitions, &out_dir);
    generate_queries_registry(&definitions, &parsers_dir, &out_dir);
}
