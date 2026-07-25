use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use super::model::{InstallScope, SupportTier};

#[derive(Clone, Debug)]
pub(super) struct AgentDescriptor {
    pub id: &'static str,
    pub aliases: &'static [&'static str],
    pub tier: SupportTier,
    pub commands: &'static [&'static str],
    pub config_paths: &'static [&'static str],
    pub project_skill: Option<&'static str>,
    pub user_skill: Option<&'static str>,
    pub documentation_url: &'static str,
    pub verified_on: &'static str,
}

impl AgentDescriptor {
    fn shared(
        id: &'static str,
        aliases: &'static [&'static str],
        commands: &'static [&'static str],
        config_paths: &'static [&'static str],
        documentation_url: &'static str,
    ) -> Self {
        Self::skill(
            id,
            aliases,
            SupportTier::SharedSkill,
            commands,
            config_paths,
            ".agents/skills/compass/SKILL.md",
            documentation_url,
        )
    }

    fn native(
        id: &'static str,
        aliases: &'static [&'static str],
        commands: &'static [&'static str],
        config_paths: &'static [&'static str],
        destination: &'static str,
        documentation_url: &'static str,
    ) -> Self {
        Self::skill(
            id,
            aliases,
            SupportTier::NativeSkill,
            commands,
            config_paths,
            destination,
            documentation_url,
        )
    }

    fn adapter(
        id: &'static str,
        aliases: &'static [&'static str],
        commands: &'static [&'static str],
        config_paths: &'static [&'static str],
        destination: Option<&'static str>,
        documentation_url: &'static str,
    ) -> Self {
        Self {
            id,
            aliases,
            tier: SupportTier::AdapterOnly,
            commands,
            config_paths,
            project_skill: destination,
            user_skill: destination,
            documentation_url,
            verified_on: "2026-07-24",
        }
    }

    fn skill(
        id: &'static str,
        aliases: &'static [&'static str],
        tier: SupportTier,
        commands: &'static [&'static str],
        config_paths: &'static [&'static str],
        destination: &'static str,
        documentation_url: &'static str,
    ) -> Self {
        Self {
            id,
            aliases,
            tier,
            commands,
            config_paths,
            project_skill: Some(destination),
            user_skill: Some(destination),
            documentation_url,
            verified_on: "2026-07-24",
        }
    }

    fn with_user_skill(mut self, destination: &'static str) -> Self {
        self.user_skill = Some(destination);
        self
    }

    pub(super) fn skill_destination(&self, scope: &InstallScope) -> Option<PathBuf> {
        if !scope.is_project()
            && self.id == "claude"
            && let Some(directory) = std::env::var_os("CLAUDE_CONFIG_DIR")
        {
            let directory = PathBuf::from(directory);
            let directory = if directory.is_absolute() {
                directory
            } else {
                scope.root().join(directory)
            };
            return Some(directory.join("skills/compass/SKILL.md"));
        }
        let relative = if scope.is_project() {
            self.project_skill
        } else {
            self.user_skill
        }?;
        Some(scope.root().join(relative))
    }
}

pub(super) struct AgentRegistry {
    agents: Vec<AgentDescriptor>,
    lookup: BTreeMap<&'static str, usize>,
}

impl AgentRegistry {
    pub(super) fn new() -> Result<Self, String> {
        let agents = descriptors();
        let mut lookup = BTreeMap::new();
        for (index, agent) in agents.iter().enumerate() {
            if agent.documentation_url.is_empty() || agent.verified_on.is_empty() {
                return Err(format!(
                    "agent '{}' has incomplete source metadata",
                    agent.id
                ));
            }
            for key in std::iter::once(agent.id).chain(agent.aliases.iter().copied()) {
                if lookup.insert(key, index).is_some() {
                    return Err(format!("duplicate agent id or alias '{key}'"));
                }
            }
        }
        Ok(Self { agents, lookup })
    }

    pub(super) fn resolve(&self, name: &str) -> Option<&AgentDescriptor> {
        self.lookup.get(name).map(|index| &self.agents[*index])
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = &AgentDescriptor> {
        self.agents.iter()
    }

    pub(super) fn ids(&self) -> String {
        self.agents
            .iter()
            .map(|agent| agent.id)
            .collect::<Vec<_>>()
            .join(", ")
    }

    pub(super) fn canonicalize(&self, names: &[String]) -> Result<Vec<String>, String> {
        let mut ids = BTreeSet::new();
        for name in names {
            let Some(agent) = self.resolve(name) else {
                return Err(format!(
                    "error: unknown platform '{name}'. Choose from: {}",
                    self.ids()
                ));
            };
            ids.insert(agent.id.to_owned());
        }
        Ok(ids.into_iter().collect())
    }
}

fn descriptors() -> Vec<AgentDescriptor> {
    const AGENT_SKILLS: &str = "https://agentskills.io/specification";
    vec![
        AgentDescriptor::shared("agents", &["skills"], &[], &[".agents"], AGENT_SKILLS),
        AgentDescriptor::shared(
            "codex",
            &[],
            &["codex"],
            &[".codex/config.toml"],
            "https://developers.openai.com/codex/concepts/customization#skills",
        ),
        AgentDescriptor::native(
            "claude",
            &["claude-code", "windows"],
            &["claude"],
            &[".claude/settings.json"],
            ".claude/skills/compass/SKILL.md",
            "https://code.claude.com/docs/en/skills",
        ),
        AgentDescriptor::shared(
            "gemini",
            &[],
            &["gemini"],
            &[".gemini/settings.json"],
            "https://geminicli.com/docs/cli/skills/",
        ),
        AgentDescriptor::shared(
            "opencode",
            &[],
            &["opencode"],
            &[".opencode/opencode.json", "opencode.json"],
            "https://opencode.ai/docs/skills/",
        ),
        AgentDescriptor::shared(
            "copilot",
            &["vscode"],
            &["copilot", "code"],
            &[".github/copilot-instructions.md"],
            "https://docs.github.com/en/copilot/how-tos/copilot-on-github/customize-copilot/customize-cloud-agent/add-skills",
        ),
        AgentDescriptor::native(
            "kiro",
            &[],
            &["kiro", "kiro-cli"],
            &[".kiro"],
            ".kiro/skills/compass/SKILL.md",
            "https://kiro.dev/docs/skills/",
        ),
        AgentDescriptor::native(
            "cline",
            &[],
            &["cline"],
            &[".cline"],
            ".cline/skills/compass/SKILL.md",
            "https://docs.cline.bot/customization/skills",
        ),
        AgentDescriptor::adapter(
            "cursor",
            &[],
            &["cursor"],
            &[".cursor"],
            None,
            "https://docs.cursor.com/context/rules-for-ai",
        ),
        AgentDescriptor::adapter(
            "devin",
            &["windsurf"],
            &["windsurf"],
            &[".windsurf"],
            Some(".devin/skills/compass/SKILL.md"),
            "https://docs.windsurf.com/windsurf/cascade/memories",
        )
        .with_user_skill(".config/devin/skills/compass/SKILL.md"),
        AgentDescriptor::adapter(
            "kilo",
            &[],
            &["kilo"],
            &[".kilo"],
            Some(".config/kilo/skills/compass/SKILL.md"),
            AGENT_SKILLS,
        ),
        AgentDescriptor::adapter(
            "aider",
            &[],
            &["aider"],
            &[".aider"],
            Some(".aider/compass/SKILL.md"),
            AGENT_SKILLS,
        ),
        AgentDescriptor::adapter(
            "claw",
            &["openclaw"],
            &["openclaw"],
            &[".openclaw"],
            Some(".openclaw/skills/compass/SKILL.md"),
            AGENT_SKILLS,
        ),
        AgentDescriptor::adapter(
            "droid",
            &[],
            &["droid"],
            &[".factory"],
            Some(".factory/skills/compass/SKILL.md"),
            AGENT_SKILLS,
        ),
        AgentDescriptor::adapter(
            "trae",
            &[],
            &["trae"],
            &[".trae"],
            Some(".trae/skills/compass/SKILL.md"),
            AGENT_SKILLS,
        ),
        AgentDescriptor::adapter(
            "trae-cn",
            &[],
            &["trae"],
            &[".trae-cn"],
            Some(".trae-cn/skills/compass/SKILL.md"),
            AGENT_SKILLS,
        ),
        AgentDescriptor::adapter(
            "hermes",
            &[],
            &["hermes"],
            &[".hermes"],
            Some(".hermes/skills/compass/SKILL.md"),
            AGENT_SKILLS,
        ),
        AgentDescriptor::adapter(
            "pi",
            &[],
            &["pi"],
            &[".pi"],
            Some(".pi/agent/skills/compass/SKILL.md"),
            AGENT_SKILLS,
        ),
        AgentDescriptor::adapter(
            "codebuddy",
            &[],
            &["codebuddy"],
            &[".codebuddy"],
            Some(".codebuddy/skills/compass/SKILL.md"),
            AGENT_SKILLS,
        ),
        AgentDescriptor::adapter(
            "antigravity",
            &["antigravity-windows"],
            &["antigravity"],
            &[".gemini/config"],
            Some(".agents/skills/compass/SKILL.md"),
            AGENT_SKILLS,
        ),
        AgentDescriptor::adapter(
            "amp",
            &[],
            &["amp"],
            &[".config/agents"],
            Some(".agents/skills/compass/SKILL.md"),
            AGENT_SKILLS,
        ),
        AgentDescriptor::adapter(
            "kimi",
            &[],
            &["kimi"],
            &[".kimi"],
            Some(".kimi/skills/compass/SKILL.md"),
            AGENT_SKILLS,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{AgentRegistry, SupportTier};
    use crate::install_commands::model::InstallScope;

    #[test]
    fn registry_is_unique_and_core_destinations_are_portable() -> Result<(), String> {
        let registry = AgentRegistry::new()?;
        assert_eq!(
            registry.resolve("skills").map(|agent| agent.id),
            Some("agents")
        );
        assert_eq!(
            registry.resolve("claude-code").map(|agent| agent.id),
            Some("claude")
        );
        let scope = InstallScope::Project(Path::new("/repo").to_path_buf());
        for id in ["codex", "gemini", "opencode", "copilot", "agents"] {
            let agent = registry
                .resolve(id)
                .ok_or_else(|| format!("missing registered agent {id}"))?;
            assert_eq!(agent.tier, SupportTier::SharedSkill);
            assert_eq!(
                agent.skill_destination(&scope).as_deref(),
                Some(Path::new("/repo/.agents/skills/compass/SKILL.md"))
            );
        }
        Ok(())
    }
}
