//! Build-time validation for Compass's generated skill bundle.
//!
//! The canonical skill and references are committed assets. This module applies
//! the same important invariants as a standalone generator—deterministic input
//! discovery, reference coverage, native-brand checks, and structural
//! validation—before `compass-cli` embeds them.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

const MINIMUM_CORE_WORDS: usize = 500;
const MAXIMUM_CORE_TOKENS: usize = 5_000;
const MINIMUM_REFERENCES: usize = 10;
const MINIMUM_REFERENCE_WORDS: usize = 120;
const MINIMUM_BUNDLE_WORDS: usize = 5_000;
const CANONICAL_SKILL_SHA256: &str =
    "c6c097e081043c3f57cacb113423ff5783b0391f9881cb3262501277663a5d91";
const FOCUSED_SKILLS: &[&str] = &[
    "compass-architecture",
    "compass-change-impact",
    "compass-debug",
    "compass-index-maintenance",
    "compass-mcp-setup",
    "compass-navigate",
];
const REQUIRED_CORE_SECTIONS: &[&str] = &[
    "## Invocation contract",
    "## Select the evidence before acting",
    "## Fast path: use an existing graph",
    "## Build or refresh",
    "## Command routing",
    "## Answering workflow",
    "## On-demand references",
    "## Completion rules",
];
const REQUIRED_INTEGRATIONS: &[&str] = &[
    "agents-md.md",
    "antigravity-rules.md",
    "antigravity-workflow.md",
    "claude-md.md",
    "gemini-md.md",
    "kilo-command.md",
    "kiro-steering.md",
    "vscode-instructions.md",
];
const DELEGATING_INTEGRATIONS: &[&str] = &["antigravity-workflow.md", "kilo-command.md"];

pub(crate) fn validate(assets: &Path, cli_source: &Path, help_source: &Path) -> io::Result<()> {
    let skill_root = assets.join("compass-skill");
    let skill_path = skill_root.join("SKILL.md");
    let skill = read_utf8(&skill_path)?;
    require(
        has_canonical_frontmatter(&skill),
        &skill_path,
        "frontmatter must start with the canonical Compass skill name",
    )?;
    let canonical_skill = canonical_text(&skill);
    let canonical_digest = format!("{:x}", Sha256::digest(canonical_skill.as_bytes()));
    require(
        canonical_digest == CANONICAL_SKILL_SHA256,
        &skill_path,
        "canonical umbrella skill changed; focused skills must remain additive",
    )?;
    require(
        skill.split_whitespace().count() >= MINIMUM_CORE_WORDS,
        &skill_path,
        "core skill is unexpectedly small",
    )?;
    require(
        approximate_token_count(&skill) <= MAXIMUM_CORE_TOKENS,
        &skill_path,
        "core skill exceeds the Agent Skills activation budget",
    )?;
    require(
        skill.contains("\ndescription:") && skill.contains("\ncompatibility:"),
        &skill_path,
        "frontmatter must describe activation and runtime compatibility",
    )?;
    validate_native(&skill_path, &skill)?;
    for section in REQUIRED_CORE_SECTIONS {
        require(
            skill.contains(section),
            &skill_path,
            &format!("core skill is missing required section {section:?}"),
        )?;
    }
    let openai_metadata_path = skill_root.join("agents/openai.yaml");
    let openai_metadata = read_utf8(&openai_metadata_path)?;
    validate_openai_metadata(&openai_metadata_path, &openai_metadata)?;

    let reference_root = skill_root.join("references");
    let reference_paths = markdown_files(&reference_root)?;
    require(
        reference_paths.len() >= MINIMUM_REFERENCES,
        &reference_root,
        "reference bundle is unexpectedly small",
    )?;

    let actual = reference_paths
        .iter()
        .map(|path| {
            path.strip_prefix(&skill_root)
                .map(path_string)
                .map_err(io::Error::other)
        })
        .collect::<io::Result<BTreeSet<_>>>()?;
    let linked = linked_references(&skill);
    require(
        linked == actual,
        &skill_path,
        &format!("reference index drift: linked={linked:?}, bundled={actual:?}"),
    )?;
    for reference in &actual {
        require(
            skill.match_indices(reference).count() == 1,
            &skill_path,
            &format!("reference index must link {reference} exactly once"),
        )?;
    }

    for path in reference_paths {
        let body = read_utf8(&path)?;
        require(
            body.starts_with("# "),
            &path,
            "reference must start with a level-one heading",
        )?;
        require(
            body.to_ascii_lowercase().contains("compass"),
            &path,
            "reference does not contain a Compass command or path",
        )?;
        require(
            body.split_whitespace().count() >= MINIMUM_REFERENCE_WORDS,
            &path,
            "reference is unexpectedly small",
        )?;
        validate_native(&path, &body)?;
    }

    let all_docs = skill_documents(&skill_root, &skill, &actual)?;
    require(
        all_docs.split_whitespace().count() >= MINIMUM_BUNDLE_WORDS,
        &skill_root,
        "complete skill bundle is unexpectedly small",
    )?;
    validate_command_coverage(cli_source, help_source, &all_docs)?;
    validate_focused_skills(&assets.join("compass-focused-skills"))?;
    validate_integrations(&assets.join("compass-integrations"))?;
    Ok(())
}

fn validate_focused_skills(root: &Path) -> io::Result<()> {
    let actual = fs::read_dir(root)?
        .map(|entry| entry.map(|value| value.path()))
        .collect::<io::Result<Vec<_>>>()?
        .into_iter()
        .filter(|path| path.is_dir())
        .filter_map(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .collect::<BTreeSet<_>>();
    let expected = FOCUSED_SKILLS
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<BTreeSet<_>>();
    require(
        actual == expected,
        root,
        &format!("focused skill inventory drift: expected={expected:?}, actual={actual:?}"),
    )?;

    let corpus = root.join("trigger-corpus.json");
    require(
        corpus.is_file() && fs::metadata(&corpus)?.len() > 100,
        &corpus,
        "focused trigger corpus is missing or unexpectedly small",
    )?;

    for name in FOCUSED_SKILLS {
        let skill_root = root.join(name);
        let skill_path = skill_root.join("SKILL.md");
        let body = canonical_text(&read_utf8(&skill_path)?);
        require(
            body.starts_with(&format!("---\nname: {name}\n")),
            &skill_path,
            "focused skill name must match its lower-kebab directory",
        )?;
        require(
            is_lower_kebab(name),
            &skill_path,
            "focused skill name is not lower-kebab",
        )?;
        let description = frontmatter_value(&body, "description").ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{}: missing description", skill_path.display()),
            )
        })?;
        require(
            !description.is_empty() && description.len() <= 1_024,
            &skill_path,
            "focused skill description must contain 1-1024 characters",
        )?;
        require(
            description.contains("Use ") || description.contains("use "),
            &skill_path,
            "focused skill description must say when to use it",
        )?;
        require(
            body.split_whitespace().count() >= 100,
            &skill_path,
            "focused skill instructions are unexpectedly small",
        )?;
        require(
            approximate_token_count(&body) <= MAXIMUM_CORE_TOKENS,
            &skill_path,
            "focused skill exceeds the Agent Skills activation budget",
        )?;
        validate_native(&skill_path, &body)?;
        validate_portable_paths(&skill_root, &skill_path, &body)?;
    }
    Ok(())
}

fn validate_portable_paths(root: &Path, skill_path: &Path, body: &str) -> io::Result<()> {
    for forbidden in ["/Users/", "/home/", "file://"] {
        require(
            !body.contains(forbidden),
            skill_path,
            &format!("focused skill contains absolute path marker {forbidden:?}"),
        )?;
    }
    require(
        !contains_windows_absolute_path(body),
        skill_path,
        "focused skill contains an absolute Windows path marker",
    )?;
    for reference in linked_references(body) {
        let relative = Path::new(&reference);
        require(
            !relative.is_absolute()
                && !relative
                    .components()
                    .any(|component| matches!(component, std::path::Component::ParentDir)),
            skill_path,
            &format!("focused skill reference is not a safe relative path: {reference}"),
        )?;
        require(
            root.join(relative).is_file(),
            skill_path,
            &format!("focused skill reference is not bundled: {reference}"),
        )?;
    }
    Ok(())
}

fn contains_windows_absolute_path(body: &str) -> bool {
    let bytes = body.as_bytes();
    bytes.windows(3).enumerate().any(|(index, window)| {
        let has_drive_boundary = index == 0 || !bytes[index - 1].is_ascii_alphanumeric();
        has_drive_boundary
            && window[0].is_ascii_alphabetic()
            && window[1] == b':'
            && matches!(window[2], b'\\' | b'/')
    }) || contains_unc_path(bytes)
}

fn contains_unc_path(bytes: &[u8]) -> bool {
    bytes.windows(2).enumerate().any(|(index, prefix)| {
        if prefix != b"\\\\"
            || index
                .checked_sub(1)
                .is_some_and(|previous| bytes[previous] == b'\\')
        {
            return false;
        }
        let server_start = index.saturating_add(2);
        let Some(separator_offset) = bytes[server_start..].iter().position(|byte| *byte == b'\\')
        else {
            return false;
        };
        separator_offset > 0
            && bytes
                .get(
                    server_start
                        .saturating_add(separator_offset)
                        .saturating_add(1),
                )
                .is_some_and(|byte| !byte.is_ascii_whitespace() && *byte != b'\\')
    })
}

fn frontmatter_value<'a>(body: &'a str, key: &str) -> Option<&'a str> {
    let prefix = format!("{key}:");
    body.lines()
        .skip(1)
        .take_while(|line| *line != "---")
        .find_map(|line| {
            line.strip_prefix(&prefix)
                .map(str::trim)
                .map(|value| value.trim_matches('"'))
        })
}

fn is_lower_kebab(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('-')
        && !value.ends_with('-')
        && !value.contains("--")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn canonical_text(input: &str) -> String {
    input.replace("\r\n", "\n").replace('\r', "\n")
}

fn skill_documents(
    skill_root: &Path,
    skill: &str,
    references: &BTreeSet<String>,
) -> io::Result<String> {
    let mut documents = String::with_capacity(skill.len() * 2);
    documents.push_str(skill);
    for reference in references {
        documents.push('\n');
        documents.push_str(&read_utf8(&skill_root.join(reference))?);
    }
    Ok(documents)
}

fn validate_command_coverage(
    cli_source: &Path,
    help_source: &Path,
    documents: &str,
) -> io::Result<()> {
    let cli = read_utf8(cli_source)?;
    let help = read_utf8(help_source)?;
    let commands = public_help_commands(&help).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "{}: could not parse the public help catalog",
                help_source.display()
            ),
        )
    })?;
    require(
        commands.len() >= 30,
        help_source,
        "public command inventory is unexpectedly small",
    )?;
    let dispatched = dispatched_commands(&cli).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{}: could not parse command dispatch", cli_source.display()),
        )
    })?;
    require(
        commands == dispatched,
        cli_source,
        &format!("CLI/help command drift: help={commands:?}, dispatch={dispatched:?}"),
    )?;
    for command in &commands {
        require(
            documents.contains(&format!("compass {command}")),
            cli_source,
            &format!("public command {command:?} is not covered by the skill bundle"),
        )?;
    }
    for internal in ["history-worker", "hook-spawn", "hook-refresh"] {
        require(
            documents.contains(internal),
            cli_source,
            &format!("internal command boundary {internal:?} is undocumented"),
        )?;
    }
    let normalized = documents.split_whitespace().collect::<Vec<_>>().join(" ");
    require(
        normalized.contains("Do not invoke them directly"),
        cli_source,
        "internal command documentation must prohibit direct invocation",
    )
}

fn public_help_commands(source: &str) -> Option<BTreeSet<String>> {
    let (_, tail) = source.split_once("const PAGES: &[Page] = &[")?;
    let (body, _) = tail.split_once("pub fn request_os")?;
    let mut commands = BTreeSet::new();
    let mut awaiting_path = false;
    for line in body.lines().map(str::trim) {
        if line == "page!(" {
            awaiting_path = true;
            continue;
        }
        if !awaiting_path {
            continue;
        }
        let Some(path) = first_string_literal(line) else {
            continue;
        };
        if let Some(command) = path.split_whitespace().next() {
            commands.insert(command.to_owned());
        }
        awaiting_path = false;
    }
    (!commands.is_empty()).then_some(commands)
}

fn dispatched_commands(source: &str) -> Option<BTreeSet<String>> {
    let (_, run) = source.split_once("pub fn run(")?;
    let (_, tail) = run.split_once("match command.as_str() {")?;
    let (body, _) = tail.split_once("\n    };")?;
    let ignored = BTreeSet::from([
        "--help",
        "--version",
        "help",
        "history-worker",
        "hook-refresh",
        "hook-spawn",
        "version",
    ]);
    let commands = body
        .lines()
        .filter(|line| line.contains("=>"))
        .filter_map(|line| first_string_literal(line.trim()))
        .filter(|command| !ignored.contains(command.as_str()))
        .collect::<BTreeSet<_>>();
    (!commands.is_empty()).then_some(commands)
}

fn first_string_literal(line: &str) -> Option<String> {
    let remainder = line.strip_prefix('"')?;
    let end = remainder.find('"')?;
    Some(remainder[..end].to_owned())
}

fn validate_integrations(root: &Path) -> io::Result<()> {
    let integrations = markdown_files(root)?;
    let actual = integrations
        .iter()
        .filter_map(|path| path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .collect::<BTreeSet<_>>();
    let required = REQUIRED_INTEGRATIONS
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<BTreeSet<_>>();
    require(
        actual == required,
        root,
        &format!("integration asset drift: required={required:?}, actual={actual:?}"),
    )?;
    for path in integrations {
        let body = read_utf8(&path)?;
        validate_native(&path, &body)?;
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if DELEGATING_INTEGRATIONS.contains(&name) {
            require(
                has_canonical_frontmatter(&body),
                &path,
                "delegating command must use canonical Compass frontmatter",
            )?;
            require(
                body.contains("canonical installed skill"),
                &path,
                "delegating command must point to the canonical installed skill",
            )?;
            require(
                body.split_whitespace().count() >= 60,
                &path,
                "delegating command is unexpectedly small",
            )?;
        } else {
            for required_text in [
                "compass-out/",
                "compass query",
                "compass update",
                "cited source",
            ] {
                require(
                    body.contains(required_text),
                    &path,
                    &format!("always-on integration is missing {required_text:?}"),
                )?;
            }
            require(
                body.split_whitespace().count() >= 80,
                &path,
                "always-on integration is unexpectedly small",
            )?;
        }
    }
    Ok(())
}

fn validate_openai_metadata(path: &Path, body: &str) -> io::Result<()> {
    for required in [
        "interface:",
        "display_name: \"Compass\"",
        "short_description:",
        "default_prompt:",
        "$compass",
    ] {
        require(
            body.contains(required),
            path,
            &format!("Codex metadata is missing {required:?}"),
        )?;
    }
    require(
        !body.contains("allowed-tools:"),
        path,
        "Codex metadata must not pre-approve tools",
    )
}

fn linked_references(skill: &str) -> BTreeSet<String> {
    let mut output = BTreeSet::new();
    let mut remainder = skill;
    while let Some(index) = remainder.find("references/") {
        let candidate = &remainder[index..];
        if let Some(end) = candidate.find(".md") {
            let path = &candidate[..end + 3];
            if path
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "/-_.".contains(character))
            {
                output.insert(path.to_owned());
            }
            remainder = &candidate[end + 3..];
        } else {
            break;
        }
    }
    output
}

fn markdown_files(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = fs::read_dir(root)?
        .map(|entry| entry.map(|value| value.path()))
        .collect::<io::Result<Vec<_>>>()?
        .into_iter()
        .filter(|path| path.extension().is_some_and(|extension| extension == "md"))
        .collect::<Vec<_>>();
    files.sort();
    Ok(files)
}

fn read_utf8(path: &Path) -> io::Result<String> {
    fs::read_to_string(path)
        .map_err(|error| io::Error::new(error.kind(), format!("{}: {error}", path.display())))
}

fn approximate_token_count(text: &str) -> usize {
    text.len().saturating_add(3) / 4
}

fn has_canonical_frontmatter(body: &str) -> bool {
    let mut lines = body.lines();
    lines.next() == Some("---") && lines.next() == Some("name: compass")
}

fn validate_native(path: &Path, body: &str) -> io::Result<()> {
    let lowercase = body.to_ascii_lowercase();
    require(
        !lowercase.contains("graphify"),
        path,
        "installed content contains a retired product name",
    )?;
    require(
        !lowercase.contains("python -m"),
        path,
        "installed content contains a Python module command",
    )
}

fn require(condition: bool, path: &Path, message: &str) -> io::Result<()> {
    if condition {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{}: {message}", path.display()),
        ))
    }
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::{approximate_token_count, has_canonical_frontmatter, linked_references};

    #[test]
    fn canonical_frontmatter_accepts_lf_and_crlf() {
        assert!(has_canonical_frontmatter(
            "---\nname: compass\ndescription: test\n"
        ));
        assert!(has_canonical_frontmatter(
            "---\r\nname: compass\r\ndescription: test\r\n"
        ));
        assert!(!has_canonical_frontmatter(
            "---\n\nname: compass\ndescription: test\n"
        ));
        assert!(!has_canonical_frontmatter(
            "---\nname: other\ndescription: test\n"
        ));
    }

    #[test]
    fn reference_links_are_deduplicated_and_sorted() {
        let links =
            linked_references("`references/z.md`, `references/a-file.md`, and `references/z.md`");
        assert_eq!(
            links.into_iter().collect::<Vec<_>>(),
            ["references/a-file.md", "references/z.md"]
        );
    }

    #[test]
    fn token_estimate_rounds_up_conservatively() {
        assert_eq!(approximate_token_count(""), 0);
        assert_eq!(approximate_token_count("abcd"), 1);
        assert_eq!(approximate_token_count("abcde"), 2);
    }
}
