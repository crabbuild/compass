use std::fs;
use std::path::{Path, PathBuf};

use crate::{FileError, io_error};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSlice {
    pub path: PathBuf,
    pub start: usize,
    pub end: usize,
    pub index: usize,
    pub total: usize,
}

const SPLITTABLE: &[&str] = &["md", "mdx", "markdown", "txt", "rst"];
const BOUNDARIES: &[&str] = &["\n#", "\n\n", "\n"];

fn char_to_byte(text: &str, index: usize) -> usize {
    text.char_indices()
        .nth(index)
        .map_or(text.len(), |(offset, _)| offset)
}

fn best_cut(text: &str, start: usize, end: usize) -> usize {
    let start_byte = char_to_byte(text, start);
    let end_byte = char_to_byte(text, end);
    let window = &text[start_byte..end_byte];
    for separator in BOUNDARIES {
        if let Some(byte_index) = window.rfind(separator)
            && byte_index > 0
        {
            let chars_before = window[..byte_index].chars().count();
            return if *separator == "\n#" {
                start + chars_before + 1
            } else {
                start + chars_before + separator.chars().count()
            };
        }
    }
    end
}

/// Gap-free Python-compatible character ranges covering all input.
pub fn slice_boundaries(text: &str, max_chars: usize) -> Vec<(usize, usize)> {
    let length = text.chars().count();
    if length <= max_chars {
        return vec![(0, length)];
    }
    let mut ranges = Vec::new();
    let mut position = 0;
    while position < length {
        let hard = (position + max_chars).min(length);
        let mut end = if hard < length {
            best_cut(text, position, hard)
        } else {
            length
        };
        if end <= position {
            end = hard;
        }
        ranges.push((position, end));
        position = end;
    }
    ranges
}

pub fn split_file(path: &Path, max_chars: usize) -> Result<Vec<FileSlice>, FileError> {
    let bytes = fs::read(path).map_err(|source| io_error(path, source))?;
    let text = String::from_utf8_lossy(&bytes);
    let splittable = path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|ext| {
            SPLITTABLE
                .iter()
                .any(|candidate| ext.eq_ignore_ascii_case(candidate))
        });
    let ranges = if splittable {
        slice_boundaries(&text, max_chars)
    } else {
        vec![(0, text.chars().count())]
    };
    let total = ranges.len();
    Ok(ranges
        .into_iter()
        .enumerate()
        .map(|(index, (start, end))| FileSlice {
            path: path.to_path_buf(),
            start,
            end,
            index,
            total,
        })
        .collect())
}

pub fn read_slice_text(slice: &FileSlice) -> Result<String, FileError> {
    let bytes = fs::read(&slice.path).map_err(|source| io_error(&slice.path, source))?;
    let text = String::from_utf8_lossy(&bytes);
    Ok(text
        .chars()
        .skip(slice.start)
        .take(slice.end.saturating_sub(slice.start))
        .collect())
}

pub fn bisect_slice(slice: &FileSlice) -> Result<Option<(FileSlice, FileSlice)>, FileError> {
    if slice.end.saturating_sub(slice.start) <= 1 {
        return Ok(None);
    }
    let bytes = fs::read(&slice.path).map_err(|source| io_error(&slice.path, source))?;
    let text = String::from_utf8_lossy(&bytes).chars().collect::<Vec<_>>();
    let midpoint = (slice.start + slice.end) / 2;
    let cut = text[midpoint..slice.end]
        .iter()
        .position(|character| *character == '\n')
        .map_or(midpoint, |offset| midpoint + offset + 1);
    if cut <= slice.start || cut >= slice.end {
        return Ok(None);
    }
    Ok(Some((
        FileSlice {
            end: cut,
            ..slice.clone()
        },
        FileSlice {
            start: cut,
            ..slice.clone()
        },
    )))
}
