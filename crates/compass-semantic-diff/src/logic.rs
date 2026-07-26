use std::collections::{BTreeMap, BTreeSet};

use compass_history::{SourceFileDelta, SourceHunk};
use compass_model::NodeRecord;
use serde_json::Value;

const MAX_CONDITIONS_PER_FINDING: usize = 20;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct LogicDelta {
    pub added_conditions: Vec<String>,
    pub removed_conditions: Vec<String>,
}

impl LogicDelta {
    pub(crate) fn explanation(&self) -> String {
        let mut changes = Vec::new();
        if !self.added_conditions.is_empty() {
            changes.push(format!(
                "added {}",
                describe_conditions(&self.added_conditions)
            ));
        }
        if !self.removed_conditions.is_empty() {
            changes.push(format!(
                "removed {}",
                describe_conditions(&self.removed_conditions)
            ));
        }
        format!("Control flow changed: {}.", changes.join("; "))
    }
}

#[derive(Clone, Debug)]
struct HunkLogic {
    hunk: SourceHunk,
    added_conditions: Vec<String>,
    removed_conditions: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct LogicDeltaIndex {
    by_path: BTreeMap<String, Vec<HunkLogic>>,
}

impl LogicDeltaIndex {
    pub(crate) fn new(source_deltas: &[SourceFileDelta]) -> Self {
        let mut by_path = BTreeMap::new();
        for delta in source_deltas {
            let parsed = parse_hunks(delta);
            if parsed.is_empty() {
                continue;
            }
            for path in [delta.old_path.as_ref(), delta.new_path.as_ref()]
                .into_iter()
                .flatten()
            {
                by_path.insert(path.clone(), parsed.clone());
            }
        }
        Self { by_path }
    }

    pub(crate) fn for_nodes(
        &self,
        old: Option<&NodeRecord>,
        new: Option<&NodeRecord>,
    ) -> Option<LogicDelta> {
        let path = source_file(new).or_else(|| source_file(old))?;
        let hunks = self.by_path.get(path)?;
        let old_span = line_span(old);
        let new_span = line_span(new);
        let mut added = BTreeSet::new();
        let mut removed = BTreeSet::new();
        for hunk in hunks {
            if !old_span
                .is_some_and(|span| overlaps(span, hunk.hunk.old_start, hunk.hunk.old_lines))
                && !new_span
                    .is_some_and(|span| overlaps(span, hunk.hunk.new_start, hunk.hunk.new_lines))
            {
                continue;
            }
            added.extend(hunk.added_conditions.iter().cloned());
            removed.extend(hunk.removed_conditions.iter().cloned());
        }
        let unchanged = added.intersection(&removed).cloned().collect::<Vec<_>>();
        for condition in unchanged {
            added.remove(&condition);
            removed.remove(&condition);
        }
        let added_conditions = added
            .into_iter()
            .take(MAX_CONDITIONS_PER_FINDING)
            .collect::<Vec<_>>();
        let removed_conditions = removed
            .into_iter()
            .take(MAX_CONDITIONS_PER_FINDING)
            .collect::<Vec<_>>();
        (!added_conditions.is_empty() || !removed_conditions.is_empty()).then_some(LogicDelta {
            added_conditions,
            removed_conditions,
        })
    }
}

fn parse_hunks(delta: &SourceFileDelta) -> Vec<HunkLogic> {
    let mut changes = Vec::<(Vec<String>, Vec<String>)>::new();
    let mut current = None;
    for line in delta.patch.lines() {
        if line.starts_with("@@") {
            changes.push((Vec::new(), Vec::new()));
            current = changes.len().checked_sub(1);
            continue;
        }
        let Some(index) = current else {
            continue;
        };
        let mut characters = line.chars();
        let Some(prefix) = characters.next() else {
            continue;
        };
        if !matches!(prefix, '+' | '-') {
            continue;
        }
        let Some(condition) = condition(characters.as_str()) else {
            continue;
        };
        match prefix {
            '+' => changes[index].0.push(condition),
            '-' => changes[index].1.push(condition),
            _ => {}
        }
    }
    delta
        .hunks
        .iter()
        .copied()
        .zip(changes)
        .map(|(hunk, (added_conditions, removed_conditions))| HunkLogic {
            hunk,
            added_conditions,
            removed_conditions,
        })
        .collect()
}

fn condition(line: &str) -> Option<String> {
    let line = line
        .trim()
        .trim_start_matches('}')
        .trim()
        .trim_end_matches('{')
        .trim_end_matches(':')
        .trim();
    let branch = [
        "if ", "if(", "if let ", "else if ", "elif ", "guard ", "when ", "switch ", "switch(",
        "match ",
    ]
    .iter()
    .any(|prefix| line.starts_with(prefix));
    branch.then(|| bounded_condition(line))
}

fn bounded_condition(line: &str) -> String {
    let mut characters = line.chars();
    let mut output = characters.by_ref().take(240).collect::<String>();
    if characters.next().is_some() {
        output.pop();
        output.push('…');
    }
    output
}

fn source_file(node: Option<&NodeRecord>) -> Option<&str> {
    node?.attributes.get("source_file")?.as_str()
}

fn line_span(node: Option<&NodeRecord>) -> Option<(u32, u32)> {
    let node = node?;
    let start = line_number(node.attributes.get("line_start")?)?;
    let end = node
        .attributes
        .get("line_end")
        .and_then(line_number)
        .unwrap_or(start);
    Some((start, end.max(start)))
}

fn line_number(value: &Value) -> Option<u32> {
    value
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .or_else(|| value.as_str()?.parse().ok())
}

fn overlaps(span: (u32, u32), start: u32, lines: u32) -> bool {
    let hunk_end = if lines == 0 {
        start
    } else {
        start.saturating_add(lines.saturating_sub(1))
    };
    span.0 <= hunk_end && start <= span.1
}

fn describe_conditions(conditions: &[String]) -> String {
    let label = if conditions.len() == 1 {
        "condition"
    } else {
        "conditions"
    };
    format!(
        "{label} {}",
        conditions
            .iter()
            .map(|condition| format!("`{condition}`"))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn condition_evidence_is_bounded_without_splitting_unicode() {
        let source = format!("if {} {{", "界".repeat(300));
        let parsed = condition(&source);
        assert!(parsed.is_some());
        let parsed = parsed.unwrap_or_default();
        assert_eq!(parsed.chars().count(), 240);
        assert!(parsed.ends_with('…'));
    }

    #[test]
    fn empty_patch_lines_are_ignored() {
        let delta = SourceFileDelta {
            old_path: Some("src/example.go".to_owned()),
            new_path: Some("src/example.go".to_owned()),
            status: compass_history::SourceFileStatus::Modified,
            hunks: vec![SourceHunk {
                old_start: 10,
                old_lines: 2,
                new_start: 10,
                new_lines: 1,
            }],
            patch: "@@ -10,2 +10,1 @@\n\n-if stale {\n".to_owned(),
        };
        let hunks = parse_hunks(&delta);
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].removed_conditions, ["if stale"]);
    }
}
