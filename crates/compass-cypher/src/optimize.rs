use std::collections::{BTreeMap, BTreeSet};

use crate::{
    Clause, Expr, ExprKind, LogicalPlan, OptimizationRecord, Pattern, ProjectionClause, QueryPart,
};

#[must_use]
pub fn optimize(mut plan: LogicalPlan) -> LogicalPlan {
    let mut eliminated_path_bindings = 0usize;
    for part in &mut plan.ast.parts {
        eliminated_path_bindings =
            eliminated_path_bindings.saturating_add(eliminate_unused_path_bindings(part));
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
    if eliminated_path_bindings > 0 {
        plan.optimizations.push(OptimizationRecord {
            rule: "unused-path-binding-elimination",
            reason: format!(
                "removed {eliminated_path_bindings} unreferenced path binding(s) before execution"
            ),
        });
    }
    plan
}

fn eliminate_unused_path_bindings(part: &mut QueryPart) -> usize {
    let mut references = BTreeSet::new();
    let mut wildcard_projection = false;
    let mut binding_counts = BTreeMap::<String, usize>::new();

    for clause in &part.clauses {
        match clause {
            Clause::Match(value) => {
                for pattern in &value.patterns {
                    count_binding(&mut binding_counts, pattern.variable.as_deref());
                    count_binding(&mut binding_counts, pattern.start.variable.as_deref());
                    for (_, expression) in &pattern.start.properties {
                        collect_references(expression, &mut references, &mut wildcard_projection);
                    }
                    for chain in &pattern.chains {
                        count_binding(&mut binding_counts, chain.relationship.variable.as_deref());
                        count_binding(&mut binding_counts, chain.node.variable.as_deref());
                        for (_, expression) in &chain.relationship.properties {
                            collect_references(
                                expression,
                                &mut references,
                                &mut wildcard_projection,
                            );
                        }
                        for (_, expression) in &chain.node.properties {
                            collect_references(
                                expression,
                                &mut references,
                                &mut wildcard_projection,
                            );
                        }
                    }
                }
                if let Some(predicate) = &value.predicate {
                    collect_references(predicate, &mut references, &mut wildcard_projection);
                }
            }
            Clause::Unwind(value) => {
                collect_references(&value.expression, &mut references, &mut wildcard_projection);
            }
            Clause::With(value) | Clause::Return(value) => {
                collect_projection_references(value, &mut references, &mut wildcard_projection);
            }
        }
    }

    if wildcard_projection {
        return 0;
    }

    let mut eliminated = 0usize;
    for clause in &mut part.clauses {
        let Clause::Match(value) = clause else {
            continue;
        };
        for pattern in &mut value.patterns {
            let Some(variable) = pattern.variable.as_deref() else {
                continue;
            };
            if binding_counts.get(variable) == Some(&1) && !references.contains(variable) {
                pattern.variable = None;
                eliminated = eliminated.saturating_add(1);
            }
        }
    }
    eliminated
}

fn count_binding(counts: &mut BTreeMap<String, usize>, variable: Option<&str>) {
    if let Some(variable) = variable {
        counts
            .entry(variable.to_owned())
            .and_modify(|count| *count = count.saturating_add(1))
            .or_insert(1);
    }
}

fn collect_projection_references(
    clause: &ProjectionClause,
    references: &mut BTreeSet<String>,
    wildcard_projection: &mut bool,
) {
    for item in &clause.items {
        collect_references(&item.expression, references, wildcard_projection);
    }
    if let Some(predicate) = &clause.predicate {
        collect_references(predicate, references, wildcard_projection);
    }
    for sort in &clause.order_by {
        collect_references(&sort.expression, references, wildcard_projection);
    }
    if let Some(skip) = &clause.skip {
        collect_references(skip, references, wildcard_projection);
    }
    if let Some(limit) = &clause.limit {
        collect_references(limit, references, wildcard_projection);
    }
}

fn collect_references(
    expression: &Expr,
    references: &mut BTreeSet<String>,
    wildcard_projection: &mut bool,
) {
    match &expression.kind {
        ExprKind::Wildcard => *wildcard_projection = true,
        ExprKind::Variable(name) => {
            references.insert(name.clone());
        }
        ExprKind::Property(value, _)
        | ExprKind::LabelTest(value, _)
        | ExprKind::IsNull(value, _)
        | ExprKind::Unary(_, value) => {
            collect_references(value, references, wildcard_projection);
        }
        ExprKind::Index(left, right) | ExprKind::Binary(left, _, right) => {
            collect_references(left, references, wildcard_projection);
            collect_references(right, references, wildcard_projection);
        }
        ExprKind::Slice(value, start, end) => {
            collect_references(value, references, wildcard_projection);
            if let Some(value) = start {
                collect_references(value, references, wildcard_projection);
            }
            if let Some(value) = end {
                collect_references(value, references, wildcard_projection);
            }
        }
        ExprKind::List(values) => {
            for value in values {
                collect_references(value, references, wildcard_projection);
            }
        }
        ExprKind::Map(values) => {
            for (_, value) in values {
                collect_references(value, references, wildcard_projection);
            }
        }
        ExprKind::Function(call) => {
            for argument in &call.arguments {
                collect_references(argument, references, wildcard_projection);
            }
        }
        ExprKind::ListPredicate(value) => {
            collect_references(&value.list, references, wildcard_projection);
            collect_references(&value.predicate, references, wildcard_projection);
        }
        ExprKind::Case(value) => {
            if let Some(operand) = &value.operand {
                collect_references(operand, references, wildcard_projection);
            }
            for (condition, result) in &value.alternatives {
                collect_references(condition, references, wildcard_projection);
                collect_references(result, references, wildcard_projection);
            }
            if let Some(fallback) = &value.fallback {
                collect_references(fallback, references, wildcard_projection);
            }
        }
        ExprKind::Exists(part) => {
            collect_part_references(part, references, wildcard_projection);
        }
        ExprKind::Literal(_) | ExprKind::Parameter(_) => {}
    }
}

fn collect_part_references(
    part: &QueryPart,
    references: &mut BTreeSet<String>,
    wildcard_projection: &mut bool,
) {
    for clause in &part.clauses {
        match clause {
            Clause::Match(value) => {
                for pattern in &value.patterns {
                    for (_, expression) in &pattern.start.properties {
                        collect_references(expression, references, wildcard_projection);
                    }
                    for chain in &pattern.chains {
                        for (_, expression) in &chain.relationship.properties {
                            collect_references(expression, references, wildcard_projection);
                        }
                        for (_, expression) in &chain.node.properties {
                            collect_references(expression, references, wildcard_projection);
                        }
                    }
                }
                if let Some(predicate) = &value.predicate {
                    collect_references(predicate, references, wildcard_projection);
                }
            }
            Clause::Unwind(value) => {
                collect_references(&value.expression, references, wildcard_projection);
            }
            Clause::With(value) | Clause::Return(value) => {
                collect_projection_references(value, references, wildcard_projection);
            }
        }
    }
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
