use crate::{Clause, ExprKind, LogicalPlan, OptimizationRecord, Pattern};

#[must_use]
pub fn optimize(mut plan: LogicalPlan) -> LogicalPlan {
    for part in &mut plan.ast.parts {
        for clause in &mut part.clauses {
            let Clause::Match(value) = clause else {
                continue;
            };
            let before = value
                .patterns
                .iter()
                .map(|pattern| pattern.span.start)
                .collect::<Vec<_>>();
            value
                .patterns
                .sort_by_key(|pattern| (pattern_selectivity(pattern), pattern.span.start));
            let after = value
                .patterns
                .iter()
                .map(|pattern| pattern.span.start)
                .collect::<Vec<_>>();
            if before != after {
                plan.optimizations.push(OptimizationRecord {
                    rule: "deterministic-pattern-order",
                    reason: "anchored and typed patterns execute before unconstrained scans"
                        .to_owned(),
                });
            }
            if value
                .patterns
                .iter()
                .any(|pattern| exact_id_anchor(pattern).is_some())
            {
                plan.optimizations.push(OptimizationRecord {
                    rule: "exact-id-anchor",
                    reason: "node id equality maps to the immutable id index".to_owned(),
                });
            }
            if value.predicate.is_some() {
                plan.optimizations.push(OptimizationRecord {
                    rule: "match-filter-fusion",
                    reason: "the MATCH predicate is evaluated before downstream projection"
                        .to_owned(),
                });
            }
        }
    }
    plan
}

fn pattern_selectivity(pattern: &Pattern) -> u8 {
    if exact_id_anchor(pattern).is_some() {
        0
    } else if !pattern.start.labels.is_empty() && !pattern.start.properties.is_empty() {
        1
    } else if !pattern.start.labels.is_empty() {
        2
    } else if pattern
        .chains
        .iter()
        .any(|chain| !chain.relationship.types.is_empty())
    {
        3
    } else {
        4
    }
}

fn exact_id_anchor(pattern: &Pattern) -> Option<&str> {
    pattern.start.properties.iter().find_map(|(key, value)| {
        if key == "id"
            && let ExprKind::Literal(crate::CompassValue::String(value)) = &value.kind
        {
            Some(value.as_ref())
        } else {
            None
        }
    })
}
