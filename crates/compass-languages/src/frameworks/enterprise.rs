use std::path::Path;

use regex::Regex;
use serde_json::{Map, Value};

use super::text::{line_anchor, text};
use super::{RawDomainFact, RawFrameworkFact, RawFrameworkOrigin};

pub(super) fn detect(path: &Path, source: &[u8], language: &str) -> Vec<RawFrameworkFact> {
    let body = text(source);
    match language {
        "python" => python(path, source, body),
        "typescript" | "tsx" | "javascript" => typescript(path, source, body),
        "java" => java(path, source, body),
        "csharp" => csharp(path, source, body),
        "ruby" => ruby(path, source, body),
        "php" => php(path, source, body),
        "go" => go(path, source, body),
        "rust" => rust(path, source, body),
        _ => Vec::new(),
    }
}

fn python(path: &Path, source: &[u8], body: &str) -> Vec<RawFrameworkFact> {
    let mut facts = Vec::new();
    if body.contains("celery") {
        let mut pending_task = None::<(Option<String>, Option<String>, usize, String)>;
        let task = Regex::new(r#"^\s*@(?:app\.task|shared_task)(?:\((.*)\))?"#).ok();
        let function = Regex::new(r"^\s*(?:async\s+)?def\s+([A-Za-z_]\w*)").ok();
        let name = Regex::new(r#"\bname\s*=\s*["']([^"']+)["']"#).ok();
        let queue = Regex::new(r#"\bqueue\s*=\s*["']([^"']+)["']"#).ok();
        let mut offset = 0;
        for line in body.split_inclusive('\n') {
            if let Some(capture) = task.as_ref().and_then(|pattern| pattern.captures(line)) {
                let args = capture
                    .get(1)
                    .map(|value| value.as_str())
                    .unwrap_or_default();
                pending_task = Some((
                    name.as_ref()
                        .and_then(|pattern| pattern.captures(args))
                        .and_then(|capture| capture.get(1))
                        .map(|value| value.as_str().to_owned()),
                    queue
                        .as_ref()
                        .and_then(|pattern| pattern.captures(args))
                        .and_then(|capture| capture.get(1))
                        .map(|value| value.as_str().to_owned()),
                    offset,
                    line.to_owned(),
                ));
            } else if let Some((configured, queue, at, anchor_line)) = pending_task.take()
                && let Some(handler) = function
                    .as_ref()
                    .and_then(|pattern| pattern.captures(line))
                    .and_then(|capture| capture.get(1))
            {
                let handler = handler.as_str();
                facts.push(job_fact(
                    "celery",
                    configured.as_deref().unwrap_or(handler),
                    handler,
                    None,
                    queue.as_deref(),
                    path,
                    source,
                    at,
                    &anchor_line,
                ));
            }
            offset += line.len();
        }
    }
    if body.contains("django.db") || body.contains("sqlalchemy") {
        let framework = if body.contains("django.db") {
            "django-orm"
        } else {
            "sqlalchemy"
        };
        facts.extend(class_table_mappings(
            framework,
            path,
            source,
            body,
            r"^\s*class\s+([A-Za-z_]\w*)\s*\([^)]*(?:Model|Base)[^)]*\)",
            if framework == "django-orm" {
                r#"^\s*db_table\s*=\s*["']([^"']+)["']"#
            } else {
                r#"^\s*__tablename__\s*=\s*["']([^"']+)["']"#
            },
        ));
    }
    facts
}

fn typescript(path: &Path, source: &[u8], body: &str) -> Vec<RawFrameworkFact> {
    let mut facts = Vec::new();
    if body.contains("@nestjs/") {
        let class = Regex::new(r"\bclass\s+([A-Za-z_]\w*)").ok();
        let method = Regex::new(r"^\s*(?:async\s+)?([A-Za-z_]\w*)\s*\(").ok();
        let publish = Regex::new(r#"\.(emit|publish|send)\(\s*["'`]([^"'`]+)["'`]"#).ok();
        let mut owner = String::new();
        let mut callable = String::new();
        let mut offset = 0;
        for line in body.split_inclusive('\n') {
            if let Some(name) = class
                .as_ref()
                .and_then(|pattern| pattern.captures(line))
                .and_then(|capture| capture.get(1))
            {
                owner = name.as_str().to_owned();
            }
            if let Some(name) = method
                .as_ref()
                .and_then(|pattern| pattern.captures(line))
                .and_then(|capture| capture.get(1))
            {
                callable = if owner.is_empty() {
                    name.as_str().to_owned()
                } else {
                    format!("{}.{}", owner, name.as_str())
                };
            }
            for capture in publish
                .as_ref()
                .into_iter()
                .flat_map(|pattern| pattern.captures_iter(line))
            {
                if let (Some(operation), Some(subject)) = (capture.get(1), capture.get(2))
                    && !callable.is_empty()
                {
                    facts.push(message_fact(
                        "nestjs",
                        if operation.as_str() == "send" {
                            "message"
                        } else {
                            "event"
                        },
                        subject.as_str(),
                        "microservice",
                        &callable,
                        if operation.as_str() == "send" {
                            "produces"
                        } else {
                            "publishes"
                        },
                        path,
                        source,
                        offset,
                        line,
                    ));
                }
            }
            offset += line.len();
        }
    }
    if body.contains("typeorm") {
        facts.extend(decorator_table_mappings(
            "typeorm",
            path,
            source,
            body,
            r#"@Entity\(\s*["']([^"']+)["']\s*\)"#,
            r"\bclass\s+([A-Za-z_]\w*)",
        ));
    }
    facts
}

fn java(path: &Path, source: &[u8], body: &str) -> Vec<RawFrameworkFact> {
    let mut facts = Vec::new();
    let class = Regex::new(r"\bclass\s+([A-Za-z_]\w*)").ok();
    let method = Regex::new(
        r"\b(?:public|protected|private|static|final|synchronized|\s)+[A-Za-z0-9_<>,.?\[\]\s]+\s+([A-Za-z_]\w*)\s*\(",
    )
    .ok();
    let listener = Regex::new(
        r#"@(KafkaListener|RabbitListener|EventListener)\s*\((?:[^"']*["']([^"']+)["']|([A-Za-z_]\w*)\.class)"#,
    )
    .ok();
    let scheduled = Regex::new(r#"@Scheduled\s*\(([^)]*)\)"#).ok();
    let schedule_value = Regex::new(r#"(?:cron|fixedRate|fixedDelay)\s*=\s*["']?([^"',)]+)"#).ok();
    let mut owner = String::new();
    let mut pending_message = None::<(String, String, String, usize, String)>;
    let mut pending_job = None::<(String, usize, String)>;
    let mut offset = 0;
    for line in body.split_inclusive('\n') {
        if let Some(name) = class
            .as_ref()
            .and_then(|pattern| pattern.captures(line))
            .and_then(|capture| capture.get(1))
        {
            owner = name.as_str().to_owned();
        }
        if let Some(capture) = listener.as_ref().and_then(|pattern| pattern.captures(line)) {
            let decorator = capture
                .get(1)
                .map(|value| value.as_str())
                .unwrap_or_default();
            let subject = capture
                .get(2)
                .or_else(|| capture.get(3))
                .map(|value| value.as_str().to_owned())
                .unwrap_or_default();
            pending_message = Some((
                if decorator == "KafkaListener" {
                    "topic"
                } else if decorator == "RabbitListener" {
                    "queue"
                } else {
                    "event"
                }
                .to_owned(),
                subject,
                if decorator == "KafkaListener" {
                    "consumes"
                } else {
                    "handles"
                }
                .to_owned(),
                offset,
                line.to_owned(),
            ));
        }
        if let Some(capture) = scheduled
            .as_ref()
            .and_then(|pattern| pattern.captures(line))
        {
            let args = capture
                .get(1)
                .map(|value| value.as_str())
                .unwrap_or_default();
            let schedule = schedule_value
                .as_ref()
                .and_then(|pattern| pattern.captures(args))
                .and_then(|capture| capture.get(1))
                .map(|value| value.as_str().to_owned())
                .unwrap_or_else(|| args.to_owned());
            pending_job = Some((schedule, offset, line.to_owned()));
        } else if let Some(name) = method
            .as_ref()
            .and_then(|pattern| pattern.captures(line))
            .and_then(|capture| capture.get(1))
        {
            let handler = if owner.is_empty() {
                name.as_str().to_owned()
            } else {
                format!("{}.{}", owner, name.as_str())
            };
            if let Some((kind, subject, relationship, at, anchor_line)) = pending_message.take() {
                facts.push(message_fact(
                    "spring",
                    &kind,
                    &subject,
                    if kind == "topic" {
                        "kafka"
                    } else if kind == "queue" {
                        "rabbitmq"
                    } else {
                        "spring"
                    },
                    &handler,
                    &relationship,
                    path,
                    source,
                    at,
                    &anchor_line,
                ));
            }
            if let Some((schedule, at, anchor_line)) = pending_job.take() {
                facts.push(job_fact(
                    "spring",
                    &handler,
                    &handler,
                    Some(&schedule),
                    None,
                    path,
                    source,
                    at,
                    &anchor_line,
                ));
            }
        }
        offset += line.len();
    }
    if body.contains("jakarta.persistence") || body.contains("javax.persistence") {
        facts.extend(java_table_mappings(path, source, body));
    }
    facts
}

fn csharp(path: &Path, source: &[u8], body: &str) -> Vec<RawFrameworkFact> {
    let mut facts = Vec::new();
    if body.contains("BackgroundService") || body.contains("IHostedService") {
        let class =
            Regex::new(r"\bclass\s+([A-Za-z_]\w*)\s*:[^{]*(?:BackgroundService|IHostedService)")
                .ok();
        if let Some(class) = class {
            for capture in class.captures_iter(body) {
                if let (Some(whole), Some(name)) = (capture.get(0), capture.get(1)) {
                    let line = body[..whole.start()]
                        .bytes()
                        .filter(|byte| *byte == b'\n')
                        .count()
                        + 1;
                    let anchor_line = body.lines().nth(line - 1).unwrap_or(name.as_str());
                    facts.push(job_fact(
                        "aspnet",
                        name.as_str(),
                        &format!("{}.ExecuteAsync", name.as_str()),
                        None,
                        None,
                        path,
                        source,
                        whole.start(),
                        anchor_line,
                    ));
                }
            }
        }
    }
    if body.contains("System.ComponentModel.DataAnnotations.Schema") {
        facts.extend(decorator_table_mappings(
            "entity-framework",
            path,
            source,
            body,
            r#"\[Table\(\s*"([^"]+)""#,
            r"\bclass\s+([A-Za-z_]\w*)",
        ));
    }
    facts
}

fn ruby(path: &Path, source: &[u8], body: &str) -> Vec<RawFrameworkFact> {
    if !body.contains("ActiveRecord") && !body.contains("ApplicationRecord") {
        return Vec::new();
    }
    class_table_mappings(
        "active-record",
        path,
        source,
        body,
        r"^\s*class\s+([A-Za-z_]\w*)\s*<\s*(?:ApplicationRecord|ActiveRecord::Base)",
        r#"^\s*self\.table_name\s*=\s*["']([^"']+)["']"#,
    )
}

fn php(path: &Path, source: &[u8], body: &str) -> Vec<RawFrameworkFact> {
    if !body.contains("Illuminate\\Database\\Eloquent") {
        return Vec::new();
    }
    class_table_mappings(
        "eloquent",
        path,
        source,
        body,
        r"^\s*class\s+([A-Za-z_]\w*)\s+extends\s+Model",
        r#"^\s*(?:protected\s+)?\$table\s*=\s*["']([^"']+)["']"#,
    )
}

fn go(path: &Path, source: &[u8], body: &str) -> Vec<RawFrameworkFact> {
    if !body.contains("gorm.io/gorm") {
        return Vec::new();
    }
    let Ok(pattern) = Regex::new(
        r#"(?s)func\s*\(\s*([A-Za-z_]\w*)\s*\)\s*TableName\s*\(\s*\)\s*string\s*\{\s*return\s*["']([^"']+)["']"#,
    ) else {
        return Vec::new();
    };
    pattern
        .captures_iter(body)
        .filter_map(|capture| {
            let (whole, model, table) = (capture.get(0)?, capture.get(1)?, capture.get(2)?);
            Some(orm_fact(
                "gorm",
                model.as_str(),
                table.as_str(),
                "",
                path,
                source,
                whole.start(),
                body[whole.start()..whole.end()]
                    .lines()
                    .next()
                    .unwrap_or_default(),
            ))
        })
        .collect()
}

fn rust(path: &Path, source: &[u8], body: &str) -> Vec<RawFrameworkFact> {
    if !body.contains("diesel") {
        return Vec::new();
    }
    decorator_table_mappings(
        "diesel",
        path,
        source,
        body,
        r"#\[diesel\(table_name\s*=\s*([A-Za-z_]\w*)\)\]",
        r"\bstruct\s+([A-Za-z_]\w*)",
    )
}

fn class_table_mappings(
    framework: &str,
    path: &Path,
    source: &[u8],
    body: &str,
    class_pattern: &str,
    table_pattern: &str,
) -> Vec<RawFrameworkFact> {
    let (Ok(class), Ok(table)) = (Regex::new(class_pattern), Regex::new(table_pattern)) else {
        return Vec::new();
    };
    let mut current = None::<(String, usize, String)>;
    let mut facts = Vec::new();
    let mut offset = 0;
    for line in body.split_inclusive('\n') {
        if let Some(name) = class.captures(line).and_then(|capture| capture.get(1)) {
            current = Some((name.as_str().to_owned(), offset, line.to_owned()));
        } else if let (Some((model, at, anchor_line)), Some(table)) = (
            current.as_ref(),
            table.captures(line).and_then(|capture| capture.get(1)),
        ) {
            facts.push(orm_fact(
                framework,
                model,
                table.as_str(),
                "",
                path,
                source,
                *at,
                anchor_line,
            ));
            current = None;
        }
        offset += line.len();
    }
    facts
}

fn decorator_table_mappings(
    framework: &str,
    path: &Path,
    source: &[u8],
    body: &str,
    table_pattern: &str,
    class_pattern: &str,
) -> Vec<RawFrameworkFact> {
    let (Ok(table), Ok(class)) = (Regex::new(table_pattern), Regex::new(class_pattern)) else {
        return Vec::new();
    };
    let mut pending = None::<(String, usize, String)>;
    let mut facts = Vec::new();
    let mut offset = 0;
    for line in body.split_inclusive('\n') {
        if let Some(name) = table.captures(line).and_then(|capture| capture.get(1)) {
            pending = Some((name.as_str().to_owned(), offset, line.to_owned()));
        } else if let (Some((table, at, anchor_line)), Some(model)) = (
            pending.take(),
            class.captures(line).and_then(|capture| capture.get(1)),
        ) {
            facts.push(orm_fact(
                framework,
                model.as_str(),
                &table,
                "",
                path,
                source,
                at,
                &anchor_line,
            ));
        }
        offset += line.len();
    }
    facts
}

fn java_table_mappings(path: &Path, source: &[u8], body: &str) -> Vec<RawFrameworkFact> {
    let Ok(table) = Regex::new(
        r#"@Table\([^)]*name\s*=\s*["']([^"']+)["'](?:[^)]*schema\s*=\s*["']([^"']+)["'])?"#,
    ) else {
        return Vec::new();
    };
    let Ok(class) = Regex::new(r"\bclass\s+([A-Za-z_]\w*)") else {
        return Vec::new();
    };
    let mut pending = None::<(String, String, usize, String)>;
    let mut facts = Vec::new();
    let mut offset = 0;
    for line in body.split_inclusive('\n') {
        if let Some(capture) = table.captures(line)
            && let Some(name) = capture.get(1)
        {
            pending = Some((
                name.as_str().to_owned(),
                capture
                    .get(2)
                    .map(|value| value.as_str().to_owned())
                    .unwrap_or_default(),
                offset,
                line.to_owned(),
            ));
        } else if let (Some((table, schema, at, anchor_line)), Some(model)) = (
            pending.take(),
            class.captures(line).and_then(|capture| capture.get(1)),
        ) {
            facts.push(orm_fact(
                "jpa",
                model.as_str(),
                &table,
                &schema,
                path,
                source,
                at,
                &anchor_line,
            ));
        }
        offset += line.len();
    }
    facts
}

#[allow(clippy::too_many_arguments)]
fn message_fact(
    framework: &str,
    kind: &str,
    subject: &str,
    transport: &str,
    handler: &str,
    relationship: &str,
    path: &Path,
    source: &[u8],
    offset: usize,
    line: &str,
) -> RawFrameworkFact {
    RawFrameworkFact::Domain(RawDomainFact {
        framework: framework.to_owned(),
        kind: kind.to_owned(),
        name: subject.to_owned(),
        declaring_scope: path.to_string_lossy().into_owned(),
        anchor: line_anchor(path, source, offset, line),
        origin: RawFrameworkOrigin::Ast,
        detail: Map::from_iter([
            ("transport".into(), Value::String(transport.to_owned())),
            ("subject".into(), Value::String(subject.to_owned())),
            (
                "handler_reference".into(),
                Value::String(handler.to_owned()),
            ),
            (
                "relationship".into(),
                Value::String(relationship.to_owned()),
            ),
        ]),
    })
}

#[allow(clippy::too_many_arguments)]
fn job_fact(
    framework: &str,
    name: &str,
    handler: &str,
    schedule: Option<&str>,
    queue: Option<&str>,
    path: &Path,
    source: &[u8],
    offset: usize,
    line: &str,
) -> RawFrameworkFact {
    let mut detail = Map::from_iter([(
        "handler_reference".into(),
        Value::String(handler.to_owned()),
    )]);
    if let Some(schedule) = schedule {
        detail.insert("schedule".into(), Value::String(schedule.to_owned()));
    }
    if let Some(queue) = queue {
        detail.insert("queue".into(), Value::String(queue.to_owned()));
    }
    RawFrameworkFact::Domain(RawDomainFact {
        framework: framework.to_owned(),
        kind: "job".to_owned(),
        name: name.to_owned(),
        declaring_scope: path.to_string_lossy().into_owned(),
        anchor: line_anchor(path, source, offset, line),
        origin: RawFrameworkOrigin::Ast,
        detail,
    })
}

#[allow(clippy::too_many_arguments)]
fn orm_fact(
    framework: &str,
    model: &str,
    table: &str,
    schema: &str,
    path: &Path,
    source: &[u8],
    offset: usize,
    line: &str,
) -> RawFrameworkFact {
    RawFrameworkFact::Domain(RawDomainFact {
        framework: framework.to_owned(),
        kind: "orm_mapping".to_owned(),
        name: model.to_owned(),
        declaring_scope: path.to_string_lossy().into_owned(),
        anchor: line_anchor(path, source, offset, line),
        origin: RawFrameworkOrigin::Ast,
        detail: Map::from_iter([
            ("model_reference".into(), Value::String(model.to_owned())),
            ("database_table".into(), Value::String(table.to_owned())),
            ("database_schema".into(), Value::String(schema.to_owned())),
            ("explicit".into(), Value::Bool(true)),
        ]),
    })
}
