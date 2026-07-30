use std::path::Path;

use super::RawFrameworkAnchor;

pub(super) fn text(source: &[u8]) -> &str {
    std::str::from_utf8(source).unwrap_or_default()
}

pub(super) fn anchor(path: &Path, source: &[u8], start: usize, end: usize) -> RawFrameworkAnchor {
    let start = start.min(source.len());
    let end = end.max(start.saturating_add(1)).min(source.len());
    let (start_line, start_column) = line_column(source, start);
    let (end_line, end_column) = line_column(source, end);
    RawFrameworkAnchor {
        source_file: path.to_string_lossy().replace('\\', "/"),
        start_byte: start as u64,
        end_byte: end as u64,
        start_line,
        start_column,
        end_line,
        end_column,
    }
}

pub(super) fn line_anchor(
    path: &Path,
    source: &[u8],
    line_start: usize,
    line: &str,
) -> RawFrameworkAnchor {
    let content_end = line.trim_end_matches(['\r', '\n']).len();
    anchor(
        path,
        source,
        line_start,
        line_start.saturating_add(content_end.max(1)),
    )
}

pub(super) fn line_anchor_at(
    path: &Path,
    source: &[u8],
    line_start: usize,
    line: &str,
    line_number: usize,
) -> RawFrameworkAnchor {
    let start = line_start.min(source.len());
    let content_end = line.trim_end_matches(['\r', '\n']).len().max(1);
    let end = start.saturating_add(content_end).min(source.len());
    let line_number = u32::try_from(line_number).unwrap_or(u32::MAX);
    RawFrameworkAnchor {
        source_file: path.to_string_lossy().replace('\\', "/"),
        start_byte: start as u64,
        end_byte: end as u64,
        start_line: line_number,
        start_column: 0,
        end_line: line_number,
        end_column: u32::try_from(end.saturating_sub(start)).unwrap_or(u32::MAX),
    }
}

pub(super) fn split_top_level(value: &str) -> Vec<&str> {
    let bytes = value.as_bytes();
    let mut parts = Vec::new();
    let mut start = 0;
    let mut stack = Vec::new();
    let mut quote = None;
    let mut escaped = false;
    for (index, byte) in bytes.iter().copied().enumerate() {
        if let Some(active) = quote {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == active {
                quote = None;
            }
            continue;
        }
        match byte {
            b'\'' | b'"' | b'`' => quote = Some(byte),
            b'(' | b'[' | b'{' => stack.push(byte),
            b')' | b']' | b'}' => {
                stack.pop();
            }
            b',' if stack.is_empty() => {
                parts.push(value[start..index].trim());
                start = index + 1;
            }
            _ => {}
        }
    }
    parts.push(value[start..].trim());
    parts
}

pub(super) fn literal(value: &str) -> Option<String> {
    let trimmed = value.trim();
    let quote = trimmed.as_bytes().first().copied()?;
    if !matches!(quote, b'\'' | b'"' | b'`') || trimmed.as_bytes().last().copied() != Some(quote) {
        return None;
    }
    Some(trimmed[1..trimmed.len().saturating_sub(1)].to_owned())
}

pub(super) fn normalize_route_path(path: &str) -> String {
    let mut result = String::with_capacity(path.len().saturating_add(1));
    if !path.starts_with('/') {
        result.push('/');
    }
    result.push_str(path);
    while result.contains("//") {
        result = result.replace("//", "/");
    }
    if result.len() > 1 {
        result = result.trim_end_matches('/').to_owned();
    }
    result
}

pub(super) fn join_route_path(prefix: &str, path: &str) -> String {
    normalize_route_path(&format!(
        "{}/{}",
        prefix.trim_matches('/'),
        path.trim_matches('/')
    ))
}

fn line_column(source: &[u8], offset: usize) -> (u32, u32) {
    let bounded = offset.min(source.len());
    let line_start = source[..bounded]
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |index| index + 1);
    let line = source[..bounded]
        .iter()
        .filter(|byte| **byte == b'\n')
        .count()
        .saturating_add(1);
    (
        u32::try_from(line).unwrap_or(u32::MAX),
        u32::try_from(bounded.saturating_sub(line_start)).unwrap_or(u32::MAX),
    )
}
