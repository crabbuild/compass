#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SupportRecord {
    pub id: &'static str,
    pub category: &'static str,
    pub supported: bool,
}

const SUPPORT: &[SupportRecord] = &[
    SupportRecord {
        id: "clause.match",
        category: "clause",
        supported: true,
    },
    SupportRecord {
        id: "clause.optional_match",
        category: "clause",
        supported: true,
    },
    SupportRecord {
        id: "clause.where",
        category: "clause",
        supported: true,
    },
    SupportRecord {
        id: "clause.unwind",
        category: "clause",
        supported: true,
    },
    SupportRecord {
        id: "clause.with",
        category: "clause",
        supported: true,
    },
    SupportRecord {
        id: "clause.return",
        category: "clause",
        supported: true,
    },
    SupportRecord {
        id: "clause.union",
        category: "clause",
        supported: true,
    },
    SupportRecord {
        id: "path.bounded",
        category: "path",
        supported: true,
    },
    SupportRecord {
        id: "path.unbounded",
        category: "path",
        supported: false,
    },
    SupportRecord {
        id: "mutation",
        category: "clause",
        supported: false,
    },
];

#[must_use]
pub fn supported_features() -> &'static [SupportRecord] {
    SUPPORT
}
