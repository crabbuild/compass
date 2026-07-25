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
            if valid_config_file(&scope.root().join(relative)) {
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

fn valid_config_file(path: &std::path::Path) -> bool {
    let Ok(content) = std::fs::read_to_string(path) else {
        return false;
    };
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("json") => serde_json::from_str::<serde_json::Value>(&content).is_ok(),
        Some("toml") => toml::from_str::<toml::Value>(&content).is_ok(),
        _ => false,
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
