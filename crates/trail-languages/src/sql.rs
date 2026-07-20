use std::collections::{HashMap, HashSet};
use std::path::Path;

use regex::Regex;
use serde_json::{Map, Value};
use trail_model::{EdgeRecord, NodeRecord};

use crate::{Extraction, file_stem, make_id};

// SQL object references may contain quoted identifiers, including escaped
// double quotes, and may be schema-qualified. Keep the quotes in the captured
// label to match tree-sitter-sql and Python's public extraction contract.
const OBJECT_REFERENCE: &str =
    r#"(?:(?:"(?:""|[^"])*")|[\w$]+)(?:\.(?:(?:"(?:""|[^"])*")|[\w$]+))*"#;

pub(crate) fn extract(path: &Path, source: &[u8]) -> Extraction {
    State::new(path, source).run()
}

struct Statement {
    offset: usize,
    kind: String,
    name: String,
}

struct State<'a> {
    path: &'a Path,
    source: &'a [u8],
    text: &'a str,
    source_file: String,
    stem: String,
    file_id: String,
    extraction: Extraction,
    seen: HashSet<String>,
    tables: HashMap<String, String>,
}

impl<'a> State<'a> {
    fn new(path: &'a Path, source: &'a [u8]) -> Self {
        let source_file = path.to_string_lossy().into_owned();
        Self {
            path,
            source,
            text: std::str::from_utf8(source).unwrap_or_default(),
            stem: file_stem(path),
            file_id: make_id(&[&source_file]),
            source_file,
            extraction: Extraction {
                raw_calls: None,
                ..Extraction::default()
            },
            seen: HashSet::new(),
            tables: HashMap::new(),
        }
    }

    fn run(mut self) -> Extraction {
        let label = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        self.add_file_node(label);
        for statement in statements(self.text) {
            match statement.kind.as_str() {
                "TABLE" => self.add_table(&statement),
                "VIEW" => self.add_view(&statement),
                "FUNCTION" | "PROCEDURE" => self.add_routine(&statement),
                "TRIGGER" => self.add_trigger(&statement),
                "ALTER TABLE" => self.add_alter_table(&statement),
                _ => {}
            }
        }
        self.extraction
    }

    fn add_table(&mut self, statement: &Statement) {
        let id = make_id(&[&self.stem, &statement.name]);
        let at = self.line_at(statement.offset);
        self.add_contained_node(&id, &statement.name, at);
        self.tables
            .insert(statement.name.to_ascii_lowercase(), id.clone());
        let Some(open) = self.text[statement.offset..]
            .find('(')
            .map(|value| statement.offset + value)
        else {
            return;
        };
        let close = matching_paren(self.source, open).unwrap_or_else(|| {
            statement_end(self.text, statement.offset).unwrap_or(self.text.len())
        });
        let block = &self.text[open..close.min(self.text.len())];
        let Ok(references) = Regex::new(&format!(r"(?i)\bREFERENCES\s+({OBJECT_REFERENCE})"))
        else {
            return;
        };
        let mut emitted = HashSet::new();
        for reference in references.captures_iter(block) {
            let Some(name) = reference.get(1).map(|value| value.as_str()) else {
                continue;
            };
            if !emitted.insert(name.to_ascii_lowercase()) {
                continue;
            }
            let target = self.resolve_table(name);
            self.add_edge(&id, &target, "references", at);
        }
    }

    fn add_view(&mut self, statement: &Statement) {
        let id = make_id(&[&self.stem, &statement.name]);
        let at = self.line_at(statement.offset);
        self.add_contained_node(&id, &statement.name, at);
        self.tables
            .insert(statement.name.to_ascii_lowercase(), id.clone());
        let end = statement_end(self.text, statement.offset).unwrap_or(self.text.len());
        self.add_reads(&id, statement.offset, end, false);
    }

    fn add_routine(&mut self, statement: &Statement) {
        let id = make_id(&[&self.stem, &statement.name]);
        let at = self.line_at(statement.offset);
        self.add_contained_node(&id, &format!("{}()", statement.name), at);
        let end = routine_end(self.text, statement.offset);
        let body = &self.text[statement.offset..end];
        let procedural = body.contains("$$")
            || body.to_ascii_lowercase().contains("language plpgsql")
            || body.to_ascii_lowercase().contains("set term");
        if !procedural {
            self.add_reads(&id, statement.offset, end, false);
        }
    }

    fn add_trigger(&mut self, statement: &Statement) {
        let id = make_id(&[&self.stem, &statement.name]);
        let at = self.line_at(statement.offset);
        self.add_contained_node(&id, &statement.name, at);
        let end = routine_end(self.text, statement.offset);
        let body = &self.text[statement.offset..end];
        if let Some(table) = capture_one(body, r"(?i)\bFOR\s+([\w$]+)") {
            let target = self.resolve_table(&table);
            self.add_edge(&id, &target, "triggers", at);
        }
        self.add_reads(&id, statement.offset, end, true);
    }

    fn add_alter_table(&mut self, statement: &Statement) {
        let at = self.line_at(statement.offset);
        let source = if let Some(id) = self.tables.get(&statement.name.to_ascii_lowercase()) {
            id.clone()
        } else {
            let id = make_id(&[&self.stem, &statement.name]);
            self.add_contained_node(&id, &statement.name, at);
            self.tables
                .insert(statement.name.to_ascii_lowercase(), id.clone());
            id
        };
        let end = statement_end(self.text, statement.offset).unwrap_or(self.text.len());
        let body = &self.text[statement.offset..end];
        let Ok(references) = Regex::new(&format!(r"(?i)\bREFERENCES\s+({OBJECT_REFERENCE})"))
        else {
            return;
        };
        for reference in references.captures_iter(body) {
            if let Some(name) = reference.get(1).map(|value| value.as_str()) {
                let target = self.resolve_table(name);
                self.add_edge(&source, &target, "references", at);
            }
        }
    }

    fn add_reads(&mut self, source: &str, start: usize, end: usize, include_writes: bool) {
        let mut patterns = vec![format!(r"(?i)\b(?:FROM|JOIN|INTO)\s+({OBJECT_REFERENCE})")];
        if include_writes {
            patterns.push(format!(r"(?i)\bUPDATE\s+({OBJECT_REFERENCE})"));
        }
        let non_tables = [
            "select", "where", "set", "dual", "null", "true", "false", "first", "skip", "rows",
            "next", "only", "lateral",
        ];
        let body = &self.text[start..end.min(self.text.len())];
        let mut seen = HashSet::new();
        for pattern in patterns {
            let Ok(regex) = Regex::new(&pattern) else {
                continue;
            };
            for reference in regex.captures_iter(body) {
                let (Some(full), Some(name_match)) = (reference.get(0), reference.get(1)) else {
                    continue;
                };
                let name = name_match.as_str();
                let lower = name.to_ascii_lowercase();
                if non_tables.contains(&lower.as_str()) || !seen.insert(lower) {
                    continue;
                }
                let target = self.resolve_table(name);
                self.add_edge(
                    source,
                    &target,
                    "reads_from",
                    self.line_at(start + full.start()),
                );
            }
        }
    }

    fn resolve_table(&self, name: &str) -> String {
        self.tables
            .get(&name.to_ascii_lowercase())
            .cloned()
            .unwrap_or_else(|| make_id(&[&self.stem, name]))
    }

    fn add_file_node(&mut self, label: &str) {
        self.seen.insert(self.file_id.clone());
        let mut attributes = Map::new();
        attributes.insert("label".into(), Value::String(label.to_owned()));
        attributes.insert("file_type".into(), Value::String("code".into()));
        attributes.insert(
            "source_file".into(),
            Value::String(self.source_file.clone()),
        );
        attributes.insert("source_location".into(), Value::Null);
        self.extraction.nodes.push(NodeRecord {
            id: self.file_id.clone(),
            attributes,
        });
    }

    fn add_contained_node(&mut self, id: &str, label: &str, at: usize) {
        if !self.seen.insert(id.to_owned()) {
            return;
        }
        let mut attributes = Map::new();
        attributes.insert("label".into(), Value::String(label.to_owned()));
        attributes.insert("file_type".into(), Value::String("code".into()));
        attributes.insert(
            "source_file".into(),
            Value::String(self.source_file.clone()),
        );
        attributes.insert("source_location".into(), Value::String(format!("L{at}")));
        self.extraction.nodes.push(NodeRecord {
            id: id.to_owned(),
            attributes,
        });
        self.add_edge(&self.file_id.clone(), id, "contains", at);
    }

    fn add_edge(&mut self, source: &str, target: &str, relation: &str, at: usize) {
        let mut attributes = Map::new();
        attributes.insert("relation".into(), Value::String(relation.to_owned()));
        attributes.insert("confidence".into(), Value::String("EXTRACTED".into()));
        attributes.insert(
            "source_file".into(),
            Value::String(self.source_file.clone()),
        );
        attributes.insert("source_location".into(), Value::String(format!("L{at}")));
        attributes.insert("weight".into(), Value::from(1.0));
        self.extraction.edges.push(EdgeRecord {
            source: source.to_owned(),
            target: target.to_owned(),
            attributes,
        });
    }

    fn line_at(&self, offset: usize) -> usize {
        self.source[..offset.min(self.source.len())]
            .iter()
            .filter(|byte| **byte == b'\n')
            .count()
            + 1
    }
}

fn statements(source: &str) -> Vec<Statement> {
    let Ok(pattern) = Regex::new(&format!(
        r"(?i)\b(?:CREATE\s+(?:OR\s+(?:REPLACE|ALTER)\s+)?(TABLE|VIEW|FUNCTION|PROCEDURE|TRIGGER)|((?:ALTER)\s+TABLE))\s+({OBJECT_REFERENCE})"
    )) else {
        return Vec::new();
    };
    pattern
        .captures_iter(source)
        .filter_map(|capture| {
            let full = capture.get(0)?;
            Some(Statement {
                offset: full.start(),
                kind: capture
                    .get(1)
                    .or_else(|| capture.get(2))?
                    .as_str()
                    .to_ascii_uppercase(),
                name: capture.get(3)?.as_str().to_owned(),
            })
        })
        .collect()
}

fn statement_end(source: &str, start: usize) -> Option<usize> {
    source[start..].find(';').map(|offset| start + offset + 1)
}

fn routine_end(source: &str, start: usize) -> usize {
    let tail = &source[start..];
    let Ok(next) = Regex::new(
        r"(?im)^\s*(?:CREATE\s+(?:OR\s+(?:REPLACE|ALTER)\s+)?(?:TABLE|VIEW|FUNCTION|PROCEDURE|TRIGGER)|ALTER\s+TABLE)\b",
    ) else {
        return source.len();
    };
    next.find_iter(tail)
        .nth(1)
        .map_or(source.len(), |value| start + value.start())
}

fn matching_paren(source: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0_u32;
    let mut quote = None;
    let mut escaped = false;
    for (index, byte) in source.iter().enumerate().skip(open) {
        if let Some(delimiter) = quote {
            if escaped {
                escaped = false;
            } else if *byte == b'\\' {
                escaped = true;
            } else if *byte == delimiter {
                quote = None;
            }
            continue;
        }
        if matches!(*byte, b'\'' | b'"') {
            quote = Some(*byte);
        } else if *byte == b'(' {
            depth += 1;
        } else if *byte == b')' {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some(index + 1);
            }
        }
    }
    None
}

fn capture_one(value: &str, pattern: &str) -> Option<String> {
    Regex::new(pattern)
        .ok()?
        .captures(value)
        .and_then(|capture| capture.get(1))
        .map(|capture| capture.as_str().to_owned())
}
