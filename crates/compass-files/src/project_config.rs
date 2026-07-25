use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{BuildScope, FileError, write_text_atomic};

pub const PROJECT_CONFIG_RELATIVE_PATH: &str = ".compass/config.toml";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectConfig {
    pub version: u32,
    #[serde(default)]
    pub build: BuildScope,
}

impl ProjectConfig {
    #[must_use]
    pub fn new(build: BuildScope) -> Self {
        Self { version: 1, build }
    }

    pub fn normalize(mut self, root: &Path) -> Result<Self, FileError> {
        if self.version != 1 {
            return Err(FileError::UnsupportedProjectConfig {
                path: root.join(PROJECT_CONFIG_RELATIVE_PATH),
                version: self.version,
            });
        }
        self.build = self.build.normalize(root)?;
        Ok(self)
    }

    pub fn load(root: &Path) -> Result<Option<Self>, FileError> {
        let path = root.join(PROJECT_CONFIG_RELATIVE_PATH);
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(FileError::Io { path, source }),
        };
        let config: Self =
            toml::from_str(&text).map_err(|source| FileError::ProjectConfigToml {
                path: path.clone(),
                source: Box::new(source),
            })?;
        config.normalize(root).map(Some)
    }

    pub fn write(&self, root: &Path) -> Result<PathBuf, FileError> {
        let config = self.clone().normalize(root)?;
        let path = root.join(PROJECT_CONFIG_RELATIVE_PATH);
        let text = toml::to_string(&config).map_err(|source| FileError::ProjectConfigEncode {
            path: path.clone(),
            source: Box::new(source),
        })?;
        write_text_atomic(&path, &text)?;
        Ok(path)
    }
}
