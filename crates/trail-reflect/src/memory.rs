use std::fs;
use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;

const MAX_MEMORY_DOC_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MemoryDoc {
    pub query_type: String,
    pub date: String,
    pub question: String,
    pub outcome: String,
    pub correction: String,
    pub contributor: String,
    pub source_nodes: Vec<String>,
    pub path: String,
}

#[must_use]
pub fn parse_memory_doc(text: &str) -> Option<MemoryDoc> {
    if !text.starts_with("---") {
        return None;
    }
    let mut lines = text.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    let mut doc = MemoryDoc::default();
    for line in lines {
        if line.trim() == "---" {
            break;
        }
        if let Some(captures) = list_pattern().captures(line)
            && captures.get(1).map(|value| value.as_str()) == Some("source_nodes")
        {
            let body = captures
                .get(2)
                .map(|value| value.as_str())
                .unwrap_or_default();
            doc.source_nodes = item_pattern()
                .captures_iter(body)
                .filter_map(|item| item.get(1))
                .map(|item| yaml_unescape(item.as_str()))
                .collect();
            continue;
        }
        let Some(captures) = scalar_pattern().captures(line) else {
            continue;
        };
        let key = captures
            .get(1)
            .map(|value| value.as_str())
            .unwrap_or_default();
        let value = yaml_unescape(
            captures
                .get(2)
                .map(|value| value.as_str())
                .unwrap_or_default(),
        );
        match key {
            "type" => doc.query_type = value,
            "date" => doc.date = value,
            "question" => doc.question = value,
            "outcome" => doc.outcome = value,
            "correction" => doc.correction = value,
            "contributor" => doc.contributor = value,
            _ => {}
        }
    }
    Some(doc)
}

#[must_use]
pub fn load_memory_docs(memory_dir: &Path) -> Vec<MemoryDoc> {
    let Ok(entries) = fs::read_dir(memory_dir) else {
        return Vec::new();
    };
    let mut paths = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|extension| extension.to_str()) == Some("md"))
        .collect::<Vec<_>>();
    paths.sort();
    let mut docs = Vec::new();
    for path in paths {
        let Ok(metadata) = path.metadata() else {
            continue;
        };
        if !metadata.is_file() || metadata.len() > MAX_MEMORY_DOC_BYTES {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let Some(mut doc) = parse_memory_doc(&text) else {
            continue;
        };
        doc.path = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        docs.push(doc);
    }
    docs.sort_by(|left, right| (&left.date, &left.path).cmp(&(&right.date, &right.path)));
    docs
}

fn scalar_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r#"^([A-Za-z_][\w-]*):\s*"(.*)"\s*$"#).unwrap_or_else(|_| std::process::abort())
    })
}

fn list_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r"^([A-Za-z_][\w-]*):\s*\[(.*)\]\s*$").unwrap_or_else(|_| std::process::abort())
    })
}

fn item_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r#""((?:[^"\\]|\\.)*)""#).unwrap_or_else(|_| std::process::abort())
    })
}

fn yaml_unescape(value: &str) -> String {
    let characters = value.chars().collect::<Vec<_>>();
    let mut output = String::with_capacity(value.len());
    let mut index = 0;
    while index < characters.len() {
        if characters[index] == '\\' && index + 1 < characters.len() {
            let next = characters[index + 1];
            let simple = match next {
                'n' => Some('\n'),
                'r' => Some('\r'),
                't' => Some('\t'),
                '0' => Some('\0'),
                '"' => Some('"'),
                '\\' => Some('\\'),
                'L' => Some('\u{2028}'),
                'P' => Some('\u{2029}'),
                _ => None,
            };
            if let Some(character) = simple {
                output.push(character);
                index += 2;
                continue;
            }
            let digits = if next == 'x' {
                2
            } else if next == 'u' {
                4
            } else {
                0
            };
            if digits > 0 && index + 2 + digits <= characters.len() {
                let encoded = characters[index + 2..index + 2 + digits]
                    .iter()
                    .collect::<String>();
                if let Ok(point) = u32::from_str_radix(&encoded, 16)
                    && let Some(character) = char::from_u32(point)
                {
                    output.push(character);
                    index += 2 + digits;
                    continue;
                }
            }
        }
        output.push(characters[index]);
        index += 1;
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_saved_frontmatter_subset() {
        let parsed = parse_memory_doc(
            "---\r\ntype: \"explain\"\r\nquestion: \"a \\\"quote\\\"\"\r\noutcome: \"useful\"\r\nsource_nodes: [\"A\", \"Node\\\\Path\"]\r\n---\r\nbody",
        )
        .unwrap_or_default();
        assert_eq!(parsed.query_type, "explain");
        assert_eq!(parsed.question, "a \"quote\"");
        assert_eq!(parsed.source_nodes, ["A", "Node\\Path"]);
    }

    #[test]
    fn yaml_subset_decodes_simple_hex_unicode_and_unknown_escapes() {
        assert_eq!(
            yaml_unescape(r#"\n\r\t\0\"\\\L\P\x41\u263a\q\xzz"#),
            "\n\r\t\0\"\\\u{2028}\u{2029}A☺\\q\\xzz"
        );
        assert!(parse_memory_doc("plain text").is_none());
        assert!(parse_memory_doc("--- not a delimiter").is_none());
        let parsed =
            parse_memory_doc("---\nunknown: \"ignored\"\ninvalid line\n---\n").unwrap_or_default();
        assert_eq!(parsed, MemoryDoc::default());
    }

    #[test]
    fn memory_directory_loading_sorts_valid_markdown_and_skips_other_shapes()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        fs::write(
            directory.path().join("b.md"),
            "---\ndate: \"2026-02-01\"\nquestion: \"B\"\n---\n",
        )?;
        fs::write(
            directory.path().join("a.md"),
            "---\ndate: \"2026-01-01\"\nquestion: \"A\"\n---\n",
        )?;
        fs::write(directory.path().join("invalid.md"), "invalid")?;
        fs::write(directory.path().join("ignored.txt"), "---\n---\n")?;
        fs::create_dir(directory.path().join("directory.md"))?;
        let oversized = fs::File::create(directory.path().join("oversized.md"))?;
        oversized.set_len(MAX_MEMORY_DOC_BYTES + 1)?;

        let docs = load_memory_docs(directory.path());
        assert_eq!(docs.len(), 2);
        assert_eq!(docs[0].path, "a.md");
        assert_eq!(docs[1].path, "b.md");
        assert!(load_memory_docs(&directory.path().join("missing")).is_empty());
        Ok(())
    }
}
