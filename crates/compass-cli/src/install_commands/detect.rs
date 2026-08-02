use std::collections::BTreeMap;
use std::env;

use super::model::InstallScope;
use super::registry::AgentRegistry;

pub(super) fn detect_agents(
    registry: &AgentRegistry,
    scope: &InstallScope,
) -> BTreeMap<String, Vec<String>> {
    let mut detected = BTreeMap::new();
    for agent in registry.iter() {
        if agent.id == "agents" {
            continue;
        }
        let mut evidence = Vec::new();
        for command in agent.commands {
            if executable_on_path(command) {
                evidence.push(format!("executable:{command}"));
                break;
            }
        }
        for relative in agent.config_paths {
            if valid_detection_path(&scope.root().join(relative)) {
                evidence.push(format!("config:{relative}"));
            }
        }
        let environment = match agent.id {
            "codex" => Some("CODEX_HOME"),
            "claude" => Some("CLAUDE_CONFIG_DIR"),
            _ => None,
        };
        if let Some(name) = environment
            && env::var_os(name).is_some()
        {
            evidence.push(format!("environment:{name}"));
        }
        if !evidence.is_empty() {
            detected.insert(agent.id.to_owned(), evidence);
        }
    }
    detected
}

fn valid_detection_path(path: &std::path::Path) -> bool {
    if path.is_dir() {
        return true;
    }
    if !path.is_file() {
        return false;
    }
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("json") => std::fs::read_to_string(path)
            .is_ok_and(|content| serde_json::from_str::<serde_json::Value>(&content).is_ok()),
        Some("toml") => std::fs::read_to_string(path)
            .is_ok_and(|content| toml::from_str::<toml::Value>(&content).is_ok()),
        _ => true,
    }
}

fn executable_on_path(name: &str) -> bool {
    let Some(path) = env::var_os("PATH") else {
        return false;
    };
    let extensions = if cfg!(windows) {
        env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_owned())
            .split(';')
            .map(str::to_owned)
            .collect::<Vec<_>>()
    } else {
        vec![String::new()]
    };
    env::split_paths(&path).any(|directory| {
        extensions
            .iter()
            .any(|extension| directory.join(format!("{name}{extension}")).is_file())
    })
}

#[cfg(test)]
mod tests {
    use super::valid_detection_path;

    #[test]
    fn detection_accepts_host_directories_and_valid_configuration_files()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let host = directory.path().join(".kiro");
        std::fs::create_dir(&host)?;
        assert!(valid_detection_path(&host));

        let markdown = directory.path().join("copilot-instructions.md");
        std::fs::write(&markdown, "# Copilot\n")?;
        assert!(valid_detection_path(&markdown));

        let json = directory.path().join("settings.json");
        std::fs::write(&json, "{}")?;
        assert!(valid_detection_path(&json));
        std::fs::write(&json, "{invalid")?;
        assert!(!valid_detection_path(&json));

        let toml = directory.path().join("config.toml");
        std::fs::write(&toml, "model = \"test\"\n")?;
        assert!(valid_detection_path(&toml));
        std::fs::write(&toml, "model = [")?;
        assert!(!valid_detection_path(&toml));
        Ok(())
    }
}
