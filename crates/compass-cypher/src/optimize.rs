use crate::{
    Clause, Expr, ExprKind, LogicalPlan, MatchClause, OptimizationRecord, Pattern,
    ProjectionClause, QueryPart,
};

#[must_use]
pub fn optimize(mut plan: LogicalPlan) -> LogicalPlan {
    for part in &mut plan.ast.parts {
        eliminate_unused_path_bindings(part, &mut plan.optimizations);
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

fn eliminate_unused_path_bindings(
    part: &mut QueryPart,
    optimizations: &mut Vec<OptimizationRecord>,
) {
    let eliminated = part
        .clauses
        .iter()
        .enumerate()
        .flat_map(|(clause_index, clause)| {
            let Clause::Match(match_clause) = clause else {
                return Vec::new();
            };
            match_clause
                .patterns
                .iter()
                .enumerate()
                .filter_map(|(pattern_index, pattern)| {
                    let variable = pattern.variable.as_ref()?;
                    (!path_variable_is_used(
                        part,
                        clause_index,
                        match_clause,
                        pattern_index,
                        variable,
                    ))
                    .then_some((
                        clause_index,
                        pattern_index,
                        variable.clone(),
                    ))
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    for (clause_index, pattern_index, variable) in eliminated {
        let Some(Clause::Match(match_clause)) = part.clauses.get_mut(clause_index) else {
            continue;
        };
        let Some(pattern) = match_clause.patterns.get_mut(pattern_index) else {
            continue;
        };
        pattern.variable = None;
        optimizations.push(OptimizationRecord {
            rule: "unused-path-binding-elimination",
            reason: format!(
                "path variable '{variable}' is not referenced after MATCH and need not be materialized"
            ),
        });
    }
}

fn path_variable_is_used(
    part: &QueryPart,
    clause_index: usize,
    match_clause: &MatchClause,
    pattern_index: usize,
    variable: &str,
) -> bool {
    match_clause
        .predicate
        .as_ref()
        .is_some_and(|predicate| expression_references(predicate, variable))
        || match_clause
            .patterns
            .iter()
            .enumerate()
            .any(|(index, pattern)| {
                pattern_properties_reference(pattern, variable)
                    || (index != pattern_index && pattern_binds(pattern, variable))
            })
        || part.clauses[clause_index.saturating_add(1)..]
            .iter()
            .any(|clause| clause_references(clause, variable))
}

fn clause_references(clause: &Clause, variable: &str) -> bool {
    match clause {
        Clause::Match(value) => {
            value
                .predicate
                .as_ref()
                .is_some_and(|predicate| expression_references(predicate, variable))
                || value.patterns.iter().any(|pattern| {
                    pattern_binds(pattern, variable)
                        || pattern_properties_reference(pattern, variable)
                })
        }
        Clause::Unwind(value) => {
            value.variable == variable || expression_references(&value.expression, variable)
        }
        Clause::With(value) | Clause::Return(value) => projection_references(value, variable),
    }
}

fn pattern_binds(pattern: &Pattern, variable: &str) -> bool {
    pattern.variable.as_deref() == Some(variable)
        || pattern.start.variable.as_deref() == Some(variable)
        || pattern.chains.iter().any(|chain| {
            chain.relationship.variable.as_deref() == Some(variable)
                || chain.node.variable.as_deref() == Some(variable)
        })
}

fn pattern_properties_reference(pattern: &Pattern, variable: &str) -> bool {
    pattern
        .start
        .properties
        .iter()
        .any(|(_, expression)| expression_references(expression, variable))
        || pattern.chains.iter().any(|chain| {
            chain
                .relationship
                .properties
                .iter()
                .any(|(_, expression)| expression_references(expression, variable))
                || chain
                    .node
                    .properties
                    .iter()
                    .any(|(_, expression)| expression_references(expression, variable))
        })
}

fn projection_references(projection: &ProjectionClause, variable: &str) -> bool {
    projection
        .items
        .iter()
        .any(|item| item.is_wildcard() || expression_references(&item.expression, variable))
        || projection
            .predicate
            .as_ref()
            .is_some_and(|expression| expression_references(expression, variable))
        || projection
            .order_by
            .iter()
            .any(|item| expression_references(&item.expression, variable))
        || projection
            .skip
            .as_ref()
            .is_some_and(|expression| expression_references(expression, variable))
        || projection
            .limit
            .as_ref()
            .is_some_and(|expression| expression_references(expression, variable))
}

fn expression_references(expression: &Expr, variable: &str) -> bool {
    match &expression.kind {
        ExprKind::Variable(name) => name == variable,
        ExprKind::Property(value, _)
        | ExprKind::LabelTest(value, _)
        | ExprKind::IsNull(value, _)
        | ExprKind::Unary(_, value) => expression_references(value, variable),
        ExprKind::Index(left, right) | ExprKind::Binary(left, _, right) => {
            expression_references(left, variable) || expression_references(right, variable)
        }
        ExprKind::Slice(value, start, end) => {
            expression_references(value, variable)
                || start
                    .as_ref()
                    .is_some_and(|value| expression_references(value, variable))
                || end
                    .as_ref()
                    .is_some_and(|value| expression_references(value, variable))
        }
        ExprKind::List(values) => values
            .iter()
            .any(|value| expression_references(value, variable)),
        ExprKind::Map(values) => values
            .iter()
            .any(|(_, value)| expression_references(value, variable)),
        ExprKind::Function(call) => call
            .arguments
            .iter()
            .any(|value| expression_references(value, variable)),
        ExprKind::ListPredicate(value) => {
            expression_references(&value.list, variable)
                || (value.variable != variable && expression_references(&value.predicate, variable))
        }
        ExprKind::Case(value) => {
            value
                .operand
                .as_ref()
                .is_some_and(|operand| expression_references(operand, variable))
                || value.alternatives.iter().any(|(condition, result)| {
                    expression_references(condition, variable)
                        || expression_references(result, variable)
                })
                || value
                    .fallback
                    .as_ref()
                    .is_some_and(|fallback| expression_references(fallback, variable))
        }
        ExprKind::Exists(part) => part
            .clauses
            .iter()
            .any(|clause| clause_references(clause, variable)),
        ExprKind::Wildcard | ExprKind::Literal(_) | ExprKind::Parameter(_) => false,
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
