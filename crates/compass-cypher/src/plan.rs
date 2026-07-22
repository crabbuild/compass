use crate::{Column, Expr, QueryAst};

#[derive(Clone, Debug, PartialEq)]
pub struct LogicalPlan {
    pub ast: QueryAst,
    pub operators: Vec<LogicalOperator>,
    pub columns: Vec<Column>,
    pub optimizations: Vec<OptimizationRecord>,
}

impl LogicalPlan {
    #[must_use]
    pub fn contains_operator(&self, name: &str) -> bool {
        self.operators
            .iter()
            .any(|operator| operator.name() == name)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OptimizationRecord {
    pub rule: &'static str,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LogicalOperator {
    NodeScan {
        variable: String,
        label: Option<String>,
    },
    Expand {
        variable: Option<String>,
        min_hops: usize,
        max_hops: usize,
    },
    Filter {
        predicate: Expr,
    },
    Unwind {
        variable: String,
    },
    Project,
    Aggregate,
    Distinct,
    Sort,
    Skip,
    Limit,
    Optional,
    Union {
        all: bool,
    },
}

impl LogicalOperator {
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::NodeScan { .. } => "NodeScan",
            Self::Expand { .. } => "Expand",
            Self::Filter { .. } => "Filter",
            Self::Unwind { .. } => "Unwind",
            Self::Project => "Project",
            Self::Aggregate => "Aggregate",
            Self::Distinct => "Distinct",
            Self::Sort => "Sort",
            Self::Skip => "Skip",
            Self::Limit => "Limit",
            Self::Optional => "Optional",
            Self::Union { .. } => "Union",
        }
    }
}
