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

const MINIMUM_CORE_WORDS: usize = 500;
const MINIMUM_REFERENCES: usize = 10;
const MINIMUM_REFERENCE_WORDS: usize = 120;
const MINIMUM_BUNDLE_WORDS: usize = 5_000;
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
        skill.starts_with("---\nname: compass\n"),
        &skill_path,
        "frontmatter must start with the canonical Compass skill name",
    )?;
    require(
        skill.split_whitespace().count() >= MINIMUM_CORE_WORDS,
        &skill_path,
        "core skill is unexpectedly small",
    )?;
    validate_native(&skill_path, &skill)?;
    for section in REQUIRED_CORE_SECTIONS {
        require(
            skill.contains(section),
            &skill_path,
            &format!("core skill is missing required section {section:?}"),
        )?;
    }

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
    validate_integrations(&assets.join("compass-integrations"))?;
    Ok(())
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
                body.starts_with("---\nname: compass\n"),
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
    use super::linked_references;

    #[test]
    fn reference_links_are_deduplicated_and_sorted() {
        let links =
            linked_references("`references/z.md`, `references/a-file.md`, and `references/z.md`");
        assert_eq!(
            links.into_iter().collect::<Vec<_>>(),
            ["references/a-file.md", "references/z.md"]
        );
    }
}
