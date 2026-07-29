use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::{Extraction, RawEdgeRecord as EdgeRecord, RawNodeRecord as NodeRecord};
use regex::Regex;
use serde_json::{Map, Value, json};
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
    body: Option<Site>,
    incomplete_body_reason: Option<String>,
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

struct BodyBoundary {
    end: usize,
    body: Option<Site>,
    incomplete_reason: Option<String>,
}

#[derive(Clone, Debug)]
struct BoundaryIssue {
    site: Site,
    reason: String,
}

struct ScannedStatement {
    site: Site,
    issue: Option<BoundaryIssue>,
}

type QueryStatement = (usize, usize, usize, String);

enum StatementBoundary {
    Complete(usize),
    EndOfInput(usize),
    IncompleteQuoted { end: usize, issue: BoundaryIssue },
}

impl StatementBoundary {
    const fn end(&self) -> usize {
        match self {
            Self::Complete(end) | Self::EndOfInput(end) | Self::IncompleteQuoted { end, .. } => {
                *end
            }
        }
    }

    fn into_issue(self) -> Option<BoundaryIssue> {
        match self {
            Self::IncompleteQuoted { issue, .. } => Some(issue),
            Self::Complete(_) | Self::EndOfInput(_) => None,
        }
    }

    const fn recovery_end(&self, fallback: usize) -> usize {
        match self {
            Self::EndOfInput(_) => fallback,
            Self::Complete(end) | Self::IncompleteQuoted { end, .. } => *end,
        }
    }
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
            self.add_incomplete_body_evidence(statement);
        }
        for statement in &declarations {
            self.add_statement_relationships(statement);
        }
        self.add_queries(&declarations);
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
            StatementKind::View => {
                self.add_data_access_statement(&source, statement.offset, statement.end);
            }
            StatementKind::Procedure => {
                if statement.incomplete_body_reason.is_some() {
                    return;
                }
                if let Some(body) = statement.body {
                    self.add_data_access_statements(&source, body.start, body.end);
                } else {
                    self.add_data_access_statement(&source, statement.offset, statement.end);
                }
            }
            StatementKind::Trigger => {
                self.link_trigger_to_table(statement, &source);
                if statement.incomplete_body_reason.is_some() {
                    return;
                }
                if let Some(body) = statement.body {
                    self.add_data_access_statements(&source, body.start, body.end);
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
                let identity_digest = sha256_prefixed(identity.as_bytes());
                let id = make_id(&[
                    "sql-constraint",
                    &self.logical_database,
                    &qualified_name,
                    &identity,
                    &identity_digest,
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
            let identity_digest = sha256_prefixed(identity.as_bytes());
            let id = make_id(&[
                "sql-column",
                &self.logical_database,
                &qualified_name,
                &identity,
                &identity_digest,
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
            let identity_digest = sha256_prefixed(identity.as_bytes());
            let id = make_id(&[
                "sql-constraint",
                &self.logical_database,
                &qualified_name,
                &identity,
                &identity_digest,
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
            if is_reserved_object_reference(name_match.as_str()) {
                continue;
            }
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
        let Ok(regex) = Regex::new(&format!(r"(?i)\bON\s+(?:ONLY\s+)?({OBJECT_REFERENCE})")) else {
            return;
        };
        let Some(capture) = regex.captures(&body) else {
            return;
        };
        let Some(name_match) = capture.get(1) else {
            return;
        };
        if is_reserved_object_reference(name_match.as_str()) {
            return;
        }
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
        if is_reserved_object_reference(name_match.as_str()) {
            return None;
        }
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

    fn add_queries(&mut self, declarations: &[Statement]) {
        let (queries, issues) = query_statements(&self.masked, declarations);
        for issue in &issues {
            self.add_boundary_issue_evidence(issue);
        }
        for (statement_start, _, end, operation) in queries {
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
        self.add_data_access_statement(&id, statement_start, end);
    }

    fn add_data_access_statements(&mut self, source: &str, start: usize, end: usize) {
        for statement in scan_statement_ranges(&self.masked, start, end) {
            if let Some(issue) = statement.issue {
                self.add_boundary_issue_evidence(&issue);
                continue;
            }
            self.add_data_access_statement(source, statement.site.start, statement.site.end);
        }
    }

    fn add_data_access_statement(&mut self, source: &str, start: usize, end: usize) {
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
                r"(?i)\bINSERT\s+(?:(?:OR\s+(?:ABORT|FAIL|IGNORE|REPLACE|ROLLBACK)|(?:LOW_PRIORITY|DELAYED|HIGH_PRIORITY)(?:\s+IGNORE)?|IGNORE)\s+)?(?:INTO\s+)?({OBJECT_REFERENCE})"
            ),
            format!(
                r"(?i)\bUPDATE\s+(?:LOW_PRIORITY\s+)?(?:IGNORE\s+)?(?:ONLY\s+)?({OBJECT_REFERENCE})"
            ),
            format!(
                r"(?i)\bDELETE\s+(?:LOW_PRIORITY\s+)?(?:QUICK\s+)?(?:IGNORE\s+)?(?:{IDENTIFIER}\s+)?FROM\s+(?:ONLY\s+)?({OBJECT_REFERENCE})"
            ),
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
                if is_reserved_object_reference(name) || bindings.cte_names.contains(&key) {
                    continue;
                }
                let target_name = bindings.aliases.get(&key).map_or(name, String::as_str);
                if is_reserved_object_reference(target_name)
                    || !emitted.insert((identifier_key(target_name), name_match.start()))
                {
                    continue;
                }
                let site = Site::new(start + name_match.start(), start + name_match.end());
                let target = self.ensure_table(target_name, site);
                self.add_edge(source, &target, relation, site, "sql-text-data-access");
            }
        }
    }

    fn add_incomplete_body_evidence(&mut self, statement: &Statement) {
        let Some(reason) = statement.incomplete_body_reason.as_deref() else {
            return;
        };
        let site = Site::new(statement.offset, statement.end);
        let anchor = self.source_anchor(site);
        let coverage = json!({
            "capability": "sql:body_ownership",
            "producer": "compass.languages.sql",
            "status": "partial",
            "reason": reason,
            "anchor": anchor.clone(),
        });
        self.push_extension_record("_compass_v1_graph_coverage", coverage);
        let diagnostic = json!({
            "severity": "warning",
            "code": "sql_incomplete_body_boundary",
            "message": reason,
            "anchor": anchor,
        });
        self.push_extension_record("_compass_v1_graph_diagnostics", diagnostic);
    }

    fn add_boundary_issue_evidence(&mut self, issue: &BoundaryIssue) {
        let anchor = self.source_anchor(issue.site);
        self.push_extension_record(
            "_compass_v1_graph_coverage",
            json!({
                "capability": "sql:statement_boundary",
                "producer": "compass.languages.sql",
                "status": "partial",
                "reason": issue.reason,
                "anchor": anchor.clone(),
            }),
        );
        self.push_extension_record(
            "_compass_v1_graph_diagnostics",
            json!({
                "severity": "warning",
                "code": "sql_incomplete_quoted_identifier",
                "message": issue.reason,
                "anchor": anchor,
            }),
        );
    }

    fn push_extension_record(&mut self, key: &str, record: Value) {
        match self.extraction.extensions.get_mut(key) {
            Some(Value::Array(records)) => records.push(record),
            Some(_) => {}
            None => {
                self.extraction
                    .extensions
                    .insert(key.to_owned(), Value::Array(vec![record]));
            }
        }
    }

    fn source_anchor(&self, site: Site) -> Value {
        let mut range = Map::new();
        crate::facts::stamp_source_range(&mut range, self.source, site.start, site.end);
        json!({
            "file": self.source_file,
            "startByte": range.get("start_byte").cloned().unwrap_or(Value::from(0)),
            "endByte": range.get("end_byte").cloned().unwrap_or(Value::from(0)),
            "startLine": range.get("line_start").cloned().unwrap_or(Value::from(1)),
            "startColumn": range.get("column_start").cloned().unwrap_or(Value::from(0)),
            "endLine": range.get("line_end").cloned().unwrap_or(Value::from(1)),
            "endColumn": range.get("column_end").cloned().unwrap_or(Value::from(0)),
        })
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
        let identity_digest = sha256_prefixed(identity.as_bytes());
        let id = make_id(&[
            kind,
            &self.logical_database,
            schema.as_deref().unwrap_or_default(),
            &qualified_name,
            &identity,
            &identity_digest,
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
        let key_digest = sha256_prefixed(key.as_bytes());
        let id = make_id(&[
            "database_schema",
            &self.logical_database,
            &normalized,
            &key,
            &key_digest,
        ]);
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
            let short = short_identifier_key(name);
            self.short_object_names.get(&short).cloned().flatten()
        })
    }

    fn register_object(&mut self, name: &str, id: &str) {
        self.objects.insert(identifier_key(name), id.to_owned());
        let short = short_identifier_key(name);
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
        r"(?ix)\b(?:
            CREATE\s+
                (?:OR\s+(?:REPLACE|ALTER)\s+)?
                (?:(?:GLOBAL|LOCAL)\s+)?
                (?:(?:TEMP|TEMPORARY|UNLOGGED)\s+)?
                (?:MATERIALIZED\s+)?
                (DATABASE|SCHEMA|TABLE|VIEW|FUNCTION|PROCEDURE|TRIGGER)
                \s+(?:IF\s+NOT\s+EXISTS\s+)?
          | CREATE\s+(?:UNIQUE\s+)?(INDEX)(?:\s+CONCURRENTLY)?
                \s+(?:IF\s+NOT\s+EXISTS\s+)?
          | ((?:ALTER)\s+TABLE)\s+(?:IF\s+EXISTS\s+)?
        )({OBJECT_REFERENCE})"
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
            if is_reserved_object_reference(name.as_str()) {
                return None;
            }
            Some(Statement {
                offset: full.start(),
                end: 0,
                kind,
                name: name.as_str().to_owned(),
                name_start: name.start(),
                name_end: name.end(),
                body: None,
                incomplete_body_reason: None,
            })
        })
        .collect::<Vec<_>>();
    declarations.sort_by_key(|statement| statement.offset);
    for index in 0..declarations.len() {
        let next = declarations
            .get(index + 1)
            .map_or(source.len(), |statement| statement.offset);
        let boundary = body_statement_boundary(
            source,
            declarations[index].offset,
            &declarations[index].kind,
        );
        if let Some(boundary) = boundary {
            declarations[index].end = boundary.end;
            declarations[index].body = boundary.body;
            declarations[index].incomplete_body_reason = boundary.incomplete_reason;
        } else {
            declarations[index].end =
                scan_statement_boundary(source, declarations[index].offset, next).end();
        }
    }
    let mut top_level = Vec::<Statement>::new();
    for declaration in declarations {
        if top_level
            .last()
            .is_some_and(|owner| declaration.offset < owner.end)
        {
            continue;
        }
        top_level.push(declaration);
    }
    top_level
}

fn body_statement_boundary(
    source: &str,
    start: usize,
    kind: &StatementKind,
) -> Option<BodyBoundary> {
    if !matches!(kind, StatementKind::Procedure | StatementKind::Trigger) {
        return None;
    }
    match routine_body_dollar_opener(source, start) {
        DollarBodySearch::Found(open) => {
            let delimiter = dollar_quote_delimiter_at(source, open)?;
            let content_start = open + delimiter.len();
            if let Some(close_start) = source[content_start..]
                .find(delimiter)
                .map(|offset| content_start + offset)
            {
                let close_end = close_start + delimiter.len();
                let end = scan_statement_boundary(source, close_end, source.len()).end();
                return Some(BodyBoundary {
                    end,
                    body: Some(Site::new(content_start, close_start)),
                    incomplete_reason: None,
                });
            }
            let boundary = scan_statement_boundary(source, start, source.len());
            let end = boundary.recovery_end(first_line_end(source, open).min(source.len()));
            return Some(BodyBoundary {
                end,
                body: None,
                incomplete_reason: Some(format!(
                    "unterminated SQL routine dollar delimiter {delimiter:?}; body ownership omitted"
                )),
            });
        }
        DollarBodySearch::Ambiguous(site) => {
            let boundary = scan_statement_boundary(source, start, source.len());
            let end = boundary.recovery_end(first_line_end(source, site.start).min(source.len()));
            return Some(BodyBoundary {
                end,
                body: None,
                incomplete_reason: Some(
                    "ambiguous SQL routine dollar body position; body ownership omitted".to_owned(),
                ),
            });
        }
        DollarBodySearch::None => {}
    }
    compound_body_boundary(source, start)
}

fn compound_body_boundary(source: &str, start: usize) -> Option<BodyBoundary> {
    let words = sql_word_spans(source, start);
    let header_end = first_unquoted_semicolon(source, start).unwrap_or(source.len());
    let begin_index = words.iter().position(|(word_start, _, word)| {
        *word_start < header_end && word.eq_ignore_ascii_case("BEGIN")
    })?;
    let body_start = words[begin_index].1;
    let mut depth = 0_u32;
    let mut case_depth = 0_u32;
    let mut skipped_case_suffix = None;
    for (position, (word_start, end, word)) in words.iter().enumerate().skip(begin_index) {
        if skipped_case_suffix == Some(position) {
            continue;
        }
        if word.eq_ignore_ascii_case("BEGIN") {
            depth = depth.saturating_add(1);
            continue;
        }
        if word.eq_ignore_ascii_case("CASE") {
            case_depth = case_depth.saturating_add(1);
            continue;
        }
        if !word.eq_ignore_ascii_case("END") {
            continue;
        }
        let closer = words
            .get(position + 1)
            .map(|(_, _, next)| next.to_ascii_uppercase());
        if closer.as_deref() == Some("CASE") {
            case_depth = case_depth.saturating_sub(1);
            skipped_case_suffix = Some(position + 1);
            continue;
        }
        let qualified_closer = closer
            .as_deref()
            .is_some_and(|next| matches!(next, "IF" | "LOOP" | "WHILE" | "REPEAT"));
        if qualified_closer {
            continue;
        }
        if case_depth > 0 {
            case_depth -= 1;
            continue;
        }
        depth = depth.saturating_sub(1);
        if depth == 0 {
            return Some(BodyBoundary {
                end: scan_statement_boundary(source, *end, source.len()).end(),
                body: Some(Site::new(body_start, *word_start)),
                incomplete_reason: None,
            });
        }
    }
    let boundary = scan_statement_boundary(source, start, source.len());
    let end = boundary.recovery_end(first_line_end(source, body_start).min(source.len()));
    Some(BodyBoundary {
        end,
        body: None,
        incomplete_reason: Some(
            "unterminated SQL compound routine body; body ownership omitted".to_owned(),
        ),
    })
}

fn sql_word_spans(source: &str, start: usize) -> Vec<(usize, usize, &str)> {
    let bytes = source.as_bytes();
    let mut words = Vec::new();
    let mut index = start;
    while index < bytes.len() {
        if let Some(delimiter) = identifier_delimiter(bytes[index]) {
            index = quoted_identifier_end(bytes, index, delimiter).unwrap_or(bytes.len());
            continue;
        }
        if let Some(end) = skip_dollar_quote(source, index) {
            index = end;
            continue;
        }
        if bytes[index].is_ascii_alphabetic() || bytes[index] == b'_' {
            let word_start = index;
            index += 1;
            while bytes
                .get(index)
                .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
            {
                index += 1;
            }
            words.push((word_start, index, &source[word_start..index]));
            continue;
        }
        index += 1;
    }
    words
}

fn scan_statement_ranges(source: &str, start: usize, end: usize) -> Vec<ScannedStatement> {
    let mut output = Vec::new();
    let mut cursor = start.min(source.len());
    let limit = end.min(source.len());
    while cursor < limit {
        let boundary = scan_statement_boundary(source, cursor, limit);
        let statement_end = boundary.end();
        let issue = boundary.into_issue();
        if source[cursor..statement_end]
            .bytes()
            .any(|byte| !byte.is_ascii_whitespace())
        {
            output.push(ScannedStatement {
                site: Site::new(cursor, statement_end),
                issue,
            });
        }
        if statement_end <= cursor {
            break;
        }
        cursor = statement_end;
    }
    output
}

fn scan_statement_boundary(source: &str, start: usize, end: usize) -> StatementBoundary {
    let bytes = source.as_bytes();
    let limit = end.min(bytes.len());
    let mut index = start.min(limit);
    while index < limit {
        if let Some(delimiter) = identifier_delimiter(bytes[index]) {
            if let Some(quoted_end) =
                quoted_identifier_end(bytes, index, delimiter).filter(|end| *end <= limit)
            {
                index = quoted_end;
                continue;
            }
            let recovery_end = incomplete_quote_recovery_end(source, index, limit);
            let style = match bytes[index] {
                b'"' => "double-quoted",
                b'`' => "backtick-quoted",
                b'[' => "bracket-quoted",
                _ => "quoted",
            };
            return StatementBoundary::IncompleteQuoted {
                end: recovery_end,
                issue: BoundaryIssue {
                    site: Site::new(index, recovery_end),
                    reason: format!("unterminated {style} SQL identifier; statement facts omitted"),
                },
            };
        }
        if let Some(dollar_end) = paired_dollar_quote_end(source, index, limit) {
            index = dollar_end;
            continue;
        }
        if bytes[index] == b';' {
            return StatementBoundary::Complete(index + 1);
        }
        index += 1;
    }
    StatementBoundary::EndOfInput(limit)
}

fn incomplete_quote_recovery_end(source: &str, open: usize, limit: usize) -> usize {
    let start = open.saturating_add(1).min(limit);
    let bytes = source.as_bytes();
    for (offset, byte) in bytes[start..limit].iter().enumerate() {
        if *byte == b';' || *byte == b'\n' {
            return start + offset + 1;
        }
    }
    limit
}

fn top_level_statement_ranges(source: &str, declarations: &[Statement]) -> Vec<ScannedStatement> {
    let mut output = Vec::new();
    let mut cursor = 0;
    while cursor < source.len() {
        let statement_start = skip_sql_whitespace(source, cursor);
        if let Some(declaration) = declarations
            .iter()
            .find(|declaration| declaration.offset == statement_start)
        {
            let end = declaration.end.min(source.len());
            output.push(ScannedStatement {
                site: Site::new(cursor, end),
                issue: None,
            });
            cursor = end;
            continue;
        }
        let boundary = scan_statement_boundary(source, cursor, source.len());
        let end = boundary.end();
        let issue = boundary.into_issue();
        if end <= cursor {
            break;
        }
        output.push(ScannedStatement {
            site: Site::new(cursor, end),
            issue,
        });
        cursor = end;
    }
    output
}

enum DollarBodySearch {
    None,
    Found(usize),
    Ambiguous(Site),
}

fn routine_body_dollar_opener(source: &str, start: usize) -> DollarBodySearch {
    let bytes = source.as_bytes();
    let mut index = start;
    let mut depth = 0_u32;
    while index < bytes.len() {
        if let Some(delimiter) = identifier_delimiter(bytes[index]) {
            index = quoted_identifier_end(bytes, index, delimiter).unwrap_or(bytes.len());
            continue;
        }
        if dollar_quote_delimiter_at(source, index).is_some() {
            if let Some(end) = paired_dollar_quote_end(source, index, source.len()) {
                index = end;
                continue;
            }
            return DollarBodySearch::Ambiguous(Site::new(index, first_line_end(source, index)));
        }
        if bytes[index] == b';' {
            return DollarBodySearch::None;
        }
        match bytes[index] {
            b'(' => {
                depth = depth.saturating_add(1);
                index += 1;
                continue;
            }
            b')' => {
                depth = depth.saturating_sub(1);
                index += 1;
                continue;
            }
            _ => {}
        }
        if depth == 0
            && let Some(as_end) = consume_keyword(source, index, "AS")
        {
            let next = skip_sql_whitespace(source, as_end);
            if dollar_quote_delimiter_at(source, next).is_some() {
                return DollarBodySearch::Found(next);
            }
        }
        index += 1;
    }
    DollarBodySearch::None
}

fn skip_dollar_quote(source: &str, start: usize) -> Option<usize> {
    paired_dollar_quote_end(source, start, source.len())
}

fn paired_dollar_quote_end(source: &str, start: usize, end: usize) -> Option<usize> {
    let delimiter = dollar_quote_delimiter_at(source, start)?;
    let content_start = start + delimiter.len();
    source
        .get(content_start..end.min(source.len()))?
        .find(delimiter)
        .map(|offset| content_start + offset + delimiter.len())
}

fn first_unquoted_semicolon(source: &str, start: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut index = start.min(bytes.len());
    while index < bytes.len() {
        if let Some(delimiter) = identifier_delimiter(bytes[index]) {
            index = quoted_identifier_end(bytes, index, delimiter).unwrap_or(bytes.len());
            continue;
        }
        if bytes[index] == b';' {
            return Some(index + 1);
        }
        index += 1;
    }
    None
}

fn first_line_end(source: &str, start: usize) -> usize {
    source[start..]
        .find('\n')
        .map_or(source.len(), |offset| start + offset + 1)
}

fn dollar_quote_delimiter_at(source: &str, start: usize) -> Option<&str> {
    let bytes = source.as_bytes();
    if bytes.get(start).copied() != Some(b'$') {
        return None;
    }
    let mut end = start + 1;
    if bytes.get(end).copied() == Some(b'$') {
        return source.get(start..=end);
    }
    if !bytes
        .get(end)
        .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
    {
        return None;
    }
    end += 1;
    while bytes
        .get(end)
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
    {
        end += 1;
    }
    (bytes.get(end).copied() == Some(b'$')).then(|| &source[start..=end])
}

fn query_statements(
    source: &str,
    declarations: &[Statement],
) -> (Vec<QueryStatement>, Vec<BoundaryIssue>) {
    let Ok(operation_pattern) = Regex::new(r"(?i)^(SELECT|INSERT|UPDATE|DELETE|MERGE)\b") else {
        return (Vec::new(), Vec::new());
    };
    let mut output = Vec::new();
    let mut issues = Vec::new();
    for scanned in top_level_statement_ranges(source, declarations) {
        if let Some(issue) = scanned.issue {
            issues.push(issue);
            continue;
        }
        let site = scanned.site;
        let Some(segment) = source.get(site.start..site.end) else {
            continue;
        };
        let leading = segment.len() - segment.trim_start().len();
        let statement_start = site.start + leading;
        if declarations.iter().any(|declaration| {
            statement_start >= declaration.offset && statement_start < declaration.end
        }) {
            continue;
        }
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
            output.push((statement_start, operation_start, site.end, operation));
        }
    }
    (output, issues)
}

fn top_level_cte_operation(statement: &str) -> Option<(usize, String)> {
    let parsed = parse_ctes(statement)?;
    let (start, end) = word_at(statement, parsed.terminal_start)?;
    let operation = statement[start..end].to_ascii_lowercase();
    matches!(
        operation.as_str(),
        "select" | "insert" | "update" | "delete" | "merge"
    )
    .then_some((start, operation))
}

fn cte_names(statement: &str) -> HashSet<String> {
    parse_ctes(statement)
        .map(|parsed| parsed.names)
        .unwrap_or_default()
}

struct ParsedCtes {
    names: HashSet<String>,
    terminal_start: usize,
}

fn parse_ctes(statement: &str) -> Option<ParsedCtes> {
    let mut index = skip_sql_whitespace(statement, 0);
    index = consume_keyword(statement, index, "WITH")?;
    index = skip_sql_whitespace(statement, index);
    if let Some(end) = consume_keyword(statement, index, "RECURSIVE") {
        index = skip_sql_whitespace(statement, end);
    }

    let mut names = HashSet::new();
    loop {
        let name_start = index;
        let name_end = parse_identifier_end(statement, name_start)?;
        let name = &statement[name_start..name_end];
        if is_reserved_object_reference(name) {
            return None;
        }
        names.insert(identifier_key(name));
        index = skip_sql_whitespace(statement, name_end);

        if statement.as_bytes().get(index).copied() == Some(b'(') {
            index = balanced_paren_end(statement, index)?;
            index = skip_sql_whitespace(statement, index);
        }

        index = consume_keyword(statement, index, "AS")?;
        index = skip_sql_whitespace(statement, index);
        if let Some(not_end) = consume_keyword(statement, index, "NOT") {
            let materialized_start = skip_sql_whitespace(statement, not_end);
            index = consume_keyword(statement, materialized_start, "MATERIALIZED")?;
            index = skip_sql_whitespace(statement, index);
        } else if let Some(materialized_end) = consume_keyword(statement, index, "MATERIALIZED") {
            index = skip_sql_whitespace(statement, materialized_end);
        }

        if statement.as_bytes().get(index).copied() != Some(b'(') {
            return None;
        }
        index = balanced_paren_end(statement, index)?;
        index = skip_sql_whitespace(statement, index);
        if statement.as_bytes().get(index).copied() == Some(b',') {
            index = skip_sql_whitespace(statement, index + 1);
            continue;
        }
        return Some(ParsedCtes {
            names,
            terminal_start: index,
        });
    }
}

fn skip_sql_whitespace(statement: &str, mut index: usize) -> usize {
    while statement
        .as_bytes()
        .get(index)
        .is_some_and(u8::is_ascii_whitespace)
    {
        index += 1;
    }
    index
}

fn consume_keyword(statement: &str, start: usize, keyword: &str) -> Option<usize> {
    let end = start.checked_add(keyword.len())?;
    let value = statement.get(start..end)?;
    if !value.eq_ignore_ascii_case(keyword) {
        return None;
    }
    let before_is_word = start
        .checked_sub(1)
        .and_then(|index| statement.as_bytes().get(index))
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_' || *byte == b'$');
    let after_is_word = statement
        .as_bytes()
        .get(end)
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_' || *byte == b'$');
    (!before_is_word && !after_is_word).then_some(end)
}

fn word_at(statement: &str, start: usize) -> Option<(usize, usize)> {
    let bytes = statement.as_bytes();
    if !bytes
        .get(start)
        .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
    {
        return None;
    }
    let mut end = start + 1;
    while bytes
        .get(end)
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
    {
        end += 1;
    }
    Some((start, end))
}

fn parse_identifier_end(statement: &str, start: usize) -> Option<usize> {
    let bytes = statement.as_bytes();
    if let Some(delimiter) = bytes.get(start).copied().and_then(identifier_delimiter) {
        return quoted_identifier_end(bytes, start, delimiter);
    }
    if !bytes.get(start).is_some_and(|byte| {
        byte.is_ascii_alphanumeric() || *byte == b'_' || *byte == b'$' || !byte.is_ascii()
    }) {
        return None;
    }
    let mut end = start + 1;
    while bytes.get(end).is_some_and(|byte| {
        byte.is_ascii_alphanumeric() || *byte == b'_' || *byte == b'$' || !byte.is_ascii()
    }) {
        end += 1;
    }
    Some(end)
}

fn balanced_paren_end(statement: &str, open: usize) -> Option<usize> {
    let bytes = statement.as_bytes();
    let mut depth = 0_u32;
    let mut index = open;
    while index < bytes.len() {
        if let Some(delimiter) = identifier_delimiter(bytes[index]) {
            index = quoted_identifier_end(bytes, index, delimiter)?;
            continue;
        }
        if let Some(end) = skip_dollar_quote(statement, index) {
            index = end;
            continue;
        }
        match bytes[index] {
            b'(' => depth = depth.saturating_add(1),
            b')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(index + 1);
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
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
            (!is_reserved_object_reference(alias))
                .then(|| (identifier_key(alias), target.to_owned()))
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
        .map(identifier_part_key)
        .collect::<Vec<_>>()
        .join("|")
}

fn short_identifier_key(name: &str) -> String {
    qualified_identifier_parts(name)
        .last()
        .map_or_else(String::new, |part| identifier_part_key(part))
}

fn identifier_part_key(part: &str) -> String {
    let trimmed = part.trim();
    let (quoted, style, value) = if trimmed.starts_with('"') && trimmed.ends_with('"') {
        (true, "double", normalize_identifier_part(trimmed))
    } else if trimmed.starts_with('`') && trimmed.ends_with('`') {
        (true, "backtick", normalize_identifier_part(trimmed))
    } else if trimmed.starts_with('[') && trimmed.ends_with(']') {
        (true, "bracket", normalize_identifier_part(trimmed))
    } else {
        (
            false,
            "unquoted",
            normalize_identifier_part(trimmed).to_ascii_lowercase(),
        )
    };
    if !quoted || value == value.to_ascii_lowercase() {
        return format!("folded:{}:{}", value.len(), value.to_ascii_lowercase());
    }
    format!("{style}:{}:{value}", value.len())
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

fn identifier_delimiter(open: u8) -> Option<u8> {
    match open {
        b'"' | b'`' => Some(open),
        b'[' => Some(b']'),
        _ => None,
    }
}

fn quoted_identifier_end(bytes: &[u8], start: usize, delimiter: u8) -> Option<usize> {
    let mut index = start + 1;
    while index < bytes.len() {
        if bytes[index] != delimiter {
            index += 1;
            continue;
        }
        if bytes.get(index + 1).copied() == Some(delimiter) {
            index += 2;
            continue;
        }
        return Some(index + 1);
    }
    None
}

fn is_reserved_object_reference(name: &str) -> bool {
    qualified_identifier_parts(name).into_iter().any(|part| {
        let trimmed = part.trim();
        if identifier_delimiter(trimmed.as_bytes().first().copied().unwrap_or_default()).is_some() {
            return false;
        }
        matches!(
            normalize_identifier_part(trimmed)
                .to_ascii_lowercase()
                .as_str(),
            "all"
                | "alter"
                | "and"
                | "as"
                | "begin"
                | "by"
                | "case"
                | "create"
                | "database"
                | "delayed"
                | "delete"
                | "distinct"
                | "else"
                | "end"
                | "exists"
                | "false"
                | "first"
                | "from"
                | "full"
                | "function"
                | "group"
                | "having"
                | "high_priority"
                | "if"
                | "ignore"
                | "index"
                | "inner"
                | "insert"
                | "into"
                | "join"
                | "lateral"
                | "left"
                | "low_priority"
                | "materialized"
                | "matched"
                | "merge"
                | "next"
                | "not"
                | "null"
                | "of"
                | "on"
                | "only"
                | "or"
                | "order"
                | "outer"
                | "procedure"
                | "quick"
                | "recursive"
                | "replace"
                | "returning"
                | "right"
                | "rows"
                | "schema"
                | "select"
                | "set"
                | "skip"
                | "table"
                | "temporary"
                | "then"
                | "trigger"
                | "true"
                | "union"
                | "unique"
                | "unlogged"
                | "update"
                | "using"
                | "values"
                | "view"
                | "when"
                | "where"
                | "with"
        )
    })
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
