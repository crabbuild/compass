use super::model::ArchitectureRelationClass;

#[must_use]
pub fn classify_relation(relation: &str) -> ArchitectureRelationClass {
    match relation.trim().to_ascii_lowercase().as_str() {
        "calls" | "handles" | "invokes" | "dispatches" | "routes_to" | "mounts" | "mounted_at"
        | "reads" | "writes" | "produces" | "consumes" | "publishes" | "subscribes" | "sends"
        | "receives" | "schedules" | "triggers" => ArchitectureRelationClass::Execution,
        "imports" | "imports_from" | "exports" | "depends_on" | "uses" | "configured_by"
        | "registers" | "registered_as" | "maps_to" => ArchitectureRelationClass::Dependency,
        "extends" | "implements" | "overrides" | "type_of" | "returns" | "instantiates"
        | "method" | "mixes_in" => ArchitectureRelationClass::Type,
        "contains" | "declares" | "owns" | "part_of" | "embeds" => {
            ArchitectureRelationClass::Structure
        }
        "references" | "tests" | "documents" | "aliases" | "decorates" | "derived_from" => {
            ArchitectureRelationClass::Contextual
        }
        _ => ArchitectureRelationClass::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relation_policy_is_explicit_and_conservative() {
        let complete_graph_vocabulary = [
            ("contains", ArchitectureRelationClass::Structure),
            ("embeds", ArchitectureRelationClass::Structure),
            ("calls", ArchitectureRelationClass::Execution),
            ("imports", ArchitectureRelationClass::Dependency),
            ("exports", ArchitectureRelationClass::Dependency),
            ("extends", ArchitectureRelationClass::Type),
            ("implements", ArchitectureRelationClass::Type),
            ("mixes_in", ArchitectureRelationClass::Type),
            ("references", ArchitectureRelationClass::Contextual),
            ("type_of", ArchitectureRelationClass::Type),
            ("returns", ArchitectureRelationClass::Type),
            ("instantiates", ArchitectureRelationClass::Type),
            ("overrides", ArchitectureRelationClass::Type),
            ("decorates", ArchitectureRelationClass::Contextual),
            ("routes_to", ArchitectureRelationClass::Execution),
            ("reads", ArchitectureRelationClass::Execution),
            ("writes", ArchitectureRelationClass::Execution),
            ("aliases", ArchitectureRelationClass::Contextual),
            ("registers", ArchitectureRelationClass::Dependency),
            ("handles", ArchitectureRelationClass::Execution),
            ("publishes", ArchitectureRelationClass::Execution),
            ("subscribes", ArchitectureRelationClass::Execution),
            ("produces", ArchitectureRelationClass::Execution),
            ("consumes", ArchitectureRelationClass::Execution),
            ("schedules", ArchitectureRelationClass::Execution),
            ("triggers", ArchitectureRelationClass::Execution),
            ("tests", ArchitectureRelationClass::Contextual),
            ("depends_on", ArchitectureRelationClass::Dependency),
            ("documents", ArchitectureRelationClass::Contextual),
            ("maps_to", ArchitectureRelationClass::Dependency),
        ];
        for (relation, expected) in complete_graph_vocabulary {
            assert_eq!(classify_relation(relation), expected, "{relation}");
        }
        assert_eq!(
            classify_relation("future_relation"),
            ArchitectureRelationClass::Unknown
        );
    }
}
