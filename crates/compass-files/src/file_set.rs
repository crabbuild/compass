use std::path::{Component, Path, PathBuf};

use glob::{MatchOptions, Pattern};

use crate::FileError;

/// A bounded, ignore-neutral matcher for a statically declared file set.
///
/// The matcher never walks the filesystem. Callers provide the already
/// discovered, ignore-filtered candidates, which keeps file-set semantics in
/// the filesystem boundary without allowing a language pack to perform an
/// unbounded scan.
#[derive(Clone, Debug)]
pub struct FileSetMatcher {
    root: PathBuf,
    includes: Vec<FileSetPattern>,
    excludes: Vec<FileSetPattern>,
}

#[derive(Clone, Debug)]
struct FileSetPattern {
    raw: String,
    pattern: Pattern,
}

impl FileSetMatcher {
    pub fn new(
        root: &Path,
        includes: &[String],
        excludes: &[String],
        max_patterns: usize,
    ) -> Result<Self, FileError> {
        if includes.is_empty() {
            return Err(FileError::InvalidFileSet {
                pattern: String::new(),
                reason: "at least one include pattern is required".to_owned(),
            });
        }
        let pattern_count = includes.len().saturating_add(excludes.len());
        if pattern_count > max_patterns {
            return Err(FileError::FileSetLimit {
                kind: "patterns",
                observed: pattern_count,
                maximum: max_patterns,
            });
        }
        let root = std::fs::canonicalize(root).map_err(|source| crate::io_error(root, source))?;
        Ok(Self {
            root,
            includes: includes
                .iter()
                .map(|pattern| FileSetPattern::new(pattern))
                .collect::<Result<_, _>>()?,
            excludes: excludes
                .iter()
                .map(|pattern| FileSetPattern::new(pattern))
                .collect::<Result<_, _>>()?,
        })
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub fn matches(&self, path: &Path) -> bool {
        let Ok(relative) = relative_path(&self.root, path) else {
            return false;
        };
        self.includes
            .iter()
            .any(|pattern| pattern.matches(&relative))
            && !self
                .excludes
                .iter()
                .any(|pattern| pattern.matches(&relative))
    }

    /// Filter a bounded candidate set in deterministic relative-path order.
    /// `max_matches_per_pattern` applies before de-duplication, while
    /// `max_total` bounds the published dependency fan-out.
    pub fn match_paths<'a>(
        &self,
        paths: impl IntoIterator<Item = &'a Path>,
        max_matches_per_pattern: usize,
        max_total: usize,
    ) -> Result<Vec<PathBuf>, FileError> {
        let mut candidates = paths
            .into_iter()
            .filter_map(|path| {
                relative_path(&self.root, path)
                    .ok()
                    .map(|relative| (relative, path.to_path_buf()))
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| left.0.cmp(&right.0));
        candidates.dedup_by(|left, right| left.0 == right.0);

        let mut matched = Vec::new();
        for include in &self.includes {
            let mut count = 0usize;
            for (relative, path) in &candidates {
                if !include.matches(relative)
                    || self
                        .excludes
                        .iter()
                        .any(|exclude| exclude.matches(relative))
                {
                    continue;
                }
                count = count.saturating_add(1);
                if count > max_matches_per_pattern {
                    return Err(FileError::FileSetLimit {
                        kind: "matches_per_pattern",
                        observed: count,
                        maximum: max_matches_per_pattern,
                    });
                }
                if !matched.iter().any(|existing: &PathBuf| existing == path) {
                    matched.push(path.clone());
                    if matched.len() > max_total {
                        return Err(FileError::FileSetLimit {
                            kind: "total_edges",
                            observed: matched.len(),
                            maximum: max_total,
                        });
                    }
                }
            }
        }
        matched.sort_by_key(|path| {
            relative_path(&self.root, path).unwrap_or_else(|_| path.to_string_lossy().into_owned())
        });
        Ok(matched)
    }
}

impl FileSetPattern {
    fn new(raw: &str) -> Result<Self, FileError> {
        let original = raw.to_owned();
        let mut value = raw.trim().replace('\\', "/");
        while let Some(rest) = value.strip_prefix("./") {
            value = rest.to_owned();
        }
        if value.is_empty() {
            return Err(FileError::InvalidFileSet {
                pattern: original,
                reason: "pattern is empty".to_owned(),
            });
        }
        let path = Path::new(&value);
        if path.is_absolute()
            || is_windows_absolute(&value)
            || path.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(FileError::InvalidFileSet {
                pattern: original,
                reason: "pattern must stay within the project root".to_owned(),
            });
        }
        let pattern = Pattern::new(&value).map_err(|error| FileError::InvalidFileSet {
            pattern: original,
            reason: error.to_string(),
        })?;
        Ok(Self {
            raw: value,
            pattern,
        })
    }

    fn matches(&self, relative: &str) -> bool {
        let options = MatchOptions {
            case_sensitive: true,
            require_literal_separator: true,
            require_literal_leading_dot: true,
        };
        if self.pattern.matches_with(relative, options)
            || self
                .pattern
                .matches_with(relative.trim_end_matches('/'), options)
        {
            return true;
        }
        // `glob::Pattern` treats `**/` as requiring one directory while
        // Vite's documented glob convention treats it as zero or more.
        // Remove one or more `**/` segments and retry the bounded variants.
        let mut variant = self.raw.clone();
        while let Some(index) = variant.find("**/") {
            variant.replace_range(index..index.saturating_add(3), "");
            let Ok(pattern) = Pattern::new(&variant) else {
                break;
            };
            if pattern.matches_with(relative, options) {
                return true;
            }
        }
        false
    }
}

fn relative_path(root: &Path, path: &Path) -> Result<String, ()> {
    let path = if path.is_absolute() {
        std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
    } else {
        root.join(path)
    };
    let relative = path.strip_prefix(root).map_err(|_| ())?;
    let relative = relative.to_string_lossy().replace('\\', "/");
    (!relative.is_empty() && !relative.split('/').any(|part| part == ".."))
        .then_some(relative)
        .ok_or(())
}

fn is_windows_absolute(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/'
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn matches_bounded_candidates_with_negative_patterns() -> Result<(), Box<dyn std::error::Error>>
    {
        let directory = tempdir()?;
        fs::create_dir_all(directory.path().join("src"))?;
        fs::write(directory.path().join("src/a.tsx"), "")?;
        fs::write(directory.path().join("src/b.test.tsx"), "")?;
        let matcher = FileSetMatcher::new(
            directory.path(),
            &["src/**/*.tsx".to_owned()],
            &["src/**/*.test.tsx".to_owned()],
            8,
        )?;
        let paths = [
            directory.path().join("src/a.tsx"),
            directory.path().join("src/b.test.tsx"),
        ];
        assert!(matcher.matches(&paths[0]));
        assert!(!matcher.matches(&paths[1]));
        let matches = matcher.match_paths(paths.iter().map(PathBuf::as_path), 8, 8)?;
        assert_eq!(matches, vec![paths[0].clone()]);
        Ok(())
    }

    #[test]
    fn rejects_escape_and_match_limits() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempdir()?;
        assert!(matches!(
            FileSetMatcher::new(directory.path(), &["../secret".to_owned()], &[], 8),
            Err(FileError::InvalidFileSet { .. })
        ));
        let matcher = FileSetMatcher::new(directory.path(), &["**/*.tsx".to_owned()], &[], 8)?;
        let paths = (0..2)
            .map(|index| directory.path().join(format!("{index}.tsx")))
            .collect::<Vec<_>>();
        for path in &paths {
            fs::write(path, "")?;
        }
        assert!(matches!(
            matcher.match_paths(paths.iter().map(PathBuf::as_path), 1, 8),
            Err(FileError::FileSetLimit {
                kind: "matches_per_pattern",
                ..
            })
        ));
        Ok(())
    }
}
