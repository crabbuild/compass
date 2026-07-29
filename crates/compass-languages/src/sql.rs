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
const IDENTIFIER: &str = r#"(?:"(?:""|[^"])*"|`(?:``|[^`])*`|\[(?:\]\]|[^\]])+\]|[\w$]+)"#;
const OBJECT_REFERENCE: &str = r#"(?:"(?:""|[^"])*"|`(?:``|[^`])*`|\[(?:\]\]|[^\]])+\]|[\w$]+)(?:\.(?:"(?:""|[^"])*"|`(?:``|[^`])*`|\[(?:\]\]|[^\]])+\]|[\w$]+))*"#;

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
    name_start: usize,
    name_end: usize,
}

#[derive(Clone, Copy, Debug)]
struct Site {
    start: usize,
    end: usize,
}

struct AccessBindings {
    aliases: HashMap<String, String>,
    cte_names: HashSet<String>,
}

impl Site {
    const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }
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
        let mut attributes = self.base_attributes(
            "file",
            label,
            label,
            Site::new(0, self.source.len()),
            "artifact",
            "sql-text-file",
        );
        attributes.insert("file_type".into(), Value::String("code".into()));
        self.push_node(self.file_id.clone(), attributes);
    }

    fn add_database_node(&mut self) {
        let mut attributes = self.base_attributes(
            "database",
            &self.logical_database,
            &self.logical_database,
            Site::new(0, self.source.len()),
            "convention",
            "sql-file-logical-database",
        );
        attributes.insert(
            "logical_database".into(),
            Value::String(self.logical_database.clone()),
        );
        self.push_node(self.database_id.clone(), attributes);
        self.add_edge_with_origin(
            &self.file_id.clone(),
            &self.database_id.clone(),
            "contains",
            Site::new(0, self.source.len()),
            "convention",
            "sql-file-logical-database",
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
        let attributes = self.base_attributes(
            "migration",
            name,
            &qualified_name,
            Site::new(0, self.source.len()),
            "convention",
            "sql-migration-path-convention",
        );
        self.push_node(id.clone(), attributes);
        self.add_edge_with_origin(
            &self.file_id.clone(),
            &id,
            "contains",
            Site::new(0, self.source.len()),
            "convention",
            "sql-migration-path-convention",
        );
    }

    fn add_declared_object(&mut self, statement: &Statement) {
        let site = Site::new(statement.name_start, statement.name_end);
        match statement.kind {
            StatementKind::Database => {
                // The per-file logical database remains the stable owner. An
                // explicit CREATE DATABASE name enriches it without changing
                // the identity of every dependent object in the same file.
                let source_location = format!("L{}", self.line_at(site.start));
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
                        .insert("_origin".into(), Value::String("artifact".into()));
                    node.attributes.insert(
                        "rule".into(),
                        Value::String("sql-text-database-declaration".into()),
                    );
                    crate::facts::stamp_source_range(
                        &mut node.attributes,
                        self.source,
                        site.start,
                        site.end,
                    );
                    node.attributes
                        .insert("source_location".into(), Value::String(source_location));
                }
            }
            StatementKind::Schema => {
                self.ensure_schema(
                    &statement.name,
                    site,
                    "artifact",
                    "sql-text-schema-declaration",
                );
            }
            StatementKind::Table => {
                let id = self.add_database_object(
                    "database_table",
                    &statement.name,
                    site,
                    "artifact",
                    "sql-text-table-declaration",
                );
                self.add_table_members(statement, &id);
            }
            StatementKind::View => {
                self.add_database_object(
                    "database_view",
                    &statement.name,
                    site,
                    "artifact",
                    "sql-text-view-declaration",
                );
            }
            StatementKind::Procedure => {
                self.add_database_object(
                    "database_procedure",
                    &statement.name,
                    site,
                    "artifact",
                    "sql-text-procedure-declaration",
                );
            }
            StatementKind::Trigger => {
                let id = self.add_database_object(
                    "database_trigger",
                    &statement.name,
                    site,
                    "artifact",
                    "sql-text-trigger-declaration",
                );
                let events = trigger_events(
                    &self.masked[statement.offset..statement.end.min(self.masked.len())],
                );
                if !events.is_empty()
                    && let Some(node) = self.extraction.nodes.iter_mut().find(|node| node.id == id)
                {
                    node.attributes.insert(
                        "trigger_events".into(),
                        Value::Array(events.into_iter().map(Value::String).collect()),
                    );
                }
            }
            StatementKind::Index => {
                self.add_database_object(
                    "database_index",
                    &statement.name,
                    site,
                    "artifact",
                    "sql-text-index-declaration",
                );
            }
            StatementKind::AlterTable => {
                self.ensure_table(&statement.name, site);
                self.add_alter_constraints(statement);
            }
        }
    }

    fn add_statement_relationships(&mut self, statement: &Statement) {
        let Some(source) = self.object_id(&statement.name) else {
            return;
        };
        match statement.kind {
            StatementKind::View | StatementKind::Procedure => {
                self.add_data_access(&source, statement.offset, statement.end);
            }
            StatementKind::Trigger => {
                if let Some(target_end) = self.link_trigger_to_table(statement, &source)
                    && let Some(body_start) = trigger_body_start(
                        &self.masked,
                        target_end,
                        statement.end.min(self.masked.len()),
                    )
                {
                    self.add_data_access(&source, body_start, statement.end);
                }
            }
            StatementKind::Table | StatementKind::AlterTable => {
                self.add_foreign_key_references(&source, statement.offset, statement.end);
            }
            StatementKind::Index => self.link_index_to_table(statement, &source),
            StatementKind::Database | StatementKind::Schema => {}
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
            if is_table_constraint(trimmed) {
                let (name, site) = constraint_name(trimmed).map_or_else(
                    || {
                        (
                            format!("{}#constraint@{}", statement.name, member_offset),
                            Site::new(member_offset, member_offset + trimmed.len()),
                        )
                    },
                    |(name, start, end)| {
                        (name, Site::new(member_offset + start, member_offset + end))
                    },
                );
                let qualified_name = format!("{}::{name}", statement.name);
                let identity = identifier_key(&format!("{}.{}", statement.name, name));
                let id = make_id(&[
                    "sql-constraint",
                    &self.logical_database,
                    &qualified_name,
                    &identity,
                ]);
                let attributes = self.database_attributes(
                    "database_constraint",
                    &name,
                    &qualified_name,
                    site,
                    "artifact",
                    "sql-text-table-constraint",
                );
                self.push_node(id.clone(), attributes);
                self.add_edge(table_id, &id, "contains", site, "sql-text-table-member");
                continue;
            }
            let Some((name, start, end)) = first_identifier(trimmed) else {
                continue;
            };
            let site = Site::new(member_offset + start, member_offset + end);
            let qualified_name = format!("{}.{}", statement.name, name);
            let identity = identifier_key(&qualified_name);
            let id = make_id(&[
                "sql-column",
                &self.logical_database,
                &qualified_name,
                &identity,
            ]);
            let attributes = self.database_attributes(
                "database_column",
                &name,
                &qualified_name,
                site,
                "artifact",
                "sql-text-column-declaration",
            );
            self.push_node(id.clone(), attributes);
            self.add_edge(table_id, &id, "contains", site, "sql-text-table-member");
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
            let site = Site::new(
                statement.offset + name_match.start(),
                statement.offset + name_match.end(),
            );
            let qualified_name = format!("{}::{name}", statement.name);
            let identity = identifier_key(&format!("{}.{}", statement.name, name));
            let id = make_id(&[
                "sql-constraint",
                &self.logical_database,
                &qualified_name,
                &identity,
            ]);
            let attributes = self.database_attributes(
                "database_constraint",
                name,
                &qualified_name,
                site,
                "artifact",
                "sql-text-alter-constraint",
            );
            self.push_node(id.clone(), attributes);
            self.add_edge(&table_id, &id, "contains", site, "sql-text-table-member");
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
            let site = Site::new(start + name_match.start(), start + name_match.end());
            let target = self.ensure_table(name_match.as_str(), site);
            self.add_edge(
                source,
                &target,
                "references",
                site,
                "sql-text-foreign-key-reference",
            );
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
        let site = Site::new(
            statement.offset + name_match.start(),
            statement.offset + name_match.end(),
        );
        let table = self.ensure_table(name_match.as_str(), site);
        self.add_edge(&table, index_id, "contains", site, "sql-text-index-target");
    }

    fn link_trigger_to_table(&mut self, statement: &Statement, trigger_id: &str) -> Option<usize> {
        let body = self.masked[statement.offset..statement.end].to_owned();
        let Ok(regex) = Regex::new(&format!(r"(?i)\bON\s+({OBJECT_REFERENCE})")) else {
            return None;
        };
        let capture = regex.captures(&body)?;
        let name_match = capture.get(1)?;
        let site = Site::new(
            statement.offset + name_match.start(),
            statement.offset + name_match.end(),
        );
        let table = self.ensure_table(name_match.as_str(), site);
        self.add_edge(
            &table,
            trigger_id,
            "contains",
            site,
            "sql-text-trigger-target",
        );
        self.add_edge(
            trigger_id,
            &table,
            "triggers",
            site,
            "sql-text-trigger-target",
        );
        Some(site.end)
    }

    fn add_queries(&mut self) {
        for (statement_start, _, end, operation) in query_statements(&self.masked) {
            self.add_query(&operation, statement_start, end);
        }
    }

    fn add_query(&mut self, operation: &str, statement_start: usize, end: usize) {
        let qualified_name = format!(
            "{}::{}@{}",
            self.logical_database, operation, statement_start
        );
        let id = make_id(&["sql-query", &self.source_file, &statement_start.to_string()]);
        let site = Site::new(statement_start, end);
        let mut attributes = self.base_attributes(
            "query",
            operation,
            &qualified_name,
            site,
            "artifact",
            "sql-text-query-statement",
        );
        attributes.insert("dialect".into(), Value::String("sql".into()));
        attributes.insert("operation".into(), Value::String(operation.to_owned()));
        attributes.insert(
            "text_digest".into(),
            Value::String(sha256_prefixed(
                self.source.get(statement_start..end).unwrap_or_default(),
            )),
        );
        self.push_node(id.clone(), attributes);
        self.add_edge(
            &self.file_id.clone(),
            &id,
            "contains",
            site,
            "sql-text-query-statement",
        );
        self.add_data_access(&id, statement_start, end);
    }

    fn add_data_access(&mut self, source: &str, start: usize, end: usize) {
        let body = self.masked[start..end.min(self.masked.len())].to_owned();
        let bindings = AccessBindings {
            aliases: table_aliases(&body),
            cte_names: cte_names(&body),
        };
        let read_patterns = [format!(
            r"(?i)\b(?:FROM|JOIN|USING)\s+(?:ONLY\s+)?({OBJECT_REFERENCE})"
        )];
        let write_patterns = [
            format!(
                r"(?i)\bINSERT\s+(?:OR\s+(?:ABORT|FAIL|IGNORE|REPLACE|ROLLBACK)\s+)?INTO\s+({OBJECT_REFERENCE})"
            ),
            format!(r"(?i)\bUPDATE\s+(?:ONLY\s+)?({OBJECT_REFERENCE})"),
            format!(r"(?i)\bDELETE\s+(?:{IDENTIFIER}\s+)?FROM\s+(?:ONLY\s+)?({OBJECT_REFERENCE})"),
            format!(r"(?i)\bMERGE\s+(?:INTO\s+)?({OBJECT_REFERENCE})"),
            format!(r"(?i)\bSELECT\b[\s\S]*?\bINTO\s+({OBJECT_REFERENCE})"),
        ];
        self.add_access_matches(source, start, &body, &read_patterns, "reads", &bindings);
        self.add_access_matches(source, start, &body, &write_patterns, "writes", &bindings);
    }

    fn add_access_matches(
        &mut self,
        source: &str,
        start: usize,
        body: &str,
        patterns: &[String],
        relation: &str,
        bindings: &AccessBindings,
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
                let key = identifier_key(name);
                if is_non_table_keyword(name) || bindings.cte_names.contains(&key) {
                    continue;
                }
                let target_name = bindings.aliases.get(&key).map_or(name, String::as_str);
                if is_non_table_keyword(target_name) || !emitted.insert(identifier_key(target_name))
                {
                    continue;
                }
                let site = Site::new(start + name_match.start(), start + name_match.end());
                let target = self.ensure_table(target_name, site);
                self.add_edge(source, &target, relation, site, "sql-text-data-access");
            }
        }
    }

    fn add_database_object(
        &mut self,
        kind: &str,
        name: &str,
        site: Site,
        origin: &str,
        rule: &str,
    ) -> String {
        if let Some(existing) = self.exact_object_id(name) {
            return existing;
        }
        let schema_source = schema_reference(name);
        let schema = schema_source.as_deref().map(normalized_identifier);
        let qualified_name = normalized_identifier(name);
        let identity = identifier_key(name);
        let id = make_id(&[
            kind,
            &self.logical_database,
            schema.as_deref().unwrap_or_default(),
            &qualified_name,
            &identity,
        ]);
        let attributes = self.database_attributes(kind, name, &qualified_name, site, origin, rule);
        self.push_node(id.clone(), attributes);
        self.register_object(name, &id);
        let parent = schema_source
            .as_deref()
            .map(|schema| self.ensure_schema(schema, site, origin, "sql-text-schema-qualification"))
            .unwrap_or_else(|| self.database_id.clone());
        self.add_edge(&parent, &id, "contains", site, "sql-text-containment");
        id
    }

    fn ensure_table(&mut self, name: &str, site: Site) -> String {
        self.object_id(name).unwrap_or_else(|| {
            self.add_database_object(
                "database_table",
                name,
                site,
                "artifact",
                "sql-text-table-reference",
            )
        })
    }

    fn ensure_schema(&mut self, name: &str, site: Site, origin: &str, rule: &str) -> String {
        let normalized = normalized_identifier(name);
        let key = identifier_key(name);
        if let Some(id) = self.schemas.get(&key) {
            return id.clone();
        }
        let id = make_id(&["database_schema", &self.logical_database, &normalized, &key]);
        let mut attributes =
            self.database_attributes("database_schema", name, &normalized, site, origin, rule);
        attributes.insert("database_schema".into(), Value::String(normalized.clone()));
        self.push_node(id.clone(), attributes);
        self.schemas.insert(key, id.clone());
        self.add_edge(
            &self.database_id.clone(),
            &id,
            "contains",
            site,
            "sql-text-containment",
        );
        id
    }

    fn exact_object_id(&self, name: &str) -> Option<String> {
        self.objects.get(&identifier_key(name)).cloned()
    }

    fn object_id(&self, name: &str) -> Option<String> {
        self.exact_object_id(name).or_else(|| {
            if qualified_identifier_parts(name).len() > 1 {
                return None;
            }
            let short = last_identifier(name).to_ascii_lowercase();
            self.short_object_names.get(&short).cloned().flatten()
        })
    }

    fn register_object(&mut self, name: &str, id: &str) {
        self.objects.insert(identifier_key(name), id.to_owned());
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
        site: Site,
        origin: &str,
        rule: &str,
    ) -> Map<String, Value> {
        let mut attributes = self.base_attributes(
            kind,
            &last_identifier(name),
            qualified_name,
            site,
            origin,
            rule,
        );
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
        site: Site,
        origin: &str,
        rule: &str,
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
        attributes.insert("_origin".into(), Value::String(origin.to_owned()));
        attributes.insert("rule".into(), Value::String(rule.to_owned()));
        attributes.insert(
            "extractor".into(),
            Value::String("compass.languages.sql".into()),
        );
        crate::facts::stamp_source_range(&mut attributes, self.source, site.start, site.end);
        attributes.insert(
            "source_location".into(),
            Value::String(format!("L{}", self.line_at(site.start))),
        );
        attributes
    }

    fn push_node(&mut self, id: String, attributes: Map<String, Value>) {
        if self.seen_nodes.insert(id.clone()) {
            self.extraction.nodes.push(NodeRecord { id, attributes });
        }
    }

    fn add_edge(&mut self, source: &str, target: &str, relation: &str, site: Site, rule: &str) {
        self.add_edge_with_origin(source, target, relation, site, "artifact", rule);
    }

    fn add_edge_with_origin(
        &mut self,
        source: &str,
        target: &str,
        relation: &str,
        site: Site,
        origin: &str,
        rule: &str,
    ) {
        if !self.seen_edges.insert((
            source.to_owned(),
            target.to_owned(),
            relation.to_owned(),
            site.start,
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
        attributes.insert(
            "source_location".into(),
            Value::String(format!("L{}", self.line_at(site.start))),
        );
        attributes.insert("weight".into(), Value::from(1.0));
        attributes.insert("_origin".into(), Value::String(origin.to_owned()));
        attributes.insert(
            "extractor".into(),
            Value::String("compass.languages.sql".into()),
        );
        attributes.insert("rule".into(), Value::String(rule.to_owned()));
        crate::facts::stamp_source_range(&mut attributes, self.source, site.start, site.end);
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
            let name = capture.get(4)?;
            Some(Statement {
                offset: full.start(),
                end: 0,
                kind,
                name: name.as_str().to_owned(),
                name_start: name.start(),
                name_end: name.end(),
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

fn query_statements(source: &str) -> Vec<(usize, usize, usize, String)> {
    let Ok(operation_pattern) = Regex::new(r"(?i)^(SELECT|INSERT|UPDATE|DELETE|MERGE)\b") else {
        return Vec::new();
    };
    let mut output = Vec::new();
    let mut segment_start = 0;
    for segment_end in source
        .match_indices(';')
        .map(|(index, delimiter)| index + delimiter.len())
        .chain(std::iter::once(source.len()))
    {
        let Some(segment) = source.get(segment_start..segment_end) else {
            segment_start = segment_end;
            continue;
        };
        let leading = segment.len() - segment.trim_start().len();
        let statement_start = segment_start + leading;
        let statement = segment.trim_start();
        let operation = operation_pattern
            .captures(statement)
            .and_then(|capture| capture.get(1))
            .map(|operation| {
                (
                    statement_start + operation.start(),
                    operation.as_str().to_ascii_lowercase(),
                )
            })
            .or_else(|| {
                statement
                    .get(..4)
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case("WITH"))
                    .then(|| top_level_cte_operation(statement))
                    .flatten()
                    .map(|(offset, operation)| (statement_start + offset, operation))
            });
        if let Some((operation_start, operation)) = operation {
            output.push((statement_start, operation_start, segment_end, operation));
        }
        segment_start = segment_end;
    }
    output
}

fn top_level_cte_operation(statement: &str) -> Option<(usize, String)> {
    let bytes = statement.as_bytes();
    let mut index = 0;
    let mut depth = 0_u32;
    let mut completed_cte = false;
    while index < bytes.len() {
        match bytes[index] {
            b'(' => {
                depth = depth.saturating_add(1);
                index += 1;
            }
            b')' => {
                depth = depth.saturating_sub(1);
                completed_cte |= depth == 0;
                index += 1;
            }
            byte if byte.is_ascii_alphabetic() || byte == b'_' => {
                let start = index;
                index += 1;
                while bytes
                    .get(index)
                    .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
                {
                    index += 1;
                }
                let token = &statement[start..index];
                if depth == 0
                    && completed_cte
                    && matches!(
                        token.to_ascii_uppercase().as_str(),
                        "SELECT" | "INSERT" | "UPDATE" | "DELETE" | "MERGE"
                    )
                {
                    return Some((start, token.to_ascii_lowercase()));
                }
            }
            _ => index += 1,
        }
    }
    None
}

fn cte_names(statement: &str) -> HashSet<String> {
    let Ok(pattern) = Regex::new(&format!(
        r"(?i)(?:\bWITH\b|,)\s*({IDENTIFIER})(?:\s*\([^)]*\))?\s+AS\s*\("
    )) else {
        return HashSet::new();
    };
    pattern
        .captures_iter(statement)
        .filter_map(|capture| capture.get(1))
        .map(|name| identifier_key(name.as_str()))
        .collect()
}

fn table_aliases(statement: &str) -> HashMap<String, String> {
    let Ok(pattern) = Regex::new(&format!(
        r"(?i)\b(?:FROM|JOIN|USING)\s+(?:ONLY\s+)?({OBJECT_REFERENCE})(?:\s+(?:AS\s+)?({IDENTIFIER}))?"
    )) else {
        return HashMap::new();
    };
    pattern
        .captures_iter(statement)
        .filter_map(|capture| {
            let target = capture.get(1)?.as_str();
            let alias = capture.get(2)?.as_str();
            (!is_non_table_keyword(alias)).then(|| (identifier_key(alias), target.to_owned()))
        })
        .collect()
}

fn trigger_events(statement: &str) -> Vec<String> {
    let Ok(start_pattern) = Regex::new(r"(?i)\b(?:BEFORE|AFTER|INSTEAD\s+OF)\b") else {
        return Vec::new();
    };
    let Some(start) = start_pattern.find(statement) else {
        return Vec::new();
    };
    let tail = &statement[start.end()..];
    let Ok(boundary_pattern) = Regex::new(&format!(
        r"(?i)\b(?:ON\s+{OBJECT_REFERENCE}|AS|BEGIN|EXECUTE)\b"
    )) else {
        return Vec::new();
    };
    let boundary = boundary_pattern
        .find(tail)
        .map_or(tail.len(), |item| item.start());
    let Ok(event_pattern) = Regex::new(r"(?i)\b(INSERT|UPDATE|DELETE|TRUNCATE)\b") else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    event_pattern
        .captures_iter(&tail[..boundary])
        .filter_map(|capture| {
            let event = capture.get(1)?.as_str().to_ascii_lowercase();
            seen.insert(event.clone()).then_some(event)
        })
        .collect()
}

fn trigger_body_start(source: &str, start: usize, end: usize) -> Option<usize> {
    let tail = source.get(start..end)?;
    let marker = Regex::new(r"(?i)\b(?:FOR\s+EACH\s+ROW|AS|BEGIN)\b").ok()?;
    marker.find(tail).map(|matched| start + matched.end())
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

fn constraint_name(value: &str) -> Option<(String, usize, usize)> {
    let regex = Regex::new(&format!(r"(?i)^CONSTRAINT\s+({IDENTIFIER})")).ok()?;
    let trimmed = value.trim_start();
    let trim_offset = value.len() - trimmed.len();
    let name = regex.captures(trimmed)?.get(1)?;
    Some((
        name.as_str().to_owned(),
        trim_offset + name.start(),
        trim_offset + name.end(),
    ))
}

fn first_identifier(value: &str) -> Option<(String, usize, usize)> {
    let regex = Regex::new(&format!(r"^({IDENTIFIER})")).ok()?;
    let trimmed = value.trim_start();
    let trim_offset = value.len() - trimmed.len();
    let name = regex.captures(trimmed)?.get(1)?;
    Some((
        name.as_str().to_owned(),
        trim_offset + name.start(),
        trim_offset + name.end(),
    ))
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
    schema_reference(name).map(|schema| normalized_identifier(&schema))
}

fn schema_reference(name: &str) -> Option<String> {
    let mut parts = qualified_identifier_parts(name);
    (parts.len() > 1).then(|| {
        parts.pop();
        parts.join(".")
    })
}

fn last_identifier(name: &str) -> String {
    qualified_identifier_parts(name)
        .last()
        .map(|part| normalize_identifier_part(part))
        .unwrap_or_default()
}

fn normalized_identifier(name: &str) -> String {
    qualified_identifier_parts(name)
        .into_iter()
        .map(normalize_identifier_part)
        .collect::<Vec<_>>()
        .join(".")
}

fn identifier_key(name: &str) -> String {
    qualified_identifier_parts(name)
        .into_iter()
        .map(|part| normalize_identifier_part(part).to_lowercase())
        .map(|part| format!("{}:{part}", part.len()))
        .collect::<Vec<_>>()
        .join("|")
}

fn qualified_identifier_parts(name: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut delimiter = None;
    let mut characters = name.char_indices().peekable();
    while let Some((index, character)) = characters.next() {
        match delimiter {
            Some(']') if character == ']' => {
                if characters.peek().is_some_and(|(_, next)| *next == ']') {
                    characters.next();
                } else {
                    delimiter = None;
                }
            }
            Some(quote) if character == quote => {
                if characters.peek().is_some_and(|(_, next)| *next == quote) {
                    characters.next();
                } else {
                    delimiter = None;
                }
            }
            Some(_) => {}
            None if character == '"' || character == '`' => delimiter = Some(character),
            None if character == '[' => delimiter = Some(']'),
            None if character == '.' => {
                parts.push(&name[start..index]);
                start = index + character.len_utf8();
            }
            None => {}
        }
    }
    parts.push(&name[start..]);
    parts
}

fn normalize_identifier_part(part: &str) -> String {
    let trimmed = part.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        trimmed[1..trimmed.len() - 1].replace("\"\"", "\"")
    } else if trimmed.len() >= 2 && trimmed.starts_with('`') && trimmed.ends_with('`') {
        trimmed[1..trimmed.len() - 1].replace("``", "`")
    } else if trimmed.len() >= 2 && trimmed.starts_with('[') && trimmed.ends_with(']') {
        trimmed[1..trimmed.len() - 1].replace("]]", "]")
    } else {
        trimmed.to_owned()
    }
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
            | "on"
            | "of"
            | "into"
            | "as"
            | "when"
            | "matched"
            | "then"
            | "update"
            | "insert"
            | "delete"
            | "merge"
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
