use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::{Extraction, RawEdgeRecord as EdgeRecord, RawNodeRecord as NodeRecord};
use regex::Regex;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::{file_stem, make_id};

// SQL object references may contain quoted identifiers, including escaped
// double quotes, and may be schema-qualified. Keep the quotes in the captured
// label to preserve the source spelling and dialect-specific case semantics.
const IDENTIFIER: &str = r#"(?:"(?:""|[^"])*"|`(?:``|[^`])*`|\[[^\]]+\]|[\w$]+)"#;
const OBJECT_REFERENCE: &str = r#"(?:"(?:""|[^"])*"|`(?:``|[^`])*`|\[[^\]]+\]|[\w$]+)(?:\.(?:"(?:""|[^"])*"|`(?:``|[^`])*`|\[[^\]]+\]|[\w$]+))*"#;

pub(crate) fn extract(path: &Path, source: &[u8]) -> Extraction {
    State::new(path, source).run()
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum StatementKind {
    Database,
    Schema,
    Table,
    View,
    Procedure,
    Trigger,
    Index,
    AlterTable,
}

#[derive(Clone, Debug)]
struct Statement {
    offset: usize,
    end: usize,
    kind: StatementKind,
    name: String,
}

struct State<'a> {
    path: &'a Path,
    source: &'a [u8],
    text: &'a str,
    masked: String,
    source_file: String,
    logical_database: String,
    file_id: String,
    database_id: String,
    extraction: Extraction,
    seen_nodes: HashSet<String>,
    seen_edges: HashSet<(String, String, String, usize)>,
    objects: HashMap<String, String>,
    short_object_names: HashMap<String, Option<String>>,
    schemas: HashMap<String, String>,
}

impl<'a> State<'a> {
    fn new(path: &'a Path, source: &'a [u8]) -> Self {
        let source_file = path.to_string_lossy().into_owned();
        let logical_database = logical_database(path);
        let text = std::str::from_utf8(source).unwrap_or_default();
        Self {
            path,
            source,
            text,
            masked: mask_sql_literals_and_comments(text),
            file_id: make_id(&[&source_file]),
            database_id: make_id(&["sql-database", &logical_database]),
            source_file,
            logical_database,
            extraction: Extraction {
                raw_calls: None,
                ..Extraction::default()
            },
            seen_nodes: HashSet::new(),
            seen_edges: HashSet::new(),
            objects: HashMap::new(),
            short_object_names: HashMap::new(),
            schemas: HashMap::new(),
        }
    }

    fn run(mut self) -> Extraction {
        let label = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_owned();
        self.add_file_node(&label);
        self.add_database_node();
        self.add_migration_node();

        let declarations = statements(&self.masked);
        // Discover all declared objects first. Relationships can then resolve
        // forward references without producing dangling graph endpoints.
        for statement in &declarations {
            self.add_declared_object(statement);
        }
        for statement in &declarations {
            self.add_statement_relationships(statement);
        }
        self.add_queries();
        self.extraction
    }

    fn add_file_node(&mut self, label: &str) {
        let mut attributes = self.base_attributes("file", label, label, 1, "ast");
        attributes.insert("file_type".into(), Value::String("code".into()));
        self.push_node(self.file_id.clone(), attributes);
    }

    fn add_database_node(&mut self) {
        let mut attributes = self.base_attributes(
            "database",
            &self.logical_database,
            &self.logical_database,
            1,
            "convention",
        );
        attributes.insert(
            "logical_database".into(),
            Value::String(self.logical_database.clone()),
        );
        attributes.insert(
            "rule".into(),
            Value::String("sql-file-logical-database".into()),
        );
        self.push_node(self.database_id.clone(), attributes);
        self.add_edge_with_origin(
            &self.file_id.clone(),
            &self.database_id.clone(),
            "contains",
            1,
            "convention",
            Some("sql-file-logical-database"),
        );
    }

    fn add_migration_node(&mut self) {
        if !is_migration_path(self.path) {
            return;
        }
        let name = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("migration");
        let qualified_name = format!("{}::{name}", self.logical_database);
        let id = make_id(&["sql-migration", &self.source_file]);
        let mut attributes =
            self.base_attributes("migration", name, &qualified_name, 1, "convention");
        attributes.insert(
            "rule".into(),
            Value::String("sql-migration-path-convention".into()),
        );
        self.push_node(id.clone(), attributes);
        self.add_edge_with_origin(
            &self.file_id.clone(),
            &id,
            "contains",
            1,
            "convention",
            Some("sql-migration-path-convention"),
        );
    }

    fn add_declared_object(&mut self, statement: &Statement) {
        let at = self.line_at(statement.offset);
        match statement.kind {
            StatementKind::Database => {
                // The per-file logical database remains the stable owner. An
                // explicit CREATE DATABASE name enriches it without changing
                // the identity of every dependent object in the same file.
                if let Some(node) = self
                    .extraction
                    .nodes
                    .iter_mut()
                    .find(|node| node.id == self.database_id)
                {
                    node.attributes
                        .insert("name".into(), Value::String(statement.name.clone()));
                    node.attributes
                        .insert("label".into(), Value::String(statement.name.clone()));
                    node.attributes.insert(
                        "qualified_name".into(),
                        Value::String(statement.name.clone()),
                    );
                    node.attributes
                        .insert("_origin".into(), Value::String("ast".into()));
                    node.attributes.remove("rule");
                    node.attributes
                        .insert("source_location".into(), Value::String(format!("L{at}")));
                    node.attributes.insert("line_start".into(), Value::from(at));
                    node.attributes.insert("line_end".into(), Value::from(at));
                }
            }
            StatementKind::Schema => {
                self.ensure_schema(&statement.name, at, "ast");
            }
            StatementKind::Table => {
                let id = self.add_database_object("database_table", &statement.name, at, "ast");
                self.add_table_members(statement, &id);
            }
            StatementKind::View => {
                self.add_database_object("database_view", &statement.name, at, "ast");
            }
            StatementKind::Procedure => {
                self.add_database_object("database_procedure", &statement.name, at, "ast");
            }
            StatementKind::Trigger => {
                self.add_database_object("database_trigger", &statement.name, at, "ast");
            }
            StatementKind::Index => {
                self.add_database_object("database_index", &statement.name, at, "ast");
            }
            StatementKind::AlterTable => {
                self.ensure_table(&statement.name, at);
                self.add_alter_constraints(statement);
            }
        }
    }

    fn add_statement_relationships(&mut self, statement: &Statement) {
        let Some(source) = self.object_id(&statement.name) else {
            return;
        };
        match statement.kind {
            StatementKind::View | StatementKind::Procedure | StatementKind::Trigger => {
                self.add_data_access(&source, statement.offset, statement.end);
            }
            StatementKind::Table | StatementKind::AlterTable => {
                self.add_foreign_key_references(&source, statement.offset, statement.end);
            }
            StatementKind::Index => self.link_index_to_table(statement, &source),
            StatementKind::Database | StatementKind::Schema => {}
        }
        if statement.kind == StatementKind::Trigger {
            self.link_trigger_to_table(statement, &source);
        }
    }

    fn add_table_members(&mut self, statement: &Statement, table_id: &str) {
        let Some(open) = self.text[statement.offset..statement.end]
            .find('(')
            .map(|offset| statement.offset + offset)
        else {
            return;
        };
        let Some(close) = matching_paren(self.source, open) else {
            return;
        };
        for (relative_offset, member) in split_top_level(&self.text[open + 1..close - 1]) {
            let trimmed = member.trim();
            if trimmed.is_empty() {
                continue;
            }
            let member_offset =
                open + 1 + relative_offset + member.len() - member.trim_start().len();
            let at = self.line_at(member_offset);
            if is_table_constraint(trimmed) {
                let name = constraint_name(trimmed)
                    .unwrap_or_else(|| format!("{}#constraint@{}", statement.name, member_offset));
                let qualified_name = format!("{}::{name}", statement.name);
                let id = make_id(&["sql-constraint", &self.logical_database, &qualified_name]);
                let attributes = self.database_attributes(
                    "database_constraint",
                    &name,
                    &qualified_name,
                    at,
                    "ast",
                );
                self.push_node(id.clone(), attributes);
                self.add_edge(table_id, &id, "contains", at);
                continue;
            }
            let Some(name) = first_identifier(trimmed) else {
                continue;
            };
            let qualified_name = format!("{}.{}", statement.name, name);
            let id = make_id(&["sql-column", &self.logical_database, &qualified_name]);
            let attributes =
                self.database_attributes("database_column", &name, &qualified_name, at, "ast");
            self.push_node(id.clone(), attributes);
            self.add_edge(table_id, &id, "contains", at);
        }
    }

    fn add_alter_constraints(&mut self, statement: &Statement) {
        let body = &self.text[statement.offset..statement.end];
        let Ok(regex) = Regex::new(&format!(r"(?i)\bADD\s+CONSTRAINT\s+({IDENTIFIER})")) else {
            return;
        };
        let Some(table_id) = self.object_id(&statement.name) else {
            return;
        };
        for capture in regex.captures_iter(body) {
            let Some(name_match) = capture.get(1) else {
                continue;
            };
            let name = name_match.as_str();
            let at = self.line_at(statement.offset + name_match.start());
            let qualified_name = format!("{}::{name}", statement.name);
            let id = make_id(&["sql-constraint", &self.logical_database, &qualified_name]);
            let attributes =
                self.database_attributes("database_constraint", name, &qualified_name, at, "ast");
            self.push_node(id.clone(), attributes);
            self.add_edge(&table_id, &id, "contains", at);
        }
    }

    fn add_foreign_key_references(&mut self, source: &str, start: usize, end: usize) {
        let body = self.masked[start..end].to_owned();
        let Ok(regex) = Regex::new(&format!(r"(?i)\bREFERENCES\s+({OBJECT_REFERENCE})")) else {
            return;
        };
        for capture in regex.captures_iter(&body) {
            let Some(name_match) = capture.get(1) else {
                continue;
            };
            let at = self.line_at(start + name_match.start());
            let target = self.ensure_table(name_match.as_str(), at);
            self.add_edge(source, &target, "references", at);
        }
    }

    fn link_index_to_table(&mut self, statement: &Statement, index_id: &str) {
        let body = self.masked[statement.offset..statement.end].to_owned();
        let Ok(regex) = Regex::new(&format!(r"(?i)\bON\s+({OBJECT_REFERENCE})")) else {
            return;
        };
        let Some(capture) = regex.captures(&body) else {
            return;
        };
        let Some(name_match) = capture.get(1) else {
            return;
        };
        let at = self.line_at(statement.offset + name_match.start());
        let table = self.ensure_table(name_match.as_str(), at);
        self.add_edge(&table, index_id, "contains", at);
    }

    fn link_trigger_to_table(&mut self, statement: &Statement, trigger_id: &str) {
        let body = self.masked[statement.offset..statement.end].to_owned();
        let Ok(regex) = Regex::new(&format!(r"(?i)\bON\s+({OBJECT_REFERENCE})")) else {
            return;
        };
        let Some(capture) = regex.captures(&body) else {
            return;
        };
        let Some(name_match) = capture.get(1) else {
            return;
        };
        let at = self.line_at(statement.offset + name_match.start());
        let table = self.ensure_table(name_match.as_str(), at);
        self.add_edge(&table, trigger_id, "contains", at);
        self.add_edge(trigger_id, &table, "triggers", at);
    }

    fn add_queries(&mut self) {
        let Ok(regex) = Regex::new(r"(?im)(?:^|;)\s*(SELECT|INSERT|UPDATE|DELETE|MERGE)\b") else {
            return;
        };
        let matches = regex
            .captures_iter(&self.masked)
            .filter_map(|capture| {
                let operation = capture.get(1)?;
                Some((operation.start(), operation.as_str().to_ascii_lowercase()))
            })
            .collect::<Vec<_>>();
        for (index, (start, operation)) in matches.iter().enumerate() {
            let end = matches.get(index + 1).map_or_else(
                || statement_end(&self.masked, *start).unwrap_or(self.masked.len()),
                |next| next.0,
            );
            self.add_query(operation, *start, end);
        }
    }

    fn add_query(&mut self, operation: &str, start: usize, end: usize) {
        let at = self.line_at(start);
        let qualified_name = format!("{}::{}@{}", self.logical_database, operation, start);
        let id = make_id(&["sql-query", &self.source_file, &start.to_string()]);
        let mut attributes = self.base_attributes("query", operation, &qualified_name, at, "ast");
        attributes.insert("dialect".into(), Value::String("sql".into()));
        attributes.insert("operation".into(), Value::String(operation.to_owned()));
        attributes.insert(
            "text_digest".into(),
            Value::String(sha256_prefixed(
                self.source.get(start..end).unwrap_or_default(),
            )),
        );
        attributes.insert("line_end".into(), Value::from(self.line_at(end)));
        self.push_node(id.clone(), attributes);
        self.add_edge(&self.file_id.clone(), &id, "contains", at);
        self.add_data_access(&id, start, end);
    }

    fn add_data_access(&mut self, source: &str, start: usize, end: usize) {
        let body = self.masked[start..end.min(self.masked.len())].to_owned();
        let read_patterns = [
            format!(r"(?i)\b(?:FROM|JOIN|USING)\s+({OBJECT_REFERENCE})"),
            format!(r"(?i)\bSELECT\b[\s\S]*?\bINTO\s+({OBJECT_REFERENCE})"),
        ];
        let write_patterns = [
            format!(r"(?i)\bINSERT\s+INTO\s+({OBJECT_REFERENCE})"),
            format!(r"(?i)\bUPDATE\s+({OBJECT_REFERENCE})"),
            format!(r"(?i)\bDELETE\s+FROM\s+({OBJECT_REFERENCE})"),
            format!(r"(?i)\bMERGE\s+INTO\s+({OBJECT_REFERENCE})"),
        ];
        self.add_access_matches(source, start, &body, &read_patterns, "reads");
        self.add_access_matches(source, start, &body, &write_patterns, "writes");
    }

    fn add_access_matches(
        &mut self,
        source: &str,
        start: usize,
        body: &str,
        patterns: &[String],
        relation: &str,
    ) {
        let mut emitted = HashSet::new();
        for pattern in patterns {
            let Ok(regex) = Regex::new(pattern) else {
                continue;
            };
            for capture in regex.captures_iter(body) {
                let Some(name_match) = capture.get(1) else {
                    continue;
                };
                let name = name_match.as_str();
                if is_non_table_keyword(name)
                    || !emitted.insert(normalized_identifier(name).to_ascii_lowercase())
                {
                    continue;
                }
                let at = self.line_at(start + name_match.start());
                let target = self.ensure_table(name, at);
                self.add_edge(source, &target, relation, at);
            }
        }
    }

    fn add_database_object(&mut self, kind: &str, name: &str, at: usize, origin: &str) -> String {
        if let Some(existing) = self.object_id(name) {
            return existing;
        }
        let schema = schema_name(name);
        let qualified_name = normalized_identifier(name);
        let id = make_id(&[
            kind,
            &self.logical_database,
            schema.as_deref().unwrap_or_default(),
            &qualified_name,
        ]);
        let attributes = self.database_attributes(kind, name, &qualified_name, at, origin);
        self.push_node(id.clone(), attributes);
        self.register_object(name, &id);
        let parent = schema
            .as_deref()
            .map(|schema| self.ensure_schema(schema, at, origin))
            .unwrap_or_else(|| self.database_id.clone());
        self.add_edge(&parent, &id, "contains", at);
        id
    }

    fn ensure_table(&mut self, name: &str, at: usize) -> String {
        self.object_id(name)
            .unwrap_or_else(|| self.add_database_object("database_table", name, at, "ast"))
    }

    fn ensure_schema(&mut self, name: &str, at: usize, origin: &str) -> String {
        let normalized = normalized_identifier(name);
        let key = normalized.to_ascii_lowercase();
        if let Some(id) = self.schemas.get(&key) {
            return id.clone();
        }
        let id = make_id(&["database_schema", &self.logical_database, &normalized]);
        let mut attributes =
            self.database_attributes("database_schema", name, &normalized, at, origin);
        attributes.insert("database_schema".into(), Value::String(normalized.clone()));
        self.push_node(id.clone(), attributes);
        self.schemas.insert(key, id.clone());
        self.add_edge(&self.database_id.clone(), &id, "contains", at);
        id
    }

    fn object_id(&self, name: &str) -> Option<String> {
        let normalized = normalized_identifier(name).to_ascii_lowercase();
        self.objects.get(&normalized).cloned().or_else(|| {
            let short = last_identifier(name).to_ascii_lowercase();
            self.short_object_names.get(&short).cloned().flatten()
        })
    }

    fn register_object(&mut self, name: &str, id: &str) {
        let normalized = normalized_identifier(name).to_ascii_lowercase();
        self.objects.insert(normalized, id.to_owned());
        let short = last_identifier(name).to_ascii_lowercase();
        self.short_object_names
            .entry(short)
            .and_modify(|entry| {
                if entry.as_deref() != Some(id) {
                    *entry = None;
                }
            })
            .or_insert_with(|| Some(id.to_owned()));
    }

    fn database_attributes(
        &self,
        kind: &str,
        name: &str,
        qualified_name: &str,
        at: usize,
        origin: &str,
    ) -> Map<String, Value> {
        let mut attributes =
            self.base_attributes(kind, &last_identifier(name), qualified_name, at, origin);
        attributes.insert(
            "logical_database".into(),
            Value::String(self.logical_database.clone()),
        );
        if let Some(schema) = schema_name(name) {
            attributes.insert("database_schema".into(), Value::String(schema));
        }
        attributes
    }

    fn base_attributes(
        &self,
        kind: &str,
        name: &str,
        qualified_name: &str,
        at: usize,
        origin: &str,
    ) -> Map<String, Value> {
        let mut attributes = Map::new();
        attributes.insert("label".into(), Value::String(name.to_owned()));
        attributes.insert("name".into(), Value::String(name.to_owned()));
        attributes.insert(
            "qualified_name".into(),
            Value::String(qualified_name.to_owned()),
        );
        attributes.insert("symbol_kind".into(), Value::String(kind.to_owned()));
        attributes.insert("file_type".into(), Value::String("code".into()));
        attributes.insert("language".into(), Value::String("sql".into()));
        attributes.insert(
            "source_file".into(),
            Value::String(self.source_file.clone()),
        );
        attributes.insert("source_location".into(), Value::String(format!("L{at}")));
        attributes.insert("line_start".into(), Value::from(at));
        attributes.insert("line_end".into(), Value::from(at));
        attributes.insert("_origin".into(), Value::String(origin.to_owned()));
        attributes.insert(
            "extractor".into(),
            Value::String("compass.languages.sql".into()),
        );
        attributes
    }

    fn push_node(&mut self, id: String, attributes: Map<String, Value>) {
        if self.seen_nodes.insert(id.clone()) {
            self.extraction.nodes.push(NodeRecord { id, attributes });
        }
    }

    fn add_edge(&mut self, source: &str, target: &str, relation: &str, at: usize) {
        self.add_edge_with_origin(source, target, relation, at, "ast", None);
    }

    fn add_edge_with_origin(
        &mut self,
        source: &str,
        target: &str,
        relation: &str,
        at: usize,
        origin: &str,
        rule: Option<&str>,
    ) {
        if !self.seen_edges.insert((
            source.to_owned(),
            target.to_owned(),
            relation.to_owned(),
            at,
        )) {
            return;
        }
        let mut attributes = Map::new();
        attributes.insert("relation".into(), Value::String(relation.to_owned()));
        attributes.insert("confidence".into(), Value::String("EXTRACTED".into()));
        attributes.insert(
            "source_file".into(),
            Value::String(self.source_file.clone()),
        );
        attributes.insert("source_location".into(), Value::String(format!("L{at}")));
        attributes.insert("weight".into(), Value::from(1.0));
        attributes.insert("_origin".into(), Value::String(origin.to_owned()));
        attributes.insert(
            "extractor".into(),
            Value::String("compass.languages.sql".into()),
        );
        if let Some(rule) = rule {
            attributes.insert("rule".into(), Value::String(rule.to_owned()));
        }
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
        r"(?i)\b(?:CREATE\s+(?:OR\s+(?:REPLACE|ALTER)\s+)?(DATABASE|SCHEMA|TABLE|VIEW|FUNCTION|PROCEDURE|TRIGGER)|CREATE\s+(?:UNIQUE\s+)?(INDEX)|((?:ALTER)\s+TABLE))\s+({OBJECT_REFERENCE})"
    )) else {
        return Vec::new();
    };
    let mut declarations = pattern
        .captures_iter(source)
        .filter_map(|capture| {
            let full = capture.get(0)?;
            let raw_kind = capture
                .get(1)
                .or_else(|| capture.get(2))
                .or_else(|| capture.get(3))?;
            let kind = match raw_kind.as_str().to_ascii_uppercase().as_str() {
                "DATABASE" => StatementKind::Database,
                "SCHEMA" => StatementKind::Schema,
                "TABLE" => StatementKind::Table,
                "VIEW" => StatementKind::View,
                "FUNCTION" | "PROCEDURE" => StatementKind::Procedure,
                "TRIGGER" => StatementKind::Trigger,
                "INDEX" => StatementKind::Index,
                "ALTER TABLE" => StatementKind::AlterTable,
                _ => return None,
            };
            Some(Statement {
                offset: full.start(),
                end: 0,
                kind,
                name: capture.get(4)?.as_str().to_owned(),
            })
        })
        .collect::<Vec<_>>();
    for index in 0..declarations.len() {
        declarations[index].end = declarations.get(index + 1).map_or_else(
            || statement_end(source, declarations[index].offset).unwrap_or(source.len()),
            |next| {
                statement_end(source, declarations[index].offset)
                    .unwrap_or(next.offset)
                    .min(next.offset)
            },
        );
    }
    declarations
}

fn statement_end(source: &str, start: usize) -> Option<usize> {
    source[start..].find(';').map(|offset| start + offset + 1)
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
        if matches!(*byte, b'\'' | b'"' | b'`') {
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

fn split_top_level(value: &str) -> Vec<(usize, &str)> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut depth = 0_u32;
    let mut quote = None;
    let mut escaped = false;
    for (index, byte) in value.bytes().enumerate() {
        if let Some(delimiter) = quote {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == delimiter {
                quote = None;
            }
            continue;
        }
        match byte {
            b'\'' | b'"' | b'`' => quote = Some(byte),
            b'(' => depth += 1,
            b')' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                parts.push((start, &value[start..index]));
                start = index + 1;
            }
            _ => {}
        }
    }
    parts.push((start, &value[start..]));
    parts
}

fn mask_sql_literals_and_comments(value: &str) -> String {
    #[derive(Clone, Copy)]
    enum Mode {
        Sql,
        String,
        LineComment,
        BlockComment,
    }

    let source = value.as_bytes();
    let mut masked = source.to_vec();
    let mut mode = Mode::Sql;
    let mut index = 0;
    while index < source.len() {
        match mode {
            Mode::Sql if source[index] == b'\'' => {
                masked[index] = b' ';
                mode = Mode::String;
                index += 1;
            }
            Mode::Sql if source[index] == b'-' && source.get(index + 1).copied() == Some(b'-') => {
                masked[index] = b' ';
                masked[index + 1] = b' ';
                mode = Mode::LineComment;
                index += 2;
            }
            Mode::Sql if source[index] == b'/' && source.get(index + 1).copied() == Some(b'*') => {
                masked[index] = b' ';
                masked[index + 1] = b' ';
                mode = Mode::BlockComment;
                index += 2;
            }
            Mode::String
                if source[index] == b'\'' && source.get(index + 1).copied() == Some(b'\'') =>
            {
                masked[index] = b' ';
                masked[index + 1] = b' ';
                index += 2;
            }
            Mode::String if source[index] == b'\'' => {
                masked[index] = b' ';
                mode = Mode::Sql;
                index += 1;
            }
            Mode::LineComment if source[index] == b'\n' => {
                mode = Mode::Sql;
                index += 1;
            }
            Mode::BlockComment
                if source[index] == b'*' && source.get(index + 1).copied() == Some(b'/') =>
            {
                masked[index] = b' ';
                masked[index + 1] = b' ';
                mode = Mode::Sql;
                index += 2;
            }
            Mode::String | Mode::LineComment | Mode::BlockComment => {
                if source[index] != b'\n' {
                    masked[index] = b' ';
                }
                index += 1;
            }
            Mode::Sql => index += 1,
        }
    }
    String::from_utf8(masked).unwrap_or_else(|_| value.to_owned())
}

fn is_table_constraint(value: &str) -> bool {
    let upper = value.trim_start().to_ascii_uppercase();
    [
        "CONSTRAINT",
        "PRIMARY",
        "FOREIGN",
        "UNIQUE",
        "CHECK",
        "EXCLUDE",
    ]
    .iter()
    .any(|prefix| upper.starts_with(prefix))
}

fn constraint_name(value: &str) -> Option<String> {
    let regex = Regex::new(&format!(r"(?i)^CONSTRAINT\s+({IDENTIFIER})")).ok()?;
    regex
        .captures(value.trim_start())
        .and_then(|capture| capture.get(1))
        .map(|name| name.as_str().to_owned())
}

fn first_identifier(value: &str) -> Option<String> {
    let regex = Regex::new(&format!(r"^({IDENTIFIER})")).ok()?;
    regex
        .captures(value.trim_start())
        .and_then(|capture| capture.get(1))
        .map(|name| name.as_str().to_owned())
}

fn logical_database(path: &Path) -> String {
    let stem = file_stem(path);
    let normalized = stem
        .trim_start_matches(|character: char| character.is_ascii_digit() || character == '_')
        .trim_end_matches(".up")
        .trim_end_matches(".down");
    if normalized.is_empty() {
        "default".to_owned()
    } else {
        normalized.to_owned()
    }
}

fn is_migration_path(path: &Path) -> bool {
    if path.components().any(|component| {
        component
            .as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case("migrations")
    }) {
        return true;
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    Regex::new(r"(?i)^(?:V?\d+(?:[._-]\d+)*__|\d{8,}[_-])")
        .is_ok_and(|pattern| pattern.is_match(name))
}

fn schema_name(name: &str) -> Option<String> {
    let normalized = normalized_identifier(name);
    normalized
        .rsplit_once('.')
        .map(|(schema, _)| schema.to_owned())
        .filter(|schema| !schema.is_empty())
}

fn last_identifier(name: &str) -> String {
    normalized_identifier(name)
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_owned()
}

fn normalized_identifier(name: &str) -> String {
    name.split('.')
        .map(|part| {
            let trimmed = part.trim();
            if (trimmed.starts_with('"') && trimmed.ends_with('"'))
                || (trimmed.starts_with('`') && trimmed.ends_with('`'))
                || (trimmed.starts_with('[') && trimmed.ends_with(']'))
            {
                trimmed[1..trimmed.len().saturating_sub(1)].to_owned()
            } else {
                trimmed.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(".")
}

fn is_non_table_keyword(name: &str) -> bool {
    matches!(
        normalized_identifier(name).to_ascii_lowercase().as_str(),
        "select"
            | "where"
            | "set"
            | "dual"
            | "null"
            | "true"
            | "false"
            | "first"
            | "skip"
            | "rows"
            | "next"
            | "only"
            | "lateral"
    )
}

fn sha256_prefixed(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_typed_database_domain_without_dangling_edges() {
        let source = br#"
CREATE SCHEMA app;
CREATE TABLE app.users (
    id BIGINT PRIMARY KEY,
    account_id BIGINT,
    CONSTRAINT users_account_fk FOREIGN KEY (account_id) REFERENCES app.accounts(id)
);
CREATE TABLE app.accounts (id BIGINT PRIMARY KEY);
CREATE UNIQUE INDEX users_account_idx ON app.users(account_id);
CREATE VIEW app.active_users AS SELECT * FROM app.users;
CREATE TRIGGER users_audit AFTER UPDATE ON app.users FOR EACH ROW EXECUTE FUNCTION audit_user();
INSERT INTO app.users(id) SELECT id FROM app.accounts;
"#;
        let extraction = extract(Path::new("db/migrations/V1__users.sql"), source);
        let kinds = extraction
            .nodes
            .iter()
            .map(|node| node.string("symbol_kind"))
            .collect::<HashSet<_>>();
        for expected in [
            "file",
            "database",
            "database_schema",
            "database_table",
            "database_column",
            "database_constraint",
            "database_index",
            "database_view",
            "database_trigger",
            "migration",
            "query",
        ] {
            assert!(kinds.contains(expected), "missing {expected}: {kinds:?}");
        }
        let ids = extraction
            .nodes
            .iter()
            .map(|node| node.id.as_str())
            .collect::<HashSet<_>>();
        assert!(
            extraction.edges.iter().all(
                |edge| ids.contains(edge.source.as_str()) && ids.contains(edge.target.as_str())
            )
        );
        for relation in ["contains", "references", "reads", "writes", "triggers"] {
            assert!(
                extraction
                    .edges
                    .iter()
                    .any(|edge| edge.string("relation") == relation),
                "missing {relation}"
            );
        }
    }

    #[test]
    fn split_members_ignores_commas_inside_types_and_constraints() {
        let value = "id decimal(10, 2), pair text CHECK (pair IN ('a,b', 'c')), name text";
        assert_eq!(split_top_level(value).len(), 3);
    }
}
