use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use compass_cypher::{
    BinaryOp, CaseExpr, CompassValue, Expr, ExprKind, FunctionCall, ListPredicateKind,
    ProjectionClause, UnaryOp,
};
use regex::Regex;
use serde_json::Value;

use super::error::{QueryError, QueryErrorKind};
use super::execute::{BindingRow, ExecutionContext, execute_exists_part};

pub(super) fn project_rows(
    rows: Vec<BindingRow>,
    clause: &ProjectionClause,
    context: &mut ExecutionContext<'_>,
) -> Result<Vec<BindingRow>, QueryError> {
    let aggregate = clause
        .items
        .iter()
        .any(|item| contains_aggregate(&item.expression));
    let mut output = if aggregate {
        project_aggregate(rows, clause, context)?
    } else {
        let mut projected = Vec::with_capacity(rows.len());
        for row in &rows {
            context.checkpoint()?;
            let mut next = BindingRow::new();
            for item in &clause.items {
                if item.is_wildcard() {
                    next.extend(row.clone());
                } else {
                    next.insert(
                        item.output_name(),
                        eval(&item.expression, row, None, context)?,
                    );
                }
            }
            projected.push(next);
        }
        projected
    };

    if let Some(predicate) = &clause.predicate {
        let mut filtered = Vec::with_capacity(output.len());
        for row in output {
            if truthy(&eval(predicate, &row, None, context)?) == Some(true) {
                filtered.push(row);
            }
        }
        output = filtered;
    }

    if clause.distinct {
        let mut seen = BTreeSet::new();
        output.retain(|row| seen.insert(canonical_row(row)));
    }

    if !clause.order_by.is_empty() {
        let mut decorated = Vec::with_capacity(output.len());
        for row in output {
            let mut keys = Vec::with_capacity(clause.order_by.len());
            for sort in &clause.order_by {
                keys.push(eval(&sort.expression, &row, None, context)?);
            }
            decorated.push((keys, row));
        }
        decorated.sort_by(|(left, _), (right, _)| {
            for (index, sort) in clause.order_by.iter().enumerate() {
                let order = compare_values(&left[index], &right[index]);
                if order != Ordering::Equal {
                    return if sort.descending {
                        order.reverse()
                    } else {
                        order
                    };
                }
            }
            Ordering::Equal
        });
        output = decorated.into_iter().map(|(_, row)| row).collect();
    }

    let empty = BindingRow::new();
    let skip = clause
        .skip
        .as_ref()
        .map(|value| eval_non_negative_usize(value, &empty, context, "SKIP"))
        .transpose()?
        .unwrap_or(0);
    let limit = clause
        .limit
        .as_ref()
        .map(|value| eval_non_negative_usize(value, &empty, context, "LIMIT"))
        .transpose()?;
    let iter = output.into_iter().skip(skip);
    let mut output = if let Some(limit) = limit {
        iter.take(limit).collect::<Vec<_>>()
    } else {
        iter.collect::<Vec<_>>()
    };
    context.reserve_bindings(&output)?;
    if output.len() > context.limits.max_rows {
        return Err(QueryError::new(
            QueryErrorKind::RowLimit,
            "CQL3004",
            format!("query exceeded {} returned rows", context.limits.max_rows),
        ));
    }
    output.shrink_to_fit();
    Ok(output)
}

fn project_aggregate(
    rows: Vec<BindingRow>,
    clause: &ProjectionClause,
    context: &mut ExecutionContext<'_>,
) -> Result<Vec<BindingRow>, QueryError> {
    let grouping = clause
        .items
        .iter()
        .enumerate()
        .filter(|(_, item)| !contains_aggregate(&item.expression))
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let mut groups = BTreeMap::<String, Vec<BindingRow>>::new();
    if rows.is_empty() && grouping.is_empty() {
        groups.insert(String::new(), Vec::new());
    }
    for row in rows {
        let mut key = String::new();
        for index in &grouping {
            let value = eval(&clause.items[*index].expression, &row, None, context)?;
            append_canonical(&value, &mut key);
        }
        groups.entry(key).or_default().push(row);
    }
    let mut output = Vec::with_capacity(groups.len());
    for group in groups.into_values() {
        context.checkpoint()?;
        let empty = BindingRow::new();
        let representative = group.first().unwrap_or(&empty);
        let mut row = BindingRow::new();
        for item in &clause.items {
            if item.is_wildcard() {
                row.extend(representative.clone());
            } else {
                row.insert(
                    item.output_name(),
                    eval(&item.expression, representative, Some(&group), context)?,
                );
            }
        }
        output.push(row);
    }
    Ok(output)
}

pub(super) fn eval(
    expression: &Expr,
    row: &BindingRow,
    group: Option<&[BindingRow]>,
    context: &mut ExecutionContext<'_>,
) -> Result<CompassValue, QueryError> {
    match &expression.kind {
        ExprKind::Wildcard => Err(QueryError::new(
            QueryErrorKind::Internal,
            "CQL4099",
            "projection wildcard reached scalar evaluation",
        )),
        ExprKind::Literal(value) => Ok(value.clone()),
        ExprKind::Variable(name) => Ok(row.get(name).cloned().unwrap_or(CompassValue::Null)),
        ExprKind::Parameter(name) => context.parameters.get(name).cloned().ok_or_else(|| {
            QueryError::new(
                QueryErrorKind::InvalidParameter,
                "CQL4001",
                format!("missing parameter '${name}'"),
            )
        }),
        ExprKind::Property(target, property) => {
            let target = eval(target, row, group, context)?;
            property_value(&target, property, context)
        }
        ExprKind::LabelTest(target, label) => {
            let target = eval(target, row, group, context)?;
            Ok(match target {
                CompassValue::Null => CompassValue::Null,
                CompassValue::Node(node) => CompassValue::Boolean(
                    compass_model::cypher_node_label(context.graph.node(node.index)) == *label,
                ),
                _ => return Err(type_error("label test requires a node")),
            })
        }
        ExprKind::Index(target, index) => {
            let target = eval(target, row, group, context)?;
            let index = integer(eval(index, row, group, context)?)?;
            index_value(target, index)
        }
        ExprKind::Slice(target, start, end) => {
            let target = eval(target, row, group, context)?;
            let start = start
                .as_ref()
                .map(|value| eval(value, row, group, context).and_then(integer))
                .transpose()?;
            let end = end
                .as_ref()
                .map(|value| eval(value, row, group, context).and_then(integer))
                .transpose()?;
            slice_value(target, start, end)
        }
        ExprKind::List(values) => values
            .iter()
            .map(|value| eval(value, row, group, context))
            .collect::<Result<Vec<_>, _>>()
            .map(|values| CompassValue::List(Arc::from(values))),
        ExprKind::Map(values) => values
            .iter()
            .map(|(key, value)| Ok((key.clone(), eval(value, row, group, context)?)))
            .collect::<Result<BTreeMap<_, _>, QueryError>>()
            .map(|values| CompassValue::Map(Arc::new(values))),
        ExprKind::Unary(operator, operand) => {
            let operand = eval(operand, row, group, context)?;
            eval_unary(*operator, operand)
        }
        ExprKind::Binary(left, operator, right) => {
            if *operator == BinaryOp::And {
                let left = eval(left, row, group, context)?;
                if truthy(&left) == Some(false) {
                    return Ok(CompassValue::Boolean(false));
                }
                let right = eval(right, row, group, context)?;
                return Ok(boolean_and(left, right));
            }
            if *operator == BinaryOp::Or {
                let left = eval(left, row, group, context)?;
                if truthy(&left) == Some(true) {
                    return Ok(CompassValue::Boolean(true));
                }
                let right = eval(right, row, group, context)?;
                return Ok(boolean_or(left, right));
            }
            let left = eval(left, row, group, context)?;
            let right = eval(right, row, group, context)?;
            eval_binary(*operator, left, right)
        }
        ExprKind::IsNull(value, negated) => {
            let is_null = eval(value, row, group, context)?.is_null();
            Ok(CompassValue::Boolean(if *negated {
                !is_null
            } else {
                is_null
            }))
        }
        ExprKind::Function(call) => eval_function(call, row, group, context),
        ExprKind::ListPredicate(predicate) => {
            let list = eval(&predicate.list, row, group, context)?;
            let CompassValue::List(values) = list else {
                return if list.is_null() {
                    Ok(CompassValue::Null)
                } else {
                    Err(type_error("list predicate requires a list"))
                };
            };
            let mut matched = 0_usize;
            let mut rejected = 0_usize;
            let mut unknown = false;
            for value in values.iter() {
                let mut nested = row.clone();
                nested.insert(predicate.variable.clone(), value.clone());
                match truthy(&eval(&predicate.predicate, &nested, group, context)?) {
                    Some(true) => matched = matched.saturating_add(1),
                    Some(false) => rejected = rejected.saturating_add(1),
                    None => unknown = true,
                }
            }
            let result = match predicate.kind {
                ListPredicateKind::Any if matched > 0 => Some(true),
                ListPredicateKind::Any if unknown => None,
                ListPredicateKind::Any => Some(false),
                ListPredicateKind::All if rejected > 0 => Some(false),
                ListPredicateKind::All if unknown => None,
                ListPredicateKind::All => Some(true),
                ListPredicateKind::None if matched > 0 => Some(false),
                ListPredicateKind::None if unknown => None,
                ListPredicateKind::None => Some(true),
                ListPredicateKind::Single if matched > 1 => Some(false),
                ListPredicateKind::Single if unknown => None,
                ListPredicateKind::Single => Some(matched == 1),
            };
            Ok(result.map_or(CompassValue::Null, CompassValue::Boolean))
        }
        ExprKind::Case(value) => eval_case(value, row, group, context),
        ExprKind::Exists(part) => Ok(CompassValue::Boolean(execute_exists_part(
            part, row, context,
        )?)),
    }
}

fn eval_unary(operator: UnaryOp, value: CompassValue) -> Result<CompassValue, QueryError> {
    if value.is_null() {
        return Ok(CompassValue::Null);
    }
    match (operator, value) {
        (UnaryOp::Not, CompassValue::Boolean(value)) => Ok(CompassValue::Boolean(!value)),
        (UnaryOp::Positive, value @ (CompassValue::Integer(_) | CompassValue::Float(_))) => {
            Ok(value)
        }
        (UnaryOp::Negative, CompassValue::Integer(value)) => value
            .checked_neg()
            .map(CompassValue::Integer)
            .ok_or_else(arithmetic_overflow),
        (UnaryOp::Negative, CompassValue::Float(value)) => Ok(CompassValue::Float(-value)),
        _ => Err(type_error("invalid unary operand")),
    }
}

fn eval_binary(
    operator: BinaryOp,
    left: CompassValue,
    right: CompassValue,
) -> Result<CompassValue, QueryError> {
    if left.is_null() || right.is_null() {
        return Ok(CompassValue::Null);
    }
    match operator {
        BinaryOp::Xor => match (truthy(&left), truthy(&right)) {
            (Some(left), Some(right)) => Ok(CompassValue::Boolean(left ^ right)),
            _ => Ok(CompassValue::Null),
        },
        BinaryOp::Equal => {
            Ok(nullable_equal(&left, &right).map_or(CompassValue::Null, CompassValue::Boolean))
        }
        BinaryOp::NotEqual => Ok(nullable_equal(&left, &right)
            .map_or(CompassValue::Null, |value| CompassValue::Boolean(!value))),
        BinaryOp::Less => Ok(CompassValue::Boolean(compare_values(&left, &right).is_lt())),
        BinaryOp::LessOrEqual => Ok(CompassValue::Boolean(
            !compare_values(&left, &right).is_gt(),
        )),
        BinaryOp::Greater => Ok(CompassValue::Boolean(compare_values(&left, &right).is_gt())),
        BinaryOp::GreaterOrEqual => Ok(CompassValue::Boolean(
            !compare_values(&left, &right).is_lt(),
        )),
        BinaryOp::In => match right {
            CompassValue::List(values) => {
                let mut unknown = false;
                for value in values.iter() {
                    match nullable_equal(&left, value) {
                        Some(true) => return Ok(CompassValue::Boolean(true)),
                        Some(false) => {}
                        None => unknown = true,
                    }
                }
                Ok(if unknown {
                    CompassValue::Null
                } else {
                    CompassValue::Boolean(false)
                })
            }
            _ => Err(type_error("IN requires a list on the right")),
        },
        BinaryOp::StartsWith | BinaryOp::EndsWith | BinaryOp::Contains | BinaryOp::RegexMatch => {
            let (CompassValue::String(left), CompassValue::String(right)) = (left, right) else {
                return Err(type_error("string predicate requires strings"));
            };
            let value = match operator {
                BinaryOp::StartsWith => left.starts_with(right.as_ref()),
                BinaryOp::EndsWith => left.ends_with(right.as_ref()),
                BinaryOp::Contains => left.contains(right.as_ref()),
                BinaryOp::RegexMatch => safe_regex(&right)?.is_match(&left),
                _ => false,
            };
            Ok(CompassValue::Boolean(value))
        }
        BinaryOp::Add => add_values(left, right),
        BinaryOp::Subtract => numeric_binary(left, right, NumericOp::Subtract),
        BinaryOp::Multiply => numeric_binary(left, right, NumericOp::Multiply),
        BinaryOp::Divide => numeric_binary(left, right, NumericOp::Divide),
        BinaryOp::Modulo => numeric_binary(left, right, NumericOp::Modulo),
        BinaryOp::Power => numeric_binary(left, right, NumericOp::Power),
        BinaryOp::And | BinaryOp::Or => Err(QueryError::new(
            QueryErrorKind::Internal,
            "CQL4099",
            "boolean short-circuit operator reached generic evaluator",
        )),
    }
}

fn eval_function(
    call: &FunctionCall,
    row: &BindingRow,
    group: Option<&[BindingRow]>,
    context: &mut ExecutionContext<'_>,
) -> Result<CompassValue, QueryError> {
    let name = call.name.to_ascii_lowercase();
    if matches!(
        name.as_str(),
        "count" | "sum" | "avg" | "min" | "max" | "collect"
    ) {
        return eval_aggregate(&name, call, group.unwrap_or(&[]), context);
    }
    let arguments = call
        .arguments
        .iter()
        .map(|value| eval(value, row, group, context))
        .collect::<Result<Vec<_>, _>>()?;
    match name.as_str() {
        "size" => match arguments.first() {
            Some(CompassValue::List(values)) => usize_to_integer(values.len()),
            Some(CompassValue::String(value)) => usize_to_integer(value.chars().count()),
            Some(CompassValue::Null) => Ok(CompassValue::Null),
            _ => Err(type_error("size requires a list or string")),
        },
        "length" => match arguments.first() {
            Some(CompassValue::Path(value)) => usize_to_integer(value.relationships.len()),
            Some(CompassValue::Null) => Ok(CompassValue::Null),
            _ => Err(type_error("length requires a path")),
        },
        "head" => list_endpoint(arguments.first(), true),
        "last" => list_endpoint(arguments.first(), false),
        "coalesce" => Ok(arguments
            .into_iter()
            .find(|value| !value.is_null())
            .unwrap_or(CompassValue::Null)),
        "tolower" => string_unary(arguments.first(), |value| value.to_lowercase()),
        "toupper" => string_unary(arguments.first(), |value| value.to_uppercase()),
        "trim" => string_unary(arguments.first(), |value| value.trim().to_owned()),
        "tointeger" => convert_integer(arguments.first()),
        "tofloat" => convert_float(arguments.first()),
        "tostring" => convert_string(arguments.first()),
        "toboolean" => convert_boolean(arguments.first()),
        "split" => string_split(&arguments),
        "replace" => string_replace(&arguments),
        "labels" => match arguments.first() {
            Some(CompassValue::Node(node)) => {
                Ok(CompassValue::List(Arc::from(vec![CompassValue::String(
                    Arc::from(compass_model::cypher_node_label(
                        context.graph.node(node.index),
                    )),
                )])))
            }
            Some(CompassValue::Null) => Ok(CompassValue::Null),
            _ => Err(type_error("labels requires a node")),
        },
        "type" => match arguments.first() {
            Some(CompassValue::Relationship(value)) => {
                Ok(CompassValue::String(Arc::clone(&value.relation)))
            }
            Some(CompassValue::Null) => Ok(CompassValue::Null),
            _ => Err(type_error("type requires a relationship")),
        },
        "id" => match arguments.first() {
            Some(CompassValue::Node(value)) => usize_to_integer(value.index),
            Some(CompassValue::Relationship(value)) => usize_to_integer(value.index),
            Some(CompassValue::Null) => Ok(CompassValue::Null),
            _ => Err(type_error("id requires a node or relationship")),
        },
        "nodes" => match arguments.first() {
            Some(CompassValue::Path(value)) => Ok(CompassValue::List(Arc::from(
                value
                    .nodes
                    .iter()
                    .cloned()
                    .map(CompassValue::Node)
                    .collect::<Vec<_>>(),
            ))),
            Some(CompassValue::Null) => Ok(CompassValue::Null),
            _ => Err(type_error("nodes requires a path")),
        },
        "relationships" => match arguments.first() {
            Some(CompassValue::Path(value)) => Ok(CompassValue::List(Arc::from(
                value
                    .relationships
                    .iter()
                    .cloned()
                    .map(CompassValue::Relationship)
                    .collect::<Vec<_>>(),
            ))),
            Some(CompassValue::Null) => Ok(CompassValue::Null),
            _ => Err(type_error("relationships requires a path")),
        },
        "properties" => properties_function(arguments.first(), context),
        "keys" => match properties_function(arguments.first(), context)? {
            CompassValue::Map(values) => Ok(CompassValue::List(Arc::from(
                values
                    .keys()
                    .map(|key| CompassValue::String(Arc::from(key.as_str())))
                    .collect::<Vec<_>>(),
            ))),
            CompassValue::Null => Ok(CompassValue::Null),
            _ => Err(type_error("keys requires a map or graph value")),
        },
        _ => Err(QueryError::new(
            QueryErrorKind::Type,
            "CQL4010",
            format!("unsupported function '{}()'", call.name),
        )),
    }
}

fn convert_integer(value: Option<&CompassValue>) -> Result<CompassValue, QueryError> {
    Ok(match value {
        Some(CompassValue::Null) | None => CompassValue::Null,
        Some(CompassValue::Integer(value)) => CompassValue::Integer(*value),
        Some(CompassValue::Float(value)) if value.is_finite() => {
            let truncated = value.trunc();
            if truncated < i64::MIN as f64 || truncated > i64::MAX as f64 {
                CompassValue::Null
            } else {
                CompassValue::Integer(truncated as i64)
            }
        }
        Some(CompassValue::String(value)) => value
            .parse::<i64>()
            .ok()
            .map_or(CompassValue::Null, CompassValue::Integer),
        _ => return Err(type_error("toInteger requires a number or string")),
    })
}

fn convert_float(value: Option<&CompassValue>) -> Result<CompassValue, QueryError> {
    Ok(match value {
        Some(CompassValue::Null) | None => CompassValue::Null,
        Some(CompassValue::Integer(value)) => CompassValue::Float(*value as f64),
        Some(CompassValue::Float(value)) => CompassValue::Float(*value),
        Some(CompassValue::String(value)) => value
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
            .map_or(CompassValue::Null, CompassValue::Float),
        _ => return Err(type_error("toFloat requires a number or string")),
    })
}

fn convert_string(value: Option<&CompassValue>) -> Result<CompassValue, QueryError> {
    Ok(match value {
        Some(CompassValue::Null) | None => CompassValue::Null,
        Some(CompassValue::String(value)) => CompassValue::String(Arc::clone(value)),
        Some(CompassValue::Integer(value)) => CompassValue::String(value.to_string().into()),
        Some(CompassValue::Float(value)) => CompassValue::String(value.to_string().into()),
        Some(CompassValue::Boolean(value)) => CompassValue::String(value.to_string().into()),
        _ => return Err(type_error("toString requires a scalar value")),
    })
}

fn convert_boolean(value: Option<&CompassValue>) -> Result<CompassValue, QueryError> {
    Ok(match value {
        Some(CompassValue::Null) | None => CompassValue::Null,
        Some(CompassValue::Boolean(value)) => CompassValue::Boolean(*value),
        Some(CompassValue::String(value)) if value.eq_ignore_ascii_case("true") => {
            CompassValue::Boolean(true)
        }
        Some(CompassValue::String(value)) if value.eq_ignore_ascii_case("false") => {
            CompassValue::Boolean(false)
        }
        Some(CompassValue::String(_)) => CompassValue::Null,
        _ => return Err(type_error("toBoolean requires a boolean or string")),
    })
}

fn eval_aggregate(
    name: &str,
    call: &FunctionCall,
    group: &[BindingRow],
    context: &mut ExecutionContext<'_>,
) -> Result<CompassValue, QueryError> {
    if name == "count" && call.star {
        return usize_to_integer(group.len());
    }
    let Some(argument) = call.arguments.first() else {
        return Err(type_error("aggregate argument is missing"));
    };
    let mut values = Vec::new();
    let mut seen = BTreeSet::new();
    for row in group {
        let value = eval(argument, row, None, context)?;
        if value.is_null() {
            continue;
        }
        if !call.distinct || seen.insert(canonical_value(&value)) {
            values.push(value);
        }
    }
    match name {
        "count" => usize_to_integer(values.len()),
        "collect" => Ok(CompassValue::List(Arc::from(values))),
        "sum" => aggregate_sum(&values),
        "avg" => aggregate_avg(&values),
        "min" => Ok(values
            .into_iter()
            .min_by(compare_values)
            .unwrap_or(CompassValue::Null)),
        "max" => Ok(values
            .into_iter()
            .max_by(compare_values)
            .unwrap_or(CompassValue::Null)),
        _ => Err(type_error("unknown aggregate")),
    }
}

fn eval_case(
    case: &CaseExpr,
    row: &BindingRow,
    group: Option<&[BindingRow]>,
    context: &mut ExecutionContext<'_>,
) -> Result<CompassValue, QueryError> {
    let operand = case
        .operand
        .as_ref()
        .map(|value| eval(value, row, group, context))
        .transpose()?;
    for (condition, result) in &case.alternatives {
        let condition = eval(condition, row, group, context)?;
        let matches = operand.as_ref().map_or_else(
            || truthy(&condition) == Some(true),
            |operand| equal_values(operand, &condition),
        );
        if matches {
            return eval(result, row, group, context);
        }
    }
    case.fallback
        .as_ref()
        .map(|value| eval(value, row, group, context))
        .transpose()
        .map(|value| value.unwrap_or(CompassValue::Null))
}

pub(super) fn truthy(value: &CompassValue) -> Option<bool> {
    match value {
        CompassValue::Boolean(value) => Some(*value),
        CompassValue::Null => None,
        _ => None,
    }
}

pub(super) fn equal_values(left: &CompassValue, right: &CompassValue) -> bool {
    match (left, right) {
        (CompassValue::Integer(left), CompassValue::Float(right)) => (*left as f64) == *right,
        (CompassValue::Float(left), CompassValue::Integer(right)) => *left == (*right as f64),
        _ => left == right,
    }
}

fn nullable_equal(left: &CompassValue, right: &CompassValue) -> Option<bool> {
    match (left, right) {
        (CompassValue::Null, _) | (_, CompassValue::Null) => None,
        (CompassValue::List(left), CompassValue::List(right)) => {
            if left.len() != right.len() {
                return Some(false);
            }
            let mut unknown = false;
            for (left, right) in left.iter().zip(right.iter()) {
                match nullable_equal(left, right) {
                    Some(true) => {}
                    Some(false) => return Some(false),
                    None => unknown = true,
                }
            }
            (!unknown).then_some(true)
        }
        (CompassValue::Map(left), CompassValue::Map(right)) => {
            if left.len() != right.len() || left.keys().ne(right.keys()) {
                return Some(false);
            }
            let mut unknown = false;
            for key in left.keys() {
                match left
                    .get(key)
                    .zip(right.get(key))
                    .and_then(|(left, right)| nullable_equal(left, right))
                {
                    Some(true) => {}
                    Some(false) => return Some(false),
                    None => unknown = true,
                }
            }
            (!unknown).then_some(true)
        }
        _ => Some(equal_values(left, right)),
    }
}

pub(super) fn compare_values(left: &CompassValue, right: &CompassValue) -> Ordering {
    let left_rank = value_rank(left);
    let right_rank = value_rank(right);
    if left_rank != right_rank {
        return left_rank.cmp(&right_rank);
    }
    match (left, right) {
        (CompassValue::Null, CompassValue::Null) => Ordering::Equal,
        (CompassValue::Boolean(left), CompassValue::Boolean(right)) => left.cmp(right),
        (CompassValue::Integer(left), CompassValue::Integer(right)) => left.cmp(right),
        (CompassValue::Float(left), CompassValue::Float(right)) => left.total_cmp(right),
        (CompassValue::Integer(left), CompassValue::Float(right)) => {
            (*left as f64).total_cmp(right)
        }
        (CompassValue::Float(left), CompassValue::Integer(right)) => {
            left.total_cmp(&(*right as f64))
        }
        (CompassValue::String(left), CompassValue::String(right)) => left.cmp(right),
        (CompassValue::List(left), CompassValue::List(right)) => compare_slices(left, right),
        (CompassValue::Map(left), CompassValue::Map(right)) => {
            canonical_value(&CompassValue::Map(Arc::clone(left)))
                .cmp(&canonical_value(&CompassValue::Map(Arc::clone(right))))
        }
        (CompassValue::Node(left), CompassValue::Node(right)) => left.id.cmp(&right.id),
        (CompassValue::Relationship(left), CompassValue::Relationship(right)) => left.cmp(right),
        (CompassValue::Path(left), CompassValue::Path(right)) => left.cmp(right),
        _ => Ordering::Equal,
    }
}

fn compare_slices(left: &[CompassValue], right: &[CompassValue]) -> Ordering {
    for (left, right) in left.iter().zip(right) {
        let order = compare_values(left, right);
        if order != Ordering::Equal {
            return order;
        }
    }
    left.len().cmp(&right.len())
}

pub(super) fn canonical_row(row: &BindingRow) -> String {
    let mut output = String::new();
    for (key, value) in row {
        output.push_str(&key.len().to_string());
        output.push(':');
        output.push_str(key);
        append_canonical(value, &mut output);
    }
    output
}

pub(super) fn canonical_value(value: &CompassValue) -> String {
    let mut output = String::new();
    append_canonical(value, &mut output);
    output
}

fn append_canonical(value: &CompassValue, output: &mut String) {
    match value {
        CompassValue::Null => output.push('0'),
        CompassValue::Boolean(value) => output.push_str(if *value { "1t" } else { "1f" }),
        CompassValue::Integer(value) => output.push_str(&format!("2{value};")),
        CompassValue::Float(value) => output.push_str(&format!("3{:016x};", value.to_bits())),
        CompassValue::String(value) => output.push_str(&format!("4{}:{value}", value.len())),
        CompassValue::List(values) => {
            output.push_str(&format!("5{}[", values.len()));
            for value in values.iter() {
                append_canonical(value, output);
            }
            output.push(']');
        }
        CompassValue::Map(values) => {
            output.push_str(&format!("6{}{{", values.len()));
            for (key, value) in values.iter() {
                output.push_str(&format!("{}:{key}", key.len()));
                append_canonical(value, output);
            }
            output.push('}');
        }
        CompassValue::Node(value) => output.push_str(&format!("7{}:{}", value.id.len(), value.id)),
        CompassValue::Relationship(value) => output.push_str(&format!(
            "8{}:{}:{}:{}",
            value.index, value.source, value.relation, value.target
        )),
        CompassValue::Path(value) => {
            output.push('9');
            for node in value.nodes.iter() {
                output.push_str(&format!("{}:{}", node.id.len(), node.id));
            }
            for relationship in value.relationships.iter() {
                output.push_str(&format!("{};", relationship.index));
            }
        }
    }
}

pub(super) fn property_value(
    target: &CompassValue,
    property: &str,
    context: &ExecutionContext<'_>,
) -> Result<CompassValue, QueryError> {
    match target {
        CompassValue::Null => Ok(CompassValue::Null),
        CompassValue::Node(reference) => {
            let node = context.graph.node(reference.index);
            if property == "id" {
                return Ok(CompassValue::String(Arc::clone(&reference.id)));
            }
            if property == "label" {
                return Ok(CompassValue::String(Arc::from(node.label())));
            }
            node.attributes
                .get(property)
                .map(json_value)
                .transpose()
                .map(|value| value.unwrap_or(CompassValue::Null))
        }
        CompassValue::Relationship(reference) => {
            let edge = context.graph.edge(reference.index);
            let synthetic = match property {
                "source" => Some(reference.source.as_ref()),
                "target" => Some(reference.target.as_ref()),
                "relation" | "type" => Some(reference.relation.as_ref()),
                "confidence" if !edge.attributes.contains_key("confidence") => Some("EXTRACTED"),
                _ => None,
            };
            if let Some(value) = synthetic {
                return Ok(CompassValue::String(Arc::from(value)));
            }
            edge.attributes
                .get(property)
                .map(json_value)
                .transpose()
                .map(|value| value.unwrap_or(CompassValue::Null))
        }
        CompassValue::Map(values) => {
            Ok(values.get(property).cloned().unwrap_or(CompassValue::Null))
        }
        _ => Err(type_error(
            "property access requires a node, relationship, or map",
        )),
    }
}

fn properties_function(
    value: Option<&CompassValue>,
    context: &ExecutionContext<'_>,
) -> Result<CompassValue, QueryError> {
    match value {
        Some(CompassValue::Null) | None => Ok(CompassValue::Null),
        Some(CompassValue::Map(values)) => Ok(CompassValue::Map(Arc::clone(values))),
        Some(CompassValue::Node(node)) => {
            let record = context.graph.node(node.index);
            let mut values = record
                .attributes
                .iter()
                .map(|(key, value)| Ok((key.clone(), json_value(value)?)))
                .collect::<Result<BTreeMap<_, _>, QueryError>>()?;
            values.insert("id".to_owned(), CompassValue::String(Arc::clone(&node.id)));
            values.insert(
                "label".to_owned(),
                CompassValue::String(Arc::from(record.label())),
            );
            Ok(CompassValue::Map(Arc::new(values)))
        }
        Some(CompassValue::Relationship(relationship)) => {
            let record = context.graph.edge(relationship.index);
            let mut values = record
                .attributes
                .iter()
                .map(|(key, value)| Ok((key.clone(), json_value(value)?)))
                .collect::<Result<BTreeMap<_, _>, QueryError>>()?;
            values
                .entry("confidence".to_owned())
                .or_insert_with(|| CompassValue::String(Arc::from("EXTRACTED")));
            Ok(CompassValue::Map(Arc::new(values)))
        }
        _ => Err(type_error("properties requires a map or graph value")),
    }
}

fn json_value(value: &Value) -> Result<CompassValue, QueryError> {
    match value {
        Value::Null => Ok(CompassValue::Null),
        Value::Bool(value) => Ok(CompassValue::Boolean(*value)),
        Value::Number(value) => {
            if let Some(value) = value.as_i64() {
                Ok(CompassValue::Integer(value))
            } else if let Some(value) = value.as_u64() {
                i64::try_from(value)
                    .map(CompassValue::Integer)
                    .map_err(|_| {
                        QueryError::new(
                            QueryErrorKind::Arithmetic,
                            "CQL4006",
                            "integer graph property is outside the CompassQL i64 range",
                        )
                    })
            } else if let Some(value) = value.as_f64().filter(|value| value.is_finite()) {
                Ok(CompassValue::Float(value))
            } else {
                Err(type_error("non-finite JSON number"))
            }
        }
        Value::String(value) => Ok(CompassValue::String(Arc::from(value.as_str()))),
        Value::Array(values) => values
            .iter()
            .map(json_value)
            .collect::<Result<Vec<_>, _>>()
            .map(|values| CompassValue::List(Arc::from(values))),
        Value::Object(values) => values
            .iter()
            .map(|(key, value)| Ok((key.clone(), json_value(value)?)))
            .collect::<Result<BTreeMap<_, _>, QueryError>>()
            .map(|values| CompassValue::Map(Arc::new(values))),
    }
}

fn index_value(value: CompassValue, index: i64) -> Result<CompassValue, QueryError> {
    match value {
        CompassValue::Null => Ok(CompassValue::Null),
        CompassValue::List(values) => Ok(normalized_index(values.len(), index)
            .and_then(|index| values.get(index).cloned())
            .unwrap_or(CompassValue::Null)),
        _ => Err(type_error("indexing requires a list")),
    }
}

fn slice_value(
    value: CompassValue,
    start: Option<i64>,
    end: Option<i64>,
) -> Result<CompassValue, QueryError> {
    match value {
        CompassValue::Null => Ok(CompassValue::Null),
        CompassValue::List(values) => {
            let start = normalized_bound(values.len(), start.unwrap_or(0));
            let end = normalized_bound(values.len(), end.unwrap_or(values.len() as i64));
            let end = end.max(start);
            Ok(CompassValue::List(Arc::from(values[start..end].to_vec())))
        }
        _ => Err(type_error("slicing requires a list")),
    }
}

fn normalized_index(length: usize, index: i64) -> Option<usize> {
    let length_i64 = i64::try_from(length).ok()?;
    let normalized = if index < 0 {
        length_i64.checked_add(index)?
    } else {
        index
    };
    usize::try_from(normalized)
        .ok()
        .filter(|value| *value < length)
}

fn normalized_bound(length: usize, value: i64) -> usize {
    let length_i64 = i64::try_from(length).unwrap_or(i64::MAX);
    let value = if value < 0 {
        length_i64.saturating_add(value)
    } else {
        value
    };
    usize::try_from(value.clamp(0, length_i64)).unwrap_or(length)
}

fn integer(value: CompassValue) -> Result<i64, QueryError> {
    match value {
        CompassValue::Integer(value) => Ok(value),
        _ => Err(type_error("expected an integer")),
    }
}

fn eval_non_negative_usize(
    value: &Expr,
    row: &BindingRow,
    context: &mut ExecutionContext<'_>,
    name: &str,
) -> Result<usize, QueryError> {
    let value = integer(eval(value, row, None, context)?)?;
    usize::try_from(value).map_err(|_| {
        QueryError::new(
            QueryErrorKind::Type,
            "CQL4002",
            format!("{name} must be a non-negative integer"),
        )
    })
}

fn boolean_and(left: CompassValue, right: CompassValue) -> CompassValue {
    match (truthy(&left), truthy(&right)) {
        (Some(false), _) | (_, Some(false)) => CompassValue::Boolean(false),
        (Some(true), Some(true)) => CompassValue::Boolean(true),
        _ => CompassValue::Null,
    }
}

fn boolean_or(left: CompassValue, right: CompassValue) -> CompassValue {
    match (truthy(&left), truthy(&right)) {
        (Some(true), _) | (_, Some(true)) => CompassValue::Boolean(true),
        (Some(false), Some(false)) => CompassValue::Boolean(false),
        _ => CompassValue::Null,
    }
}

fn add_values(left: CompassValue, right: CompassValue) -> Result<CompassValue, QueryError> {
    match (left, right) {
        (CompassValue::String(left), CompassValue::String(right)) => {
            Ok(CompassValue::String(Arc::from(format!("{left}{right}"))))
        }
        (CompassValue::List(left), CompassValue::List(right)) => {
            let mut values = left.to_vec();
            values.extend(right.iter().cloned());
            Ok(CompassValue::List(Arc::from(values)))
        }
        (left, right) => numeric_binary(left, right, NumericOp::Add),
    }
}

#[derive(Clone, Copy)]
enum NumericOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    Modulo,
    Power,
}

fn numeric_binary(
    left: CompassValue,
    right: CompassValue,
    operation: NumericOp,
) -> Result<CompassValue, QueryError> {
    match (left, right) {
        (CompassValue::Integer(left), CompassValue::Integer(right))
            if !matches!(operation, NumericOp::Power) =>
        {
            let value = match operation {
                NumericOp::Add => left.checked_add(right),
                NumericOp::Subtract => left.checked_sub(right),
                NumericOp::Multiply => left.checked_mul(right),
                NumericOp::Divide if right != 0 => left.checked_div(right),
                NumericOp::Modulo if right != 0 => left.checked_rem(right),
                _ => None,
            };
            value
                .map(CompassValue::Integer)
                .ok_or_else(arithmetic_overflow)
        }
        (left, right) => {
            let left = number_as_f64(left)?;
            let right = number_as_f64(right)?;
            if right == 0.0 && matches!(operation, NumericOp::Divide | NumericOp::Modulo) {
                return Err(QueryError::new(
                    QueryErrorKind::Arithmetic,
                    "CQL4005",
                    "division by zero",
                ));
            }
            let value = match operation {
                NumericOp::Add => left + right,
                NumericOp::Subtract => left - right,
                NumericOp::Multiply => left * right,
                NumericOp::Divide => left / right,
                NumericOp::Modulo => left % right,
                NumericOp::Power => left.powf(right),
            };
            if value.is_finite() {
                Ok(CompassValue::Float(value))
            } else {
                Err(arithmetic_overflow())
            }
        }
    }
}

fn number_as_f64(value: CompassValue) -> Result<f64, QueryError> {
    match value {
        CompassValue::Integer(value) => Ok(value as f64),
        CompassValue::Float(value) => Ok(value),
        _ => Err(type_error("numeric operator requires numbers")),
    }
}

fn aggregate_sum(values: &[CompassValue]) -> Result<CompassValue, QueryError> {
    if values.is_empty() {
        return Ok(CompassValue::Integer(0));
    }
    values
        .iter()
        .cloned()
        .try_fold(CompassValue::Integer(0), |sum, value| {
            numeric_binary(sum, value, NumericOp::Add)
        })
}

fn aggregate_avg(values: &[CompassValue]) -> Result<CompassValue, QueryError> {
    if values.is_empty() {
        return Ok(CompassValue::Null);
    }
    let mut sum = 0.0;
    for value in values {
        sum += number_as_f64(value.clone())?;
    }
    Ok(CompassValue::Float(sum / values.len() as f64))
}

fn list_endpoint(value: Option<&CompassValue>, first: bool) -> Result<CompassValue, QueryError> {
    match value {
        Some(CompassValue::List(values)) => Ok(if first {
            values.first().cloned()
        } else {
            values.last().cloned()
        }
        .unwrap_or(CompassValue::Null)),
        Some(CompassValue::Null) | None => Ok(CompassValue::Null),
        _ => Err(type_error("head/last requires a list")),
    }
}

fn string_unary(
    value: Option<&CompassValue>,
    function: impl FnOnce(&str) -> String,
) -> Result<CompassValue, QueryError> {
    match value {
        Some(CompassValue::String(value)) => Ok(CompassValue::String(Arc::from(function(value)))),
        Some(CompassValue::Null) | None => Ok(CompassValue::Null),
        _ => Err(type_error("string function requires a string")),
    }
}

fn string_split(values: &[CompassValue]) -> Result<CompassValue, QueryError> {
    let [CompassValue::String(value), CompassValue::String(separator)] = values else {
        return Err(type_error("split requires two strings"));
    };
    Ok(CompassValue::List(Arc::from(
        value
            .split(separator.as_ref())
            .map(|value| CompassValue::String(Arc::from(value)))
            .collect::<Vec<_>>(),
    )))
}

fn string_replace(values: &[CompassValue]) -> Result<CompassValue, QueryError> {
    let [
        CompassValue::String(value),
        CompassValue::String(search),
        CompassValue::String(replacement),
    ] = values
    else {
        return Err(type_error("replace requires three strings"));
    };
    Ok(CompassValue::String(Arc::from(
        value.replace(search.as_ref(), replacement.as_ref()),
    )))
}

fn safe_regex(pattern: &str) -> Result<Regex, QueryError> {
    if pattern.len() > 16 * 1024
        || ["(?=", "(?!", "(?<=", "(?<!", "\\1", "\\2", "\\k<"]
            .iter()
            .any(|needle| pattern.contains(needle))
    {
        return Err(QueryError::new(
            QueryErrorKind::Regex,
            "CQL4018",
            "regex uses an unsupported or unsafe construct",
        ));
    }
    Regex::new(pattern).map_err(|error| {
        QueryError::new(
            QueryErrorKind::Regex,
            "CQL4018",
            format!("invalid regex: {error}"),
        )
    })
}

fn contains_aggregate(expression: &Expr) -> bool {
    match &expression.kind {
        ExprKind::Function(call) => {
            matches!(
                call.name.to_ascii_lowercase().as_str(),
                "count" | "sum" | "avg" | "min" | "max" | "collect"
            ) || call.arguments.iter().any(contains_aggregate)
        }
        ExprKind::ListPredicate(value) => {
            contains_aggregate(&value.list) || contains_aggregate(&value.predicate)
        }
        ExprKind::Property(value, _)
        | ExprKind::LabelTest(value, _)
        | ExprKind::IsNull(value, _)
        | ExprKind::Unary(_, value) => contains_aggregate(value),
        ExprKind::Index(left, right) | ExprKind::Binary(left, _, right) => {
            contains_aggregate(left) || contains_aggregate(right)
        }
        ExprKind::Slice(value, start, end) => {
            contains_aggregate(value)
                || start.as_deref().is_some_and(contains_aggregate)
                || end.as_deref().is_some_and(contains_aggregate)
        }
        ExprKind::List(values) => values.iter().any(contains_aggregate),
        ExprKind::Map(values) => values.iter().any(|(_, value)| contains_aggregate(value)),
        ExprKind::Case(value) => {
            value.operand.as_deref().is_some_and(contains_aggregate)
                || value.alternatives.iter().any(|(condition, result)| {
                    contains_aggregate(condition) || contains_aggregate(result)
                })
                || value.fallback.as_deref().is_some_and(contains_aggregate)
        }
        ExprKind::Wildcard
        | ExprKind::Literal(_)
        | ExprKind::Variable(_)
        | ExprKind::Parameter(_)
        | ExprKind::Exists(_) => false,
    }
}

fn value_rank(value: &CompassValue) -> u8 {
    match value {
        CompassValue::Map(_) => 0,
        CompassValue::Node(_) => 1,
        CompassValue::Relationship(_) => 2,
        CompassValue::Path(_) => 3,
        CompassValue::List(_) => 4,
        CompassValue::String(_) => 5,
        CompassValue::Boolean(_) => 6,
        CompassValue::Integer(_) | CompassValue::Float(_) => 7,
        CompassValue::Null => 8,
    }
}

fn usize_to_integer(value: usize) -> Result<CompassValue, QueryError> {
    i64::try_from(value)
        .map(CompassValue::Integer)
        .map_err(|_| arithmetic_overflow())
}

fn arithmetic_overflow() -> QueryError {
    QueryError::new(
        QueryErrorKind::Arithmetic,
        "CQL4006",
        "numeric result is outside the CompassQL range",
    )
}

fn type_error(message: &str) -> QueryError {
    QueryError::new(QueryErrorKind::Type, "CQL4003", message)
}
