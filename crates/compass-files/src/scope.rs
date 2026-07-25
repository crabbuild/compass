use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use glob::{MatchOptions, Pattern};
use serde::{Deserialize, Serialize};

use crate::FileError;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BuildScope {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
}

impl BuildScope {
    pub fn normalize(mut self, root: &Path) -> Result<Self, FileError> {
        self.include = normalize_entries(root, self.include)?;
        self.exclude = normalize_entries(root, self.exclude)?;
        Ok(self)
    }
}

fn normalize_entries(root: &Path, entries: Vec<String>) -> Result<Vec<String>, FileError> {
    let canonical_root =
        std::fs::canonicalize(root).map_err(|source| crate::io_error(root, source))?;
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for original in entries {
        let directory_hint = original.ends_with(['/', '\\']);
        let mut value = original.trim().replace('\\', "/");
        while let Some(rest) = value.strip_prefix("./") {
            value = rest.to_owned();
        }
        if value.is_empty() {
            return Err(FileError::InvalidScope {
                entry: original,
                reason: "entry is empty".to_owned(),
            });
        }
        let path = Path::new(&value);
        if path.is_absolute()
            || path.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(FileError::InvalidScope {
                entry: original,
                reason: "entry must stay within the project root".to_owned(),
            });
        }
        Pattern::new(value.trim_end_matches('/')).map_err(|error| FileError::InvalidScope {
            entry: original.clone(),
            reason: error.to_string(),
        })?;
        let candidate = root.join(value.trim_end_matches('/'));
        if candidate.exists() {
            let canonical = std::fs::canonicalize(&candidate)
                .map_err(|source| crate::io_error(&candidate, source))?;
            if !canonical.starts_with(&canonical_root) {
                return Err(FileError::InvalidScope {
                    entry: original,
                    reason: "entry resolves outside the project root".to_owned(),
                });
            }
            if canonical.is_dir() && !value.ends_with('/') {
                value.push('/');
            }
        } else if directory_hint && !value.ends_with('/') {
            value.push('/');
        }
        if seen.insert(value.clone()) {
            normalized.push(value);
        }
    }
    Ok(normalized)
}

#[derive(Clone, Debug)]
struct ScopePattern {
    raw: String,
    literal: bool,
    directory: bool,
    pattern: Pattern,
}

impl ScopePattern {
    fn new(raw: &str) -> Result<Self, FileError> {
        let trimmed = raw.trim_end_matches('/');
        let literal = !trimmed.contains(['*', '?', '[']);
        Ok(Self {
            raw: trimmed.to_owned(),
            literal,
            directory: raw.ends_with('/'),
            pattern: Pattern::new(trimmed).map_err(|error| FileError::InvalidScope {
                entry: raw.to_owned(),
                reason: error.to_string(),
            })?,
        })
    }

    fn matches(&self, relative: &str) -> bool {
        if self.literal {
            return relative == self.raw
                || (self.directory
                    && relative
                        .strip_prefix(&self.raw)
                        .is_some_and(|rest| rest.starts_with('/')));
        }
        let options = MatchOptions {
            case_sensitive: true,
            require_literal_separator: true,
            require_literal_leading_dot: true,
        };
        if self.pattern.matches_with(relative, options) {
            return true;
        }
        let mut ancestor = relative;
        while let Some((parent, _)) = ancestor.rsplit_once('/') {
            if self.pattern.matches_with(parent, options) {
                return true;
            }
            ancestor = parent;
        }
        false
    }
}

#[derive(Clone, Debug)]
pub struct ScopeMatcher {
    root: PathBuf,
    includes: Vec<ScopePattern>,
    excludes: Vec<ScopePattern>,
}

impl ScopeMatcher {
    pub fn new(root: &Path, scope: &BuildScope) -> Result<Self, FileError> {
        let root = std::fs::canonicalize(root).map_err(|source| crate::io_error(root, source))?;
        Ok(Self {
            root,
            includes: scope
                .include
                .iter()
                .map(|entry| ScopePattern::new(entry))
                .collect::<Result<_, _>>()?,
            excludes: scope
                .exclude
                .iter()
                .map(|entry| ScopePattern::new(entry))
                .collect::<Result<_, _>>()?,
        })
    }

    #[must_use]
    pub fn allows(&self, path: &Path) -> bool {
        let Ok(relative) = path.strip_prefix(&self.root) else {
            return false;
        };
        let relative = relative.to_string_lossy().replace('\\', "/");
        (self.includes.is_empty() || self.includes.iter().any(|rule| rule.matches(&relative)))
            && !self.excludes.iter().any(|rule| rule.matches(&relative))
    }

    #[must_use]
    pub fn unmatched_includes<'a>(&self, paths: impl IntoIterator<Item = &'a Path>) -> Vec<String> {
        let paths = paths.into_iter().collect::<Vec<_>>();
        self.includes
            .iter()
            .filter(|rule| {
                !paths.iter().any(|path| {
                    path.strip_prefix(&self.root)
                        .ok()
                        .map(|relative| relative.to_string_lossy().replace('\\', "/"))
                        .is_some_and(|relative| rule.matches(&relative))
                })
            })
            .map(|rule| rule.raw.clone())
            .collect()
    }
}
