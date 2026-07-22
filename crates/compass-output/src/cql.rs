use std::collections::BTreeMap;

use compass_cypher::{Column, CompassValue};
use compass_query::QueryResult;
use serde::Serialize;

use crate::OutputError;

#[must_use]
pub fn render_cql_table(result: &QueryResult) -> String {
    if let Some(explain) = &result.explain
        && result.rows.is_empty()
    {
        let mut lines = vec![format!("Plan (schema {})", explain.schema_fingerprint)];
        for (index, operator) in explain.operators.iter().enumerate() {
            lines.push(format!("{:>3}  {operator}", index + 1));
        }
        if !explain.optimizations.is_empty() {
            lines.push("Optimizations".to_owned());
            lines.extend(
                explain
                    .optimizations
                    .iter()
                    .map(|value| format!("  {value}")),
            );
        }
        return lines.join("\n");
    }
    let headers = result
        .columns
        .iter()
        .map(|column| escape_table(&column.name))
        .collect::<Vec<_>>();
    let mut widths = headers.iter().map(String::len).collect::<Vec<_>>();
    let rendered = result
        .rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|value| escape_table(&format_value(value)))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    for row in &rendered {
        for (index, value) in row.iter().enumerate() {
            if let Some(width) = widths.get_mut(index) {
                *width = (*width).max(value.len());
            }
        }
    }
    let mut lines = Vec::new();
    if !headers.is_empty() {
        lines.push(render_table_row(&headers, &widths));
        lines.push(
            widths
                .iter()
                .map(|width| "-".repeat(*width))
                .collect::<Vec<_>>()
                .join("-+-"),
        );
    }
    for row in &rendered {
        lines.push(render_table_row(row, &widths));
    }
    lines.push(format!("{} row(s)", result.rows.len()));
    if let Some(profile) = &result.profile {
        lines.push(format!(
            "Profile: {} candidates, {} relationships, {} bytes peak, {:?}",
            profile.candidate_nodes,
            profile.expanded_relationships,
            profile.peak_memory_bytes,
            profile.elapsed
        ));
    }
    lines.join("\n")
}

pub fn render_cql_json(result: &QueryResult) -> Result<String, OutputError> {
    serde_json::to_string_pretty(&JsonResult::from(result)).map_err(OutputError::from)
}

pub fn render_cql_jsonl(result: &QueryResult) -> Result<String, OutputError> {
    let mut lines = Vec::with_capacity(result.rows.len().saturating_add(2));
    lines.push(serde_json::to_string(&serde_json::json!({
        "schema": "compass.cql.jsonl/1",
        "columns": result.columns,
    }))?);
    for row in &result.rows {
        let values = result
            .columns
            .iter()
            .zip(row)
            .map(|(column, value)| (column.name.clone(), value.clone()))
            .collect::<BTreeMap<_, _>>();
        lines.push(serde_json::to_string(&serde_json::json!({
            "type": "row",
            "values": values,
        }))?);
    }
    lines.push(serde_json::to_string(&serde_json::json!({
        "type": "summary",
        "rows": result.rows.len(),
        "profile": result.profile,
        "explain": result.explain,
    }))?);
    Ok(lines.join("\n"))
}

#[derive(Serialize)]
struct JsonResult<'a> {
    schema: &'static str,
    columns: &'a [Column],
    rows: Vec<BTreeMap<&'a str, &'a CompassValue>>,
    profile: &'a Option<compass_query::QueryProfile>,
    explain: &'a Option<compass_query::ExplainPlan>,
}

impl<'a> From<&'a QueryResult> for JsonResult<'a> {
    fn from(result: &'a QueryResult) -> Self {
        let rows = result
            .rows
            .iter()
            .map(|row| {
                result
                    .columns
                    .iter()
                    .zip(row)
                    .map(|(column, value)| (column.name.as_str(), value))
                    .collect()
            })
            .collect();
        Self {
            schema: "compass.cql.result/1",
            columns: &result.columns,
            rows,
            profile: &result.profile,
            explain: &result.explain,
        }
    }
}

fn render_table_row(values: &[String], widths: &[usize]) -> String {
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            format!(
                "{value:<width$}",
                width = widths.get(index).copied().unwrap_or(0)
            )
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

fn escape_table(value: &str) -> String {
    value
        .chars()
        .flat_map(|character| match character {
            '\n' => "\\n".chars().collect::<Vec<_>>(),
            '\r' => "\\r".chars().collect::<Vec<_>>(),
            '\t' => "\\t".chars().collect::<Vec<_>>(),
            value if value.is_control() => {
                format!("\\u{{{:04x}}}", u32::from(value)).chars().collect()
            }
            value => vec![value],
        })
        .collect()
}

fn format_value(value: &CompassValue) -> String {
    match value {
        CompassValue::Null => "null".to_owned(),
        CompassValue::Boolean(value) => value.to_string(),
        CompassValue::Integer(value) => value.to_string(),
        CompassValue::Float(value) => value.to_string(),
        CompassValue::String(value) => value.to_string(),
        CompassValue::List(values) => format!(
            "[{}]",
            values
                .iter()
                .map(format_value)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        CompassValue::Map(values) => format!(
            "{{{}}}",
            values
                .iter()
                .map(|(key, value)| format!("{key}: {}", format_value(value)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        CompassValue::Node(value) => format!("(:{})", value.id),
        CompassValue::Relationship(value) => {
            format!("[{}:{}]", value.index, value.relation)
        }
        CompassValue::Path(value) => {
            let mut output = String::new();
            for (index, node) in value.nodes.iter().enumerate() {
                if index > 0
                    && let Some(relationship) = value.relationships.get(index - 1)
                {
                    output.push_str(&format!("-[:{}]->", relationship.relation));
                }
                output.push_str(&format!("({})", node.id));
            }
            output
        }
    }
}
