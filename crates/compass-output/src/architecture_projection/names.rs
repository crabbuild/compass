use std::collections::{BTreeMap, BTreeSet};

use compass_model::NodeRecord;
use sha1::{Digest, Sha1};

use super::model::{ArchitectureGroupName, ArchitectureNameProvenance};

const GENERIC_WORDS: &[&str] = &[
    "src",
    "lib",
    "crates",
    "packages",
    "apps",
    "services",
    "module",
    "mod",
    "index",
    "main",
    "core",
    "code",
    "project",
    "repository",
    "compass",
];

#[must_use]
pub fn stable_fragment(value: &str) -> String {
    format!("{:x}", Sha1::digest(value.as_bytes()))[..12].to_owned()
}

#[must_use]
pub fn membership_signature(ids: &[String]) -> String {
    let mut sorted = ids.to_vec();
    sorted.sort();
    let mut hasher = Sha1::new();
    for id in sorted {
        hasher.update(id.as_bytes());
        hasher.update([0]);
    }
    format!("{:x}", hasher.finalize())[..16].to_owned()
}

#[must_use]
pub fn owner_display_name(owner_key: &str) -> String {
    let terminal = owner_key
        .rsplit('/')
        .next()
        .unwrap_or(owner_key)
        .trim_start_matches("compass-");
    humanize(terminal)
}

#[must_use]
pub fn community_name(
    community: Option<usize>,
    owner_name: &str,
    node_ids: &[String],
    nodes: &BTreeMap<String, &NodeRecord>,
    labels: Option<&BTreeMap<usize, String>>,
    max_evidence: usize,
) -> ArchitectureGroupName {
    let signature = membership_signature(node_ids);
    if let Some((label, community)) = community
        .and_then(|community| {
            labels
                .and_then(|labels| labels.get(&community))
                .map(|label| (label, community))
        })
        .filter(|(label, _)| {
            acceptable_name(label) && !same_meaning_after_scaffolding(label, owner_name)
        })
    {
        return ArchitectureGroupName {
            value: label.trim().to_owned(),
            provenance: ArchitectureNameProvenance::Persisted,
            membership_signature: signature,
            quality: 90,
            evidence: vec![format!("community:{community}")],
        };
    }
    let mut path_counts = BTreeMap::<String, usize>::new();
    let mut declaration_counts = BTreeMap::<String, usize>::new();
    for id in node_ids {
        let Some(node) = nodes.get(id) else {
            continue;
        };
        let source = node.string("source_file").replace('\\', "/");
        if let Some(stem) = source.rsplit('/').next().and_then(file_stem)
            && acceptable_name(stem)
        {
            *path_counts.entry(humanize(stem)).or_default() += 1;
        }
        let label = node.label().trim().trim_end_matches("()");
        if acceptable_name(label) {
            *declaration_counts.entry(humanize(label)).or_default() += 1;
        }
    }
    let best_path = best_counted(&path_counts);
    let best_declaration = best_counted(&declaration_counts);
    let mut evidence = Vec::new();
    let (value, provenance, quality) = if let Some(path) = best_path {
        evidence.push(format!("path:{path}"));
        (
            qualified(owner_name, path),
            ArchitectureNameProvenance::Path,
            80,
        )
    } else if let Some(declaration) = best_declaration {
        evidence.push(format!("declaration:{declaration}"));
        (
            qualified(owner_name, declaration),
            ArchitectureNameProvenance::Declaration,
            72,
        )
    } else {
        let suffix =
            community.map_or_else(|| stable_fragment(&signature), |value| value.to_string());
        (
            format!("Unnamed subsystem · {owner_name} {suffix}"),
            ArchitectureNameProvenance::Fallback,
            30,
        )
    };
    evidence.truncate(max_evidence);
    ArchitectureGroupName {
        value,
        provenance,
        membership_signature: signature,
        quality,
        evidence,
    }
}

#[must_use]
pub fn owner_name(
    value: String,
    node_ids: &[String],
    provenance: ArchitectureNameProvenance,
    evidence: Vec<String>,
) -> ArchitectureGroupName {
    ArchitectureGroupName {
        value,
        provenance,
        membership_signature: membership_signature(node_ids),
        quality: if matches!(provenance, ArchitectureNameProvenance::Overlay) {
            100
        } else {
            92
        },
        evidence,
    }
}

pub fn disambiguate_names(names: &mut [ArchitectureGroupName], owner_keys: &[String]) {
    let counts = names
        .iter()
        .fold(BTreeMap::<String, usize>::new(), |mut counts, name| {
            *counts.entry(name.value.to_ascii_lowercase()).or_default() += 1;
            counts
        });
    for (index, name) in names.iter_mut().enumerate() {
        if counts
            .get(&name.value.to_ascii_lowercase())
            .copied()
            .unwrap_or_default()
            > 1
        {
            let owner = owner_keys.get(index).map_or("group", String::as_str);
            name.value = format!("{} · {}", name.value, owner);
            name.evidence.push(format!("owner:{owner}"));
            name.evidence.sort();
            name.evidence.dedup();
        }
    }
    let mut seen = BTreeSet::new();
    for name in names {
        if !seen.insert(name.value.to_ascii_lowercase()) {
            name.value = format!(
                "{} · {}",
                name.value,
                stable_fragment(&name.membership_signature)
            );
        }
    }
}

fn best_counted(counts: &BTreeMap<String, usize>) -> Option<&str> {
    counts
        .iter()
        .max_by(|(left_name, left_count), (right_name, right_count)| {
            left_count
                .cmp(right_count)
                .then_with(|| right_name.cmp(left_name))
        })
        .map(|(name, _)| name.as_str())
}

fn qualified(owner: &str, candidate: &str) -> String {
    if candidate.eq_ignore_ascii_case(owner)
        || candidate
            .to_ascii_lowercase()
            .contains(&owner.to_ascii_lowercase())
    {
        candidate.to_owned()
    } else {
        format!("{owner} · {candidate}")
    }
}

fn file_stem(filename: &str) -> Option<&str> {
    filename.rsplit_once('.').map_or_else(
        || (!filename.is_empty()).then_some(filename),
        |(stem, _)| (!stem.is_empty()).then_some(stem),
    )
}

fn acceptable_name(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.len() < 2 || trimmed.len() > 96 {
        return false;
    }
    let words = trimmed
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>();
    if words.is_empty()
        || words
            .iter()
            .all(|word| GENERIC_WORDS.contains(&word.as_str()))
    {
        return false;
    }
    let alphanumeric = trimmed
        .chars()
        .filter(|character| character.is_alphanumeric())
        .count();
    let vowels = trimmed
        .chars()
        .filter(|character| "aeiouAEIOU".contains(*character))
        .count();
    alphanumeric < 24 || vowels > 0
}

fn same_meaning_after_scaffolding(left: &str, right: &str) -> bool {
    let meaningful = |value: &str| {
        value
            .split(|character: char| !character.is_alphanumeric())
            .filter(|word| !word.is_empty())
            .map(str::to_ascii_lowercase)
            .filter(|word| !GENERIC_WORDS.contains(&word.as_str()))
            .collect::<Vec<_>>()
    };
    meaningful(left) == meaningful(right)
}

fn humanize(value: &str) -> String {
    let mut output = String::new();
    let mut previous_lowercase = false;
    for character in value.chars() {
        if matches!(character, '_' | '-' | '.') {
            if !output.ends_with(' ') && !output.is_empty() {
                output.push(' ');
            }
            previous_lowercase = false;
            continue;
        }
        if character.is_ascii_uppercase() && previous_lowercase && !output.ends_with(' ') {
            output.push(' ');
        }
        if output.is_empty() || output.ends_with(' ') {
            output.extend(character.to_uppercase());
        } else {
            output.push(character);
        }
        previous_lowercase = character.is_ascii_lowercase();
    }
    output.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_names_remove_repository_scaffolding() {
        assert_eq!(owner_display_name("crates/compass-output"), "Output");
        assert_eq!(owner_display_name("packages/compass-viewer"), "Viewer");
    }

    #[test]
    fn duplicate_names_gain_owner_evidence() {
        let mut names = vec![
            owner_name(
                "Runtime".to_owned(),
                &["a".to_owned()],
                ArchitectureNameProvenance::Owner,
                Vec::new(),
            ),
            owner_name(
                "Runtime".to_owned(),
                &["b".to_owned()],
                ArchitectureNameProvenance::Owner,
                Vec::new(),
            ),
        ];
        disambiguate_names(&mut names, &["crates/a".to_owned(), "crates/b".to_owned()]);
        assert_ne!(names[0].value, names[1].value);
    }

    #[test]
    fn scaffold_only_persisted_label_does_not_repeat_owner() {
        assert!(same_meaning_after_scaffolding(
            "Crates Compass Output",
            "Output"
        ));
        assert!(!same_meaning_after_scaffolding(
            "Output Rendering",
            "Output"
        ));
    }
}
