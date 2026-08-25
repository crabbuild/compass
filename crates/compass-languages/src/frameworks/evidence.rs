#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum EvidenceKind {
    Import,
    Receiver,
    #[allow(dead_code)]
    DecoratorOrAttribute,
    Macro,
    ConfigurationContract,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum EvidenceStrength {
    Direct,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ActivationEvidence {
    pub framework: &'static str,
    pub kind: EvidenceKind,
    pub identity: String,
    pub strength: EvidenceStrength,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct EvidenceSet {
    entries: Vec<ActivationEvidence>,
}

impl EvidenceSet {
    pub(super) fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub(super) fn direct_if(
        mut self,
        condition: bool,
        framework: &'static str,
        kind: EvidenceKind,
        identity: impl Into<String>,
    ) -> Self {
        if condition {
            self.entries.push(ActivationEvidence {
                framework,
                kind,
                identity: identity.into(),
                strength: EvidenceStrength::Direct,
            });
        }
        self
    }

    pub(super) fn activates(&self, framework: &str) -> bool {
        self.entries.iter().any(|evidence| {
            evidence.framework == framework && evidence.strength == EvidenceStrength::Direct
        })
    }

    #[cfg(test)]
    fn evidence(&self, framework: &str) -> impl Iterator<Item = &ActivationEvidence> {
        self.entries
            .iter()
            .filter(move |evidence| evidence.framework == framework)
    }
}

#[cfg(test)]
mod tests {
    use super::{EvidenceKind, EvidenceSet, EvidenceStrength};

    #[test]
    fn direct_evidence_activates_only_its_framework() {
        let evidence = EvidenceSet::new().direct_if(
            true,
            "laravel",
            EvidenceKind::Import,
            "Illuminate\\Support\\Facades\\Route",
        );

        assert!(evidence.activates("laravel"));
        assert!(!evidence.activates("symfony"));
        assert_eq!(evidence.evidence("laravel").count(), 1);
        assert!(evidence.evidence("laravel").any(|item| {
            item.kind == EvidenceKind::Import && item.strength == EvidenceStrength::Direct
        }));
    }

    #[test]
    fn false_conditions_do_not_record_evidence() {
        let evidence = EvidenceSet::new()
            .direct_if(false, "spring", EvidenceKind::Import, "spring")
            .direct_if(
                false,
                "spring",
                EvidenceKind::DecoratorOrAttribute,
                "@RestController",
            );

        assert!(!evidence.activates("spring"));
        assert_eq!(evidence.evidence("spring").count(), 0);
    }

    #[test]
    fn evidence_kinds_cover_receiver_macro_and_configuration_contracts() {
        let evidence = EvidenceSet::new()
            .direct_if(true, "flask", EvidenceKind::Receiver, "flask.Flask")
            .direct_if(true, "rocket", EvidenceKind::Macro, "rocket::get")
            .direct_if(
                true,
                "play",
                EvidenceKind::ConfigurationContract,
                "conf/routes",
            );

        assert!(
            ["flask", "rocket", "play"]
                .into_iter()
                .all(|framework| evidence.activates(framework))
        );
    }
}
