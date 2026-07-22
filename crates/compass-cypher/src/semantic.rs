use std::collections::{BTreeMap, BTreeSet};

use crate::{
    BinaryOp, Clause, Column, CompassType, Diagnostic, Diagnostics, Expr, ExprKind,
    LogicalOperator, LogicalPlan, MatchClause, ParameterTypes, PathSelector, ProjectionClause,
    QueryAst, QueryPart, Span, UnaryOp,
};

#[derive(Clone, Copy)]
struct Binding {
    value_type: CompassType,
    nullable: bool,
}

type Scope = BTreeMap<String, Binding>;

pub fn analyze(
    ast: QueryAst,
    parameter_types: &ParameterTypes,
) -> Result<LogicalPlan, Diagnostics> {
    let mut part_columns = Vec::new();
    let mut operators = Vec::new();
    for part in &ast.parts {
        let columns = analyze_part(part, parameter_types, &Scope::new(), &mut operators)?;
        part_columns.push(columns);
    }
    let columns = part_columns.first().cloned().unwrap_or_default();
    for other in part_columns.iter().skip(1) {
        if other.len() != columns.len()
            || other.iter().zip(&columns).any(|(left, right)| {
                left.name != right.name || !compatible(left.value_type, right.value_type)
            })
        {
            return Err(Diagnostics::single(Diagnostic::new(
                "CQL2014",
                "UNION branches must return the same column names and compatible types",
                ast.span,
            )));
        }
    }
    for kind in &ast.unions {
        operators.push(LogicalOperator::Union {
            all: *kind == crate::UnionKind::All,
        });
    }
    Ok(LogicalPlan {
        ast,
        operators,
        columns,
        optimizations: Vec::new(),
    })
}

fn analyze_part(
    part: &QueryPart,
    parameter_types: &ParameterTypes,
    parent: &Scope,
    operators: &mut Vec<LogicalOperator>,
) -> Result<Vec<Column>, Diagnostics> {
    let mut scope = parent.clone();
    let mut returned = None;
    for clause in &part.clauses {
        match clause {
            Clause::Match(value) => {
                analyze_match(value, &mut scope, parameter_types, operators)?;
            }
            Clause::Unwind(value) => {
                validate_expr(&value.expression, &scope, parameter_types)?;
                scope.insert(
                    value.variable.clone(),
                    Binding {
                        value_type: CompassType::Any,
                        nullable: true,
                    },
                );
                operators.push(LogicalOperator::Unwind {
                    variable: value.variable.clone(),
                });
            }
            Clause::With(value) => {
                scope = analyze_projection(value, &scope, parameter_types, operators)?;
                if let Some(predicate) = &value.predicate {
                    require_boolean(predicate, &scope, parameter_types)?;
                    operators.push(LogicalOperator::Filter {
                        predicate: predicate.clone(),
                    });
                }
            }
            Clause::Return(value) => {
                let projected = analyze_projection(value, &scope, parameter_types, operators)?;
                returned = Some(projection_columns(value, &scope, &projected));
            }
        }
    }
    returned.ok_or_else(|| {
        Diagnostics::single(Diagnostic::new(
            "CQL2002",
            "every query part must end with RETURN",
            part.span,
        ))
    })
}

fn analyze_match(
    clause: &MatchClause,
    scope: &mut Scope,
    parameter_types: &ParameterTypes,
    operators: &mut Vec<LogicalOperator>,
) -> Result<(), Diagnostics> {
    for pattern in &clause.patterns {
        bind_node(
            &pattern.start.variable,
            clause.optional,
            scope,
            pattern.start.span,
        )?;
        for (_, value) in &pattern.start.properties {
            validate_expr(value, scope, parameter_types)?;
        }
        operators.push(LogicalOperator::NodeScan {
            variable: pattern.start.variable.clone().unwrap_or_default(),
            label: pattern.start.labels.first().cloned(),
        });
        for chain in &pattern.chains {
            bind_relationship(
                &chain.relationship,
                clause.optional,
                scope,
                chain.relationship.span,
            )?;
            bind_node(
                &chain.node.variable,
                clause.optional,
                scope,
                chain.node.span,
            )?;
            for (_, value) in &chain.relationship.properties {
                validate_expr(value, scope, parameter_types)?;
            }
            for (_, value) in &chain.node.properties {
                validate_expr(value, scope, parameter_types)?;
            }
            operators.push(LogicalOperator::Expand {
                variable: chain.relationship.variable.clone(),
                min_hops: chain.relationship.min_hops,
                max_hops: chain.relationship.max_hops,
            });
        }
        if let Some(variable) = &pattern.variable {
            bind(
                variable,
                CompassType::Path,
                clause.optional,
                scope,
                pattern.span,
            )?;
        }
        if pattern.selector != PathSelector::All && pattern.chains.len() != 1 {
            return Err(Diagnostics::single(Diagnostic::new(
                "CQL2020",
                "shortestPath and allShortestPaths require one relationship pattern",
                pattern.span,
            )));
        }
    }
    if clause.optional {
        operators.push(LogicalOperator::Optional);
    }
    if let Some(predicate) = &clause.predicate {
        require_boolean(predicate, scope, parameter_types)?;
        operators.push(LogicalOperator::Filter {
            predicate: predicate.clone(),
        });
    }
    Ok(())
}

fn analyze_projection(
    clause: &ProjectionClause,
    scope: &Scope,
    parameter_types: &ParameterTypes,
    operators: &mut Vec<LogicalOperator>,
) -> Result<Scope, Diagnostics> {
    let mut projected = Scope::new();
    let mut has_aggregate = false;
    let mut has_non_aggregate = false;
    for item in &clause.items {
        if item.is_wildcard() {
            has_non_aggregate = true;
            for (name, binding) in scope {
                if projected.insert(name.clone(), *binding).is_some() {
                    return Err(duplicate_projection(name, item.span));
                }
            }
            continue;
        }
        let binding = infer_expr(&item.expression, scope, parameter_types)?;
        let aggregate = contains_aggregate(&item.expression);
        has_aggregate |= aggregate;
        has_non_aggregate |= !aggregate;
        let name = item.output_name();
        if projected.insert(name.clone(), binding).is_some() {
            return Err(duplicate_projection(&name, item.span));
        }
    }
    if has_aggregate {
        operators.push(LogicalOperator::Aggregate);
        if has_non_aggregate {
            let _grouping_columns = clause
                .items
                .iter()
                .filter(|item| !contains_aggregate(&item.expression))
                .count();
        }
    }
    operators.push(LogicalOperator::Project);
    if clause.distinct {
        operators.push(LogicalOperator::Distinct);
    }
    for item in &clause.order_by {
        validate_expr(&item.expression, &projected, parameter_types)?;
    }
    if !clause.order_by.is_empty() {
        operators.push(LogicalOperator::Sort);
    }
    if let Some(skip) = &clause.skip {
        require_integer(skip, &projected, parameter_types)?;
        operators.push(LogicalOperator::Skip);
    }
    if let Some(limit) = &clause.limit {
        require_integer(limit, &projected, parameter_types)?;
        operators.push(LogicalOperator::Limit);
    }
    Ok(projected)
}

fn projection_columns(clause: &ProjectionClause, input: &Scope, projected: &Scope) -> Vec<Column> {
    let mut columns = Vec::new();
    for item in &clause.items {
        if item.is_wildcard() {
            columns.extend(input.iter().map(|(name, binding)| Column {
                name: name.clone(),
                value_type: binding.value_type,
                nullable: binding.nullable,
            }));
        } else {
            let name = item.output_name();
            if let Some(binding) = projected.get(&name) {
                columns.push(Column {
                    name,
                    value_type: binding.value_type,
                    nullable: binding.nullable,
                });
            }
        }
    }
    columns
}

fn duplicate_projection(name: &str, span: Span) -> Diagnostics {
    Diagnostics::single(Diagnostic::new(
        "CQL2015",
        format!("duplicate projection column '{name}'"),
        span,
    ))
}

fn bind_node(
    variable: &Option<String>,
    nullable: bool,
    scope: &mut Scope,
    span: Span,
) -> Result<(), Diagnostics> {
    if let Some(variable) = variable {
        bind(variable, CompassType::Node, nullable, scope, span)?;
    }
    Ok(())
}

fn bind_relationship(
    relationship: &crate::RelationshipPattern,
    nullable: bool,
    scope: &mut Scope,
    span: Span,
) -> Result<(), Diagnostics> {
    if let Some(variable) = &relationship.variable {
        let value_type = if relationship.min_hops == 1 && relationship.max_hops == 1 {
            CompassType::Relationship
        } else {
            CompassType::List
        };
        bind(variable, value_type, nullable, scope, span)?;
    }
    Ok(())
}

fn bind(
    variable: &str,
    value_type: CompassType,
    nullable: bool,
    scope: &mut Scope,
    span: Span,
) -> Result<(), Diagnostics> {
    if let Some(existing) = scope.get_mut(variable) {
        if matches!(existing.value_type, CompassType::Null | CompassType::Any) {
            existing.value_type = value_type;
            existing.nullable = true;
        } else if existing.value_type != value_type {
            return Err(Diagnostics::single(Diagnostic::new(
                "CQL2005",
                format!("'{variable}' is already bound as a different graph value"),
                span,
            )));
        } else {
            existing.nullable |= nullable;
        }
    } else {
        scope.insert(
            variable.to_owned(),
            Binding {
                value_type,
                nullable,
            },
        );
    }
    Ok(())
}

fn validate_expr(
    expression: &Expr,
    scope: &Scope,
    parameter_types: &ParameterTypes,
) -> Result<(), Diagnostics> {
    infer_expr(expression, scope, parameter_types).map(|_| ())
}

fn infer_expr(
    expression: &Expr,
    scope: &Scope,
    parameter_types: &ParameterTypes,
) -> Result<Binding, Diagnostics> {
    let binding = match &expression.kind {
        ExprKind::Wildcard => {
            return Err(Diagnostics::single(Diagnostic::new(
                "CQL2016",
                "projection wildcard is only valid as a top-level WITH or RETURN item",
                expression.span,
            )));
        }
        ExprKind::Literal(value) => Binding {
            value_type: value.compass_type(),
            nullable: value.is_null(),
        },
        ExprKind::Variable(name) => *scope.get(name).ok_or_else(|| {
            Diagnostics::single(Diagnostic::new(
                "CQL2004",
                format!("unknown variable '{name}'"),
                expression.span,
            ))
        })?,
        ExprKind::Parameter(name) => Binding {
            value_type: *parameter_types.get(name).ok_or_else(|| {
                Diagnostics::single(Diagnostic::new(
                    "CQL2011",
                    format!("unknown parameter '${name}'"),
                    expression.span,
                ))
            })?,
            nullable: true,
        },
        ExprKind::Property(target, _) | ExprKind::Index(target, _) => {
            validate_expr(target, scope, parameter_types)?;
            if let ExprKind::Index(_, index) = &expression.kind {
                validate_expr(index, scope, parameter_types)?;
            }
            Binding {
                value_type: CompassType::Any,
                nullable: true,
            }
        }
        ExprKind::Slice(target, start, end) => {
            validate_expr(target, scope, parameter_types)?;
            if let Some(value) = start {
                require_integer(value, scope, parameter_types)?;
            }
            if let Some(value) = end {
                require_integer(value, scope, parameter_types)?;
            }
            Binding {
                value_type: CompassType::List,
                nullable: true,
            }
        }
        ExprKind::LabelTest(target, _) | ExprKind::IsNull(target, _) => {
            validate_expr(target, scope, parameter_types)?;
            Binding {
                value_type: CompassType::Boolean,
                nullable: false,
            }
        }
        ExprKind::List(values) => {
            for value in values {
                validate_expr(value, scope, parameter_types)?;
            }
            Binding {
                value_type: CompassType::List,
                nullable: false,
            }
        }
        ExprKind::Map(values) => {
            for (_, value) in values {
                validate_expr(value, scope, parameter_types)?;
            }
            Binding {
                value_type: CompassType::Map,
                nullable: false,
            }
        }
        ExprKind::Unary(operator, operand) => {
            let operand = infer_expr(operand, scope, parameter_types)?;
            match operator {
                UnaryOp::Not => Binding {
                    value_type: CompassType::Boolean,
                    nullable: operand.nullable,
                },
                UnaryOp::Positive | UnaryOp::Negative => {
                    if !matches!(
                        operand.value_type,
                        CompassType::Any | CompassType::Integer | CompassType::Float
                    ) {
                        return Err(type_error("numeric unary operator", expression.span));
                    }
                    operand
                }
            }
        }
        ExprKind::Binary(left, operator, right) => {
            let left = infer_expr(left, scope, parameter_types)?;
            let right = infer_expr(right, scope, parameter_types)?;
            infer_binary(*operator, left, right, expression.span)?
        }
        ExprKind::Function(call) => infer_function(call, scope, parameter_types)?,
        ExprKind::ListPredicate(predicate) => {
            validate_expr(&predicate.list, scope, parameter_types)?;
            let mut nested = scope.clone();
            nested.insert(
                predicate.variable.clone(),
                Binding {
                    value_type: CompassType::Any,
                    nullable: true,
                },
            );
            require_boolean(&predicate.predicate, &nested, parameter_types)?;
            Binding {
                value_type: CompassType::Boolean,
                nullable: true,
            }
        }
        ExprKind::Case(value) => {
            if let Some(operand) = &value.operand {
                validate_expr(operand, scope, parameter_types)?;
            }
            let mut result_type = CompassType::Null;
            let mut nullable = value.fallback.is_none();
            for (condition, result) in &value.alternatives {
                validate_expr(condition, scope, parameter_types)?;
                let result = infer_expr(result, scope, parameter_types)?;
                result_type = unify(result_type, result.value_type);
                nullable |= result.nullable;
            }
            if let Some(fallback) = &value.fallback {
                let fallback = infer_expr(fallback, scope, parameter_types)?;
                result_type = unify(result_type, fallback.value_type);
                nullable |= fallback.nullable;
            }
            Binding {
                value_type: result_type,
                nullable,
            }
        }
        ExprKind::Exists(part) => {
            analyze_exists_part(part, parameter_types, scope)?;
            Binding {
                value_type: CompassType::Boolean,
                nullable: false,
            }
        }
    };
    Ok(binding)
}

fn infer_binary(
    operator: BinaryOp,
    left: Binding,
    right: Binding,
    span: Span,
) -> Result<Binding, Diagnostics> {
    let nullable = left.nullable || right.nullable;
    match operator {
        BinaryOp::Or
        | BinaryOp::Xor
        | BinaryOp::And
        | BinaryOp::Equal
        | BinaryOp::NotEqual
        | BinaryOp::Less
        | BinaryOp::LessOrEqual
        | BinaryOp::Greater
        | BinaryOp::GreaterOrEqual
        | BinaryOp::In
        | BinaryOp::StartsWith
        | BinaryOp::EndsWith
        | BinaryOp::Contains
        | BinaryOp::RegexMatch => Ok(Binding {
            value_type: CompassType::Boolean,
            nullable,
        }),
        BinaryOp::Add => {
            let value_type = if left.value_type == CompassType::String
                || right.value_type == CompassType::String
            {
                CompassType::String
            } else if left.value_type == CompassType::List || right.value_type == CompassType::List
            {
                CompassType::List
            } else {
                numeric_result(left.value_type, right.value_type, span)?
            };
            Ok(Binding {
                value_type,
                nullable,
            })
        }
        BinaryOp::Subtract
        | BinaryOp::Multiply
        | BinaryOp::Divide
        | BinaryOp::Modulo
        | BinaryOp::Power => Ok(Binding {
            value_type: numeric_result(left.value_type, right.value_type, span)?,
            nullable,
        }),
    }
}

fn infer_function(
    call: &crate::FunctionCall,
    scope: &Scope,
    parameter_types: &ParameterTypes,
) -> Result<Binding, Diagnostics> {
    let name = call.name.to_ascii_lowercase();
    if call.star && name != "count" {
        return Err(Diagnostics::single(Diagnostic::new(
            "CQL2016",
            "only count(*) accepts '*'",
            call.span,
        )));
    }
    for argument in &call.arguments {
        validate_expr(argument, scope, parameter_types)?;
    }
    let arity = call.arguments.len();
    let allowed = match name.as_str() {
        "count" => call.star || arity == 1,
        "sum" | "avg" | "min" | "max" | "collect" | "size" | "head" | "last" | "length"
        | "tolower" | "toupper" | "trim" | "tointeger" | "tofloat" | "tostring" | "toboolean" => {
            arity == 1
        }
        "split" | "replace" => matches!(arity, 2 | 3),
        "coalesce" => arity >= 1,
        "labels" | "type" | "properties" | "keys" | "nodes" | "relationships" => arity == 1,
        "id" => arity == 1,
        _ => false,
    };
    if !allowed {
        return Err(Diagnostics::single(Diagnostic::new(
            "CQL2016",
            format!("unknown function or invalid arity: {}", call.name),
            call.span,
        )));
    }
    let value_type = match name.as_str() {
        "count" | "size" | "length" | "tointeger" | "id" => CompassType::Integer,
        "avg" | "tofloat" => CompassType::Float,
        "sum" | "min" | "max" => CompassType::Any,
        "collect" | "split" | "labels" | "keys" | "nodes" | "relationships" => CompassType::List,
        "tolower" | "toupper" | "trim" | "replace" | "type" | "tostring" => CompassType::String,
        "toboolean" => CompassType::Boolean,
        "properties" => CompassType::Map,
        "head" | "last" | "coalesce" => CompassType::Any,
        _ => CompassType::Any,
    };
    Ok(Binding {
        value_type,
        nullable: !matches!(name.as_str(), "count" | "collect" | "labels" | "keys"),
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
        | ExprKind::IsNull(value, _) => contains_aggregate(value),
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
        ExprKind::Unary(_, value) => contains_aggregate(value),
        ExprKind::Case(value) => {
            value.operand.as_deref().is_some_and(contains_aggregate)
                || value
                    .alternatives
                    .iter()
                    .any(|(left, right)| contains_aggregate(left) || contains_aggregate(right))
                || value.fallback.as_deref().is_some_and(contains_aggregate)
        }
        ExprKind::Wildcard
        | ExprKind::Literal(_)
        | ExprKind::Variable(_)
        | ExprKind::Parameter(_)
        | ExprKind::Exists(_) => false,
    }
}

fn require_boolean(
    expression: &Expr,
    scope: &Scope,
    parameter_types: &ParameterTypes,
) -> Result<(), Diagnostics> {
    let binding = infer_expr(expression, scope, parameter_types)?;
    if matches!(
        binding.value_type,
        CompassType::Boolean | CompassType::Any | CompassType::Null
    ) {
        Ok(())
    } else {
        Err(type_error("predicate must be boolean", expression.span))
    }
}

fn require_integer(
    expression: &Expr,
    scope: &Scope,
    parameter_types: &ParameterTypes,
) -> Result<(), Diagnostics> {
    let binding = infer_expr(expression, scope, parameter_types)?;
    if matches!(binding.value_type, CompassType::Integer | CompassType::Any) {
        Ok(())
    } else {
        Err(type_error("value must be an integer", expression.span))
    }
}

fn numeric_result(
    left: CompassType,
    right: CompassType,
    span: Span,
) -> Result<CompassType, Diagnostics> {
    let numeric = |value| {
        matches!(
            value,
            CompassType::Any | CompassType::Integer | CompassType::Float
        )
    };
    if !numeric(left) || !numeric(right) {
        return Err(type_error(
            "numeric operator requires numeric operands",
            span,
        ));
    }
    Ok(
        if left == CompassType::Float || right == CompassType::Float {
            CompassType::Float
        } else if left == CompassType::Any || right == CompassType::Any {
            CompassType::Any
        } else {
            CompassType::Integer
        },
    )
}

fn type_error(message: &str, span: Span) -> Diagnostics {
    Diagnostics::single(Diagnostic::new("CQL2006", message, span))
}

fn compatible(left: CompassType, right: CompassType) -> bool {
    left == right
        || matches!(left, CompassType::Any | CompassType::Null)
        || matches!(right, CompassType::Any | CompassType::Null)
        || matches!(
            (left, right),
            (CompassType::Integer, CompassType::Float) | (CompassType::Float, CompassType::Integer)
        )
}

fn unify(left: CompassType, right: CompassType) -> CompassType {
    if left == CompassType::Null {
        right
    } else if compatible(left, right) {
        if left == CompassType::Float || right == CompassType::Float {
            CompassType::Float
        } else if left == CompassType::Any || right == CompassType::Any {
            CompassType::Any
        } else {
            left
        }
    } else {
        CompassType::Any
    }
}

#[allow(dead_code)]
fn referenced_variables(expression: &Expr, output: &mut BTreeSet<String>) {
    match &expression.kind {
        ExprKind::Variable(name) => {
            output.insert(name.clone());
        }
        ExprKind::Property(value, _)
        | ExprKind::LabelTest(value, _)
        | ExprKind::IsNull(value, _)
        | ExprKind::Unary(_, value) => referenced_variables(value, output),
        ExprKind::Index(left, right) | ExprKind::Binary(left, _, right) => {
            referenced_variables(left, output);
            referenced_variables(right, output);
        }
        ExprKind::Slice(value, start, end) => {
            referenced_variables(value, output);
            if let Some(value) = start {
                referenced_variables(value, output);
            }
            if let Some(value) = end {
                referenced_variables(value, output);
            }
        }
        ExprKind::List(values) => {
            for value in values {
                referenced_variables(value, output);
            }
        }
        ExprKind::Map(values) => {
            for (_, value) in values {
                referenced_variables(value, output);
            }
        }
        ExprKind::Function(call) => {
            for value in &call.arguments {
                referenced_variables(value, output);
            }
        }
        ExprKind::ListPredicate(value) => {
            referenced_variables(&value.list, output);
            referenced_variables(&value.predicate, output);
            output.remove(&value.variable);
        }
        ExprKind::Case(value) => {
            if let Some(operand) = &value.operand {
                referenced_variables(operand, output);
            }
            for (condition, result) in &value.alternatives {
                referenced_variables(condition, output);
                referenced_variables(result, output);
            }
            if let Some(fallback) = &value.fallback {
                referenced_variables(fallback, output);
            }
        }
        ExprKind::Wildcard
        | ExprKind::Literal(_)
        | ExprKind::Parameter(_)
        | ExprKind::Exists(_) => {}
    }
}

fn analyze_exists_part(
    part: &QueryPart,
    parameter_types: &ParameterTypes,
    parent: &Scope,
) -> Result<(), Diagnostics> {
    let mut scope = parent.clone();
    let mut operators = Vec::new();
    for clause in &part.clauses {
        let Clause::Match(value) = clause else {
            return Err(Diagnostics::single(Diagnostic::new(
                "CQL2019",
                "EXISTS subqueries support MATCH and WHERE only",
                part.span,
            )));
        };
        if value
            .predicate
            .as_ref()
            .is_some_and(contains_exists_expression)
        {
            return Err(Diagnostics::single(Diagnostic::new(
                "CQL1008",
                "nested EXISTS subqueries are unsupported",
                value.span,
            )));
        }
        analyze_match(value, &mut scope, parameter_types, &mut operators)?;
    }
    Ok(())
}

fn contains_exists_expression(expression: &Expr) -> bool {
    match &expression.kind {
        ExprKind::Exists(_) => true,
        ExprKind::Property(value, _)
        | ExprKind::LabelTest(value, _)
        | ExprKind::IsNull(value, _)
        | ExprKind::Unary(_, value) => contains_exists_expression(value),
        ExprKind::Index(left, right) | ExprKind::Binary(left, _, right) => {
            contains_exists_expression(left) || contains_exists_expression(right)
        }
        ExprKind::Slice(value, start, end) => {
            contains_exists_expression(value)
                || start.as_deref().is_some_and(contains_exists_expression)
                || end.as_deref().is_some_and(contains_exists_expression)
        }
        ExprKind::List(values) => values.iter().any(contains_exists_expression),
        ExprKind::Map(values) => values
            .iter()
            .any(|(_, value)| contains_exists_expression(value)),
        ExprKind::Function(value) => value.arguments.iter().any(contains_exists_expression),
        ExprKind::ListPredicate(value) => {
            contains_exists_expression(&value.list) || contains_exists_expression(&value.predicate)
        }
        ExprKind::Case(value) => {
            value
                .operand
                .as_deref()
                .is_some_and(contains_exists_expression)
                || value.alternatives.iter().any(|(condition, result)| {
                    contains_exists_expression(condition) || contains_exists_expression(result)
                })
                || value
                    .fallback
                    .as_deref()
                    .is_some_and(contains_exists_expression)
        }
        ExprKind::Wildcard
        | ExprKind::Literal(_)
        | ExprKind::Variable(_)
        | ExprKind::Parameter(_) => false,
    }
}
