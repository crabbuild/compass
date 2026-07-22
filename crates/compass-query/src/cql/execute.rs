use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use compass_cypher::{
    Clause, Column, CompassValue, CompiledQuery, Direction, MatchClause, NodePattern, NodeRef,
    Parameters, PathRef, PathSelector, Pattern, ProjectionClause, QueryPart, QueryProfileMode,
    RelationshipPattern, RelationshipRef, Row,
};
use compass_model::{EdgeIndex, Graph, NodeIndex};
use serde::Serialize;

use super::error::{QueryError, QueryErrorKind};
use super::eval::{
    canonical_row, canonical_value, equal_values, eval, project_rows, property_value, truthy,
};
use super::profile::{OperatorProfile, QueryProfile};

pub(super) type BindingRow = BTreeMap<String, CompassValue>;

#[derive(Clone, Copy, Debug)]
pub struct QueryLimits {
    pub deadline: Instant,
    pub max_rows: usize,
    pub max_path_depth: usize,
    pub max_expanded_relationships: u64,
    pub max_memory_bytes: usize,
}

impl QueryLimits {
    #[must_use]
    pub fn interactive() -> Self {
        Self {
            deadline: Instant::now() + Duration::from_secs(5),
            max_rows: 10_000,
            max_path_depth: 32,
            max_expanded_relationships: 5_000_000,
            max_memory_bytes: 256 * 1024 * 1024,
        }
    }
}

pub struct QueryRequest<'a> {
    pub compiled: &'a CompiledQuery,
    pub graph: &'a Graph,
    pub parameters: &'a Parameters,
    pub limits: QueryLimits,
    pub cancellation: &'a AtomicBool,
}

#[derive(Clone, Debug, Serialize)]
pub struct ExplainPlan {
    pub operators: Vec<String>,
    pub optimizations: Vec<String>,
    pub columns: Vec<Column>,
    pub schema_fingerprint: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct QueryResult {
    pub columns: Vec<Column>,
    pub rows: Vec<Row>,
    pub profile: Option<QueryProfile>,
    pub explain: Option<ExplainPlan>,
}

pub fn execute(request: QueryRequest<'_>) -> Result<QueryResult, QueryError> {
    if request.limits.max_path_depth > 32 {
        return Err(QueryError::new(
            QueryErrorKind::Internal,
            "CQL4098",
            "runtime path depth cannot exceed the language ceiling of 32",
        ));
    }
    let explain = ExplainPlan {
        operators: request
            .compiled
            .plan
            .operators
            .iter()
            .map(|operator| operator.name().to_owned())
            .collect(),
        optimizations: request
            .compiled
            .plan
            .optimizations
            .iter()
            .map(|record| format!("{}: {}", record.rule, record.reason))
            .collect(),
        columns: request.compiled.columns.clone(),
        schema_fingerprint: request.graph.schema_fingerprint().to_hex(),
    };
    if request.compiled.profile == QueryProfileMode::Explain {
        return Ok(QueryResult {
            columns: request.compiled.columns.clone(),
            rows: Vec::new(),
            profile: None,
            explain: Some(explain),
        });
    }

    let started = Instant::now();
    let mut context = ExecutionContext {
        graph: request.graph,
        parameters: request.parameters,
        limits: request.limits,
        cancellation: request.cancellation,
        candidate_nodes: 0,
        expanded_relationships: 0,
        peak_memory_bytes: 0,
        cancellation_checks: 0,
        operators: Vec::new(),
    };
    context.checkpoint()?;
    let mut result_rows = Vec::<Row>::new();
    for (part_index, part) in request.compiled.plan.ast.parts.iter().enumerate() {
        let mut bindings = execute_part(part, vec![BindingRow::new()], &mut context)?;
        if part
            .clauses
            .last()
            .and_then(projection_clause)
            .is_some_and(|projection| projection.order_by.is_empty())
        {
            bindings.sort_by_key(canonical_row);
        }
        let rows = bindings
            .into_iter()
            .map(|binding| {
                request
                    .compiled
                    .columns
                    .iter()
                    .map(|column| {
                        binding
                            .get(&column.name)
                            .cloned()
                            .unwrap_or(CompassValue::Null)
                    })
                    .collect::<Row>()
            })
            .collect::<Vec<_>>();
        if part_index == 0 {
            result_rows = rows;
        } else {
            let union = request.compiled.plan.ast.unions[part_index - 1];
            if union == compass_cypher::UnionKind::All {
                result_rows.extend(rows);
            } else {
                let mut seen = BTreeSet::new();
                result_rows.extend(rows);
                result_rows.retain(|row| seen.insert(canonical_values(row)));
            }
        }
        context.reserve_results(&result_rows)?;
        if result_rows.len() > context.limits.max_rows {
            return Err(QueryError::new(
                QueryErrorKind::RowLimit,
                "CQL3004",
                format!("query exceeded {} returned rows", context.limits.max_rows),
            ));
        }
    }
    let profile = (request.compiled.profile == QueryProfileMode::Profile).then(|| QueryProfile {
        operators: context.operators,
        candidate_nodes: context.candidate_nodes,
        expanded_relationships: context.expanded_relationships,
        peak_memory_bytes: context.peak_memory_bytes,
        elapsed: started.elapsed(),
        cancellation_checks: context.cancellation_checks,
        plan_cache_hit: None,
    });
    Ok(QueryResult {
        columns: request.compiled.columns.clone(),
        rows: result_rows,
        profile,
        explain: (request.compiled.profile == QueryProfileMode::Profile).then_some(explain),
    })
}

pub(super) struct ExecutionContext<'a> {
    pub(super) graph: &'a Graph,
    pub(super) parameters: &'a Parameters,
    pub(super) limits: QueryLimits,
    cancellation: &'a AtomicBool,
    candidate_nodes: u64,
    expanded_relationships: u64,
    peak_memory_bytes: usize,
    cancellation_checks: u64,
    operators: Vec<OperatorProfile>,
}

impl ExecutionContext<'_> {
    pub(super) fn checkpoint(&mut self) -> Result<(), QueryError> {
        self.cancellation_checks = self.cancellation_checks.saturating_add(1);
        if self.cancellation.load(Ordering::Relaxed) {
            return Err(QueryError::new(
                QueryErrorKind::Cancelled,
                "CQL3008",
                "query was cancelled",
            ));
        }
        if Instant::now() >= self.limits.deadline {
            return Err(QueryError::new(
                QueryErrorKind::Timeout,
                "CQL3007",
                "query deadline exceeded",
            ));
        }
        Ok(())
    }

    pub(super) fn reserve_bindings(&mut self, rows: &[BindingRow]) -> Result<usize, QueryError> {
        let bytes = estimate_binding_rows(rows);
        self.reserve_bytes(bytes)?;
        Ok(bytes)
    }

    fn reserve_results(&mut self, rows: &[Row]) -> Result<(), QueryError> {
        self.reserve_bytes(estimate_result_rows(rows))
    }

    fn reserve_bytes(&mut self, bytes: usize) -> Result<(), QueryError> {
        self.peak_memory_bytes = self.peak_memory_bytes.max(bytes);
        if bytes > self.limits.max_memory_bytes {
            return Err(QueryError::new(
                QueryErrorKind::MemoryLimit,
                "CQL3006",
                format!(
                    "query working memory exceeds {} bytes",
                    self.limits.max_memory_bytes
                ),
            ));
        }
        Ok(())
    }

    fn count_candidate(&mut self) -> Result<(), QueryError> {
        self.candidate_nodes = self.candidate_nodes.saturating_add(1);
        if self.candidate_nodes.is_multiple_of(1_024) {
            self.checkpoint()?;
        }
        Ok(())
    }

    fn count_expansion(&mut self) -> Result<(), QueryError> {
        self.expanded_relationships = self.expanded_relationships.saturating_add(1);
        if self.expanded_relationships > self.limits.max_expanded_relationships {
            return Err(QueryError::new(
                QueryErrorKind::ExpansionLimit,
                "CQL3005",
                format!(
                    "query exceeded {} expanded relationships",
                    self.limits.max_expanded_relationships
                ),
            ));
        }
        if self.expanded_relationships.is_multiple_of(1_024) {
            self.checkpoint()?;
        }
        Ok(())
    }
}

pub(super) fn execute_part(
    part: &QueryPart,
    mut rows: Vec<BindingRow>,
    context: &mut ExecutionContext<'_>,
) -> Result<Vec<BindingRow>, QueryError> {
    for clause in &part.clauses {
        let started = Instant::now();
        let input_rows = rows.len();
        let candidates_before = context.candidate_nodes;
        let expansions_before = context.expanded_relationships;
        let checks_before = context.cancellation_checks;
        rows = match clause {
            Clause::Match(value) => apply_match(rows, value, context)?,
            Clause::Unwind(value) => {
                let mut output = Vec::new();
                for row in rows {
                    match eval(&value.expression, &row, None, context)? {
                        CompassValue::List(values) => {
                            for item in values.iter() {
                                let mut next = row.clone();
                                next.insert(value.variable.clone(), item.clone());
                                output.push(next);
                            }
                        }
                        CompassValue::Null => {}
                        _ => {
                            return Err(QueryError::new(
                                QueryErrorKind::Type,
                                "CQL4003",
                                "UNWIND requires a list",
                            ));
                        }
                    }
                }
                output
            }
            Clause::With(value) | Clause::Return(value) => project_rows(rows, value, context)?,
        };
        let working_memory = context.reserve_bindings(&rows)?;
        context.checkpoint()?;
        context.operators.push(OperatorProfile {
            name: clause_name(clause).to_owned(),
            input_rows: input_rows as u64,
            output_rows: rows.len() as u64,
            candidate_nodes: context.candidate_nodes.saturating_sub(candidates_before),
            expanded_relationships: context
                .expanded_relationships
                .saturating_sub(expansions_before),
            peak_memory_bytes: working_memory,
            elapsed: started.elapsed(),
            cancellation_checks: context.cancellation_checks.saturating_sub(checks_before),
        });
    }
    Ok(rows)
}

pub(super) fn execute_exists_part(
    part: &QueryPart,
    row: &BindingRow,
    context: &mut ExecutionContext<'_>,
) -> Result<bool, QueryError> {
    exists_clause(part, 0, row.clone(), context)
}

fn exists_clause(
    part: &QueryPart,
    clause_index: usize,
    row: BindingRow,
    context: &mut ExecutionContext<'_>,
) -> Result<bool, QueryError> {
    let Some(Clause::Match(clause)) = part.clauses.get(clause_index) else {
        return Ok(false);
    };
    exists_pattern(part, clause_index, clause, 0, row, context)
}

fn exists_pattern(
    part: &QueryPart,
    clause_index: usize,
    clause: &MatchClause,
    pattern_index: usize,
    row: BindingRow,
    context: &mut ExecutionContext<'_>,
) -> Result<bool, QueryError> {
    if let Some(pattern) = clause.patterns.get(pattern_index) {
        for candidate in match_pattern(&row, pattern, context)? {
            if exists_pattern(
                part,
                clause_index,
                clause,
                pattern_index + 1,
                candidate,
                context,
            )? {
                return Ok(true);
            }
        }
        return Ok(false);
    }
    if let Some(predicate) = &clause.predicate
        && truthy(&eval(predicate, &row, None, context)?) != Some(true)
    {
        return Ok(false);
    }
    if clause_index + 1 == part.clauses.len() {
        Ok(true)
    } else {
        exists_clause(part, clause_index + 1, row, context)
    }
}

fn apply_match(
    rows: Vec<BindingRow>,
    clause: &MatchClause,
    context: &mut ExecutionContext<'_>,
) -> Result<Vec<BindingRow>, QueryError> {
    let mut output = Vec::new();
    for row in rows {
        let mut matched = vec![row.clone()];
        for pattern in &clause.patterns {
            let mut next = Vec::new();
            for candidate in matched {
                next.extend(match_pattern(&candidate, pattern, context)?);
            }
            matched = next;
            if matched.is_empty() {
                break;
            }
        }
        if let Some(predicate) = &clause.predicate {
            let mut filtered = Vec::with_capacity(matched.len());
            for candidate in matched {
                if truthy(&eval(predicate, &candidate, None, context)?) == Some(true) {
                    filtered.push(candidate);
                }
            }
            matched = filtered;
        }
        if clause.optional && matched.is_empty() {
            let mut null_row = row;
            bind_optional_nulls(&mut null_row, clause);
            output.push(null_row);
        } else {
            output.extend(matched);
        }
    }
    Ok(output)
}

#[derive(Clone)]
struct PathState {
    row: BindingRow,
    current: NodeIndex,
    nodes: Vec<NodeRef>,
    relationships: Vec<RelationshipRef>,
    used_edges: BTreeSet<EdgeIndex>,
}

fn match_pattern(
    row: &BindingRow,
    pattern: &Pattern,
    context: &mut ExecutionContext<'_>,
) -> Result<Vec<BindingRow>, QueryError> {
    let candidates = start_candidates(row, &pattern.start, context)?;
    let mut states = Vec::new();
    for node in candidates {
        context.count_candidate()?;
        if let Some(next) = bind_and_match_node(row.clone(), node, &pattern.start, context)? {
            states.push(PathState {
                row: next,
                current: node,
                nodes: vec![node_ref(context.graph, node)],
                relationships: Vec::new(),
                used_edges: BTreeSet::new(),
            });
        }
    }
    if pattern.selector != PathSelector::All {
        let chain = &pattern.chains[0];
        let states = match_shortest_chain(states, chain, pattern.selector, context)?;
        return bind_path_results(states, pattern);
    }
    for chain in &pattern.chains {
        let mut next_states = Vec::new();
        for state in states {
            let expanded = expand_relationship(&state, &chain.relationship, context)?;
            for expanded in expanded {
                if let Some(row) = bind_and_match_node(
                    expanded.row.clone(),
                    expanded.current,
                    &chain.node,
                    context,
                )? {
                    let mut expanded = expanded;
                    expanded.row = row;
                    next_states.push(expanded);
                }
            }
        }
        states = next_states;
    }
    bind_path_results(states, pattern)
}

fn bind_path_results(
    states: Vec<PathState>,
    pattern: &Pattern,
) -> Result<Vec<BindingRow>, QueryError> {
    let mut output = Vec::with_capacity(states.len());
    for mut state in states {
        if let Some(variable) = &pattern.variable {
            let path = CompassValue::Path(PathRef {
                nodes: Arc::from(state.nodes),
                relationships: Arc::from(state.relationships),
            });
            if !bind_value(&mut state.row, variable, path) {
                continue;
            }
        }
        output.push(state.row);
    }
    Ok(output)
}

fn match_shortest_chain(
    mut frontier: Vec<PathState>,
    chain: &compass_cypher::PatternChain,
    selector: PathSelector,
    context: &mut ExecutionContext<'_>,
) -> Result<Vec<PathState>, QueryError> {
    let relationship = &chain.relationship;
    if relationship.max_hops > context.limits.max_path_depth {
        return Err(QueryError::new(
            QueryErrorKind::ExpansionLimit,
            "CQL3002",
            "path bound exceeds runtime maximum",
        ));
    }
    for depth in 0..=relationship.max_hops {
        if depth >= relationship.min_hops {
            let mut matches = Vec::new();
            for mut state in frontier.iter().cloned() {
                if !bind_relationship_path(&mut state, relationship) {
                    continue;
                }
                if let Some(row) =
                    bind_and_match_node(state.row.clone(), state.current, &chain.node, context)?
                {
                    state.row = row;
                    matches.push(state);
                    if selector == PathSelector::Shortest {
                        return Ok(matches);
                    }
                }
            }
            if !matches.is_empty() {
                return Ok(matches);
            }
        }
        if depth == relationship.max_hops {
            break;
        }
        let mut next_frontier = Vec::new();
        for state in frontier {
            for (edge, neighbor) in adjacent_edges(state.current, relationship, context.graph) {
                if state.used_edges.contains(&edge) {
                    continue;
                }
                context.count_expansion()?;
                if !relationship_properties_match(&state, edge, relationship, context)? {
                    continue;
                }
                let mut next = state.clone();
                next.current = neighbor;
                next.relationships
                    .push(relationship_ref(context.graph, edge));
                next.nodes.push(node_ref(context.graph, neighbor));
                next.used_edges.insert(edge);
                next_frontier.push(next);
            }
        }
        frontier = next_frontier;
        if frontier.is_empty() {
            break;
        }
    }
    Ok(Vec::new())
}

fn bind_relationship_path(state: &mut PathState, pattern: &RelationshipPattern) -> bool {
    let value = if pattern.min_hops == 1 && pattern.max_hops == 1 {
        state
            .relationships
            .first()
            .cloned()
            .map(CompassValue::Relationship)
            .unwrap_or(CompassValue::Null)
    } else {
        CompassValue::List(Arc::from(
            state
                .relationships
                .iter()
                .cloned()
                .map(CompassValue::Relationship)
                .collect::<Vec<_>>(),
        ))
    };
    pattern
        .variable
        .as_ref()
        .is_none_or(|variable| bind_value(&mut state.row, variable, value))
}

fn start_candidates(
    row: &BindingRow,
    pattern: &NodePattern,
    context: &mut ExecutionContext<'_>,
) -> Result<Vec<NodeIndex>, QueryError> {
    if let Some(variable) = &pattern.variable
        && let Some(value) = row.get(variable)
    {
        return match value {
            CompassValue::Node(node) => Ok(vec![node.index]),
            CompassValue::Null => Ok(Vec::new()),
            _ => Err(QueryError::new(
                QueryErrorKind::Type,
                "CQL4003",
                format!("'{variable}' is not a node"),
            )),
        };
    }
    for (key, expression) in &pattern.properties {
        let value = eval(expression, row, None, context)?;
        if key == "id" {
            if let CompassValue::String(id) = value {
                return Ok(context.graph.node_index(&id).into_iter().collect());
            }
        } else if key == "source_file" {
            if let CompassValue::String(source_file) = value {
                return Ok(context
                    .graph
                    .query_index()
                    .nodes_with_source_file(&source_file)
                    .to_vec());
            }
        } else if key == "label"
            && let CompassValue::String(label) = value
        {
            return Ok(context
                .graph
                .query_index()
                .nodes_with_display_label(&label)
                .to_vec());
        }
    }
    if let Some(label) = pattern.labels.first() {
        return Ok(context.graph.query_index().nodes_with_label(label).to_vec());
    }
    Ok((0..context.graph.node_count()).collect())
}

fn bind_and_match_node(
    mut row: BindingRow,
    node: NodeIndex,
    pattern: &NodePattern,
    context: &mut ExecutionContext<'_>,
) -> Result<Option<BindingRow>, QueryError> {
    let reference = node_ref(context.graph, node);
    if let Some(variable) = &pattern.variable
        && !bind_value(&mut row, variable, CompassValue::Node(reference.clone()))
    {
        return Ok(None);
    }
    let record = context.graph.node(node);
    if pattern
        .labels
        .iter()
        .any(|label| compass_model::cypher_node_label(record) != *label)
    {
        return Ok(None);
    }
    let target = CompassValue::Node(reference);
    for (property, expression) in &pattern.properties {
        let actual = property_value(&target, property, context)?;
        let expected = eval(expression, &row, None, context)?;
        if actual.is_null() || expected.is_null() || !equal_values(&actual, &expected) {
            return Ok(None);
        }
    }
    Ok(Some(row))
}

fn expand_relationship(
    state: &PathState,
    pattern: &RelationshipPattern,
    context: &mut ExecutionContext<'_>,
) -> Result<Vec<PathState>, QueryError> {
    if pattern.max_hops > context.limits.max_path_depth {
        return Err(QueryError::new(
            QueryErrorKind::ExpansionLimit,
            "CQL3002",
            "path bound exceeds runtime maximum",
        ));
    }
    let mut output = Vec::new();
    expand_depth_first(state.clone(), pattern, 0, &mut output, context)?;
    Ok(output)
}

fn expand_depth_first(
    state: PathState,
    pattern: &RelationshipPattern,
    depth: usize,
    output: &mut Vec<PathState>,
    context: &mut ExecutionContext<'_>,
) -> Result<(), QueryError> {
    if depth >= pattern.min_hops {
        let mut matched = state.clone();
        let slice = &matched.relationships[matched.relationships.len().saturating_sub(depth)..];
        let value = if pattern.min_hops == 1 && pattern.max_hops == 1 {
            slice
                .first()
                .cloned()
                .map(CompassValue::Relationship)
                .unwrap_or(CompassValue::Null)
        } else {
            CompassValue::List(Arc::from(
                slice
                    .iter()
                    .cloned()
                    .map(CompassValue::Relationship)
                    .collect::<Vec<_>>(),
            ))
        };
        if pattern
            .variable
            .as_ref()
            .is_none_or(|variable| bind_value(&mut matched.row, variable, value))
        {
            output.push(matched);
        }
    }
    if depth == pattern.max_hops {
        return Ok(());
    }
    for (edge, neighbor) in adjacent_edges(state.current, pattern, context.graph) {
        if state.used_edges.contains(&edge) {
            continue;
        }
        context.count_expansion()?;
        if !relationship_properties_match(&state, edge, pattern, context)? {
            continue;
        }
        let reference = relationship_ref(context.graph, edge);
        let mut next = state.clone();
        next.current = neighbor;
        next.relationships.push(reference);
        next.nodes.push(node_ref(context.graph, neighbor));
        next.used_edges.insert(edge);
        expand_depth_first(next, pattern, depth + 1, output, context)?;
    }
    Ok(())
}

fn relationship_properties_match(
    state: &PathState,
    edge: EdgeIndex,
    pattern: &RelationshipPattern,
    context: &mut ExecutionContext<'_>,
) -> Result<bool, QueryError> {
    let target = CompassValue::Relationship(relationship_ref(context.graph, edge));
    for (property, expression) in &pattern.properties {
        let actual = property_value(&target, property, context)?;
        let expected = eval(expression, &state.row, None, context)?;
        if actual.is_null() || expected.is_null() || !equal_values(&actual, &expected) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn adjacent_edges(
    node: NodeIndex,
    pattern: &RelationshipPattern,
    graph: &Graph,
) -> Vec<(EdgeIndex, NodeIndex)> {
    let mut output = Vec::new();
    let outgoing = |node: NodeIndex| -> Vec<EdgeIndex> {
        if graph.is_directed() && !pattern.types.is_empty() {
            pattern
                .types
                .iter()
                .flat_map(|relation| graph.query_index().outgoing_with_type(node, relation))
                .copied()
                .collect()
        } else {
            graph.outgoing_edges(node).collect()
        }
    };
    let incoming = |node: NodeIndex| -> Vec<EdgeIndex> {
        if graph.is_directed() && !pattern.types.is_empty() {
            pattern
                .types
                .iter()
                .flat_map(|relation| graph.query_index().incoming_with_type(node, relation))
                .copied()
                .collect()
        } else {
            graph.incoming_edges(node).collect()
        }
    };
    if matches!(
        pattern.direction,
        Direction::Outgoing | Direction::Undirected
    ) {
        for edge in outgoing(node) {
            if let Some((source, target)) = graph.edge_endpoints(edge) {
                let neighbor = if source == node { target } else { source };
                if relation_matches(edge, pattern, graph) {
                    output.push((edge, neighbor));
                }
            }
        }
    }
    if matches!(
        pattern.direction,
        Direction::Incoming | Direction::Undirected
    ) {
        for edge in incoming(node) {
            if let Some((source, target)) = graph.edge_endpoints(edge) {
                let neighbor = if target == node { source } else { target };
                if relation_matches(edge, pattern, graph) {
                    output.push((edge, neighbor));
                }
            }
        }
    }
    output.sort_unstable();
    output.dedup();
    output
}

fn relation_matches(edge: EdgeIndex, pattern: &RelationshipPattern, graph: &Graph) -> bool {
    pattern.types.is_empty()
        || pattern
            .types
            .iter()
            .any(|relation| compass_model::cypher_relationship_type(graph.edge(edge)) == *relation)
}

fn bind_optional_nulls(row: &mut BindingRow, clause: &MatchClause) {
    for pattern in &clause.patterns {
        if let Some(variable) = &pattern.variable {
            row.entry(variable.clone()).or_insert(CompassValue::Null);
        }
        if let Some(variable) = &pattern.start.variable {
            row.entry(variable.clone()).or_insert(CompassValue::Null);
        }
        for chain in &pattern.chains {
            if let Some(variable) = &chain.relationship.variable {
                row.entry(variable.clone()).or_insert(CompassValue::Null);
            }
            if let Some(variable) = &chain.node.variable {
                row.entry(variable.clone()).or_insert(CompassValue::Null);
            }
        }
    }
}

fn bind_value(row: &mut BindingRow, name: &str, value: CompassValue) -> bool {
    if let Some(existing) = row.get(name) {
        existing.is_null() || equal_values(existing, &value)
    } else {
        row.insert(name.to_owned(), value);
        true
    }
}

fn node_ref(graph: &Graph, node: NodeIndex) -> NodeRef {
    NodeRef {
        index: node,
        id: Arc::from(graph.node(node).id.as_str()),
    }
}

fn relationship_ref(graph: &Graph, edge: EdgeIndex) -> RelationshipRef {
    let record = graph.edge(edge);
    RelationshipRef {
        index: edge,
        source: Arc::from(record.source.as_str()),
        target: Arc::from(record.target.as_str()),
        relation: Arc::from(compass_model::cypher_relationship_type(record)),
    }
}

fn projection_clause(clause: &Clause) -> Option<&ProjectionClause> {
    match clause {
        Clause::With(value) | Clause::Return(value) => Some(value),
        Clause::Match(_) | Clause::Unwind(_) => None,
    }
}

fn clause_name(clause: &Clause) -> &'static str {
    match clause {
        Clause::Match(value) if value.optional => "OptionalMatch",
        Clause::Match(_) => "Match",
        Clause::Unwind(_) => "Unwind",
        Clause::With(_) => "With",
        Clause::Return(_) => "Return",
    }
}

fn canonical_values(values: &[CompassValue]) -> String {
    values
        .iter()
        .map(canonical_value)
        .collect::<Vec<_>>()
        .join("|")
}

fn estimate_binding_rows(rows: &[BindingRow]) -> usize {
    rows.iter().fold(std::mem::size_of_val(rows), |total, row| {
        total.saturating_add(row.iter().fold(
            std::mem::size_of::<BindingRow>(),
            |row_total, (name, value)| {
                row_total
                    .saturating_add(std::mem::size_of::<(String, CompassValue)>())
                    .saturating_add(name.capacity())
                    .saturating_add(estimate_value(value))
            },
        ))
    })
}

fn estimate_result_rows(rows: &[Row]) -> usize {
    rows.iter().fold(std::mem::size_of_val(rows), |total, row| {
        total
            .saturating_add(std::mem::size_of_val(row.as_slice()))
            .saturating_add(row.iter().fold(0usize, |sum, value| {
                sum.saturating_add(estimate_value(value))
            }))
    })
}

fn estimate_value(value: &CompassValue) -> usize {
    let base = std::mem::size_of::<CompassValue>();
    match value {
        CompassValue::String(value) => base.saturating_add(value.len()),
        CompassValue::List(values) => values.iter().fold(base, |total, value| {
            total.saturating_add(estimate_value(value))
        }),
        CompassValue::Map(values) => values.iter().fold(base, |total, (key, value)| {
            total
                .saturating_add(key.len())
                .saturating_add(estimate_value(value))
        }),
        CompassValue::Node(value) => base.saturating_add(value.id.len()),
        CompassValue::Relationship(value) => base
            .saturating_add(value.source.len())
            .saturating_add(value.target.len())
            .saturating_add(value.relation.len()),
        CompassValue::Path(value) => {
            let nodes = value.nodes.iter().fold(0usize, |sum, node| {
                sum.saturating_add(std::mem::size_of_val(node).saturating_add(node.id.len()))
            });
            let relationships = value
                .relationships
                .iter()
                .fold(0usize, |sum, relationship| {
                    sum.saturating_add(
                        std::mem::size_of_val(relationship)
                            .saturating_add(relationship.source.len())
                            .saturating_add(relationship.target.len())
                            .saturating_add(relationship.relation.len()),
                    )
                });
            base.saturating_add(nodes).saturating_add(relationships)
        }
        CompassValue::Null
        | CompassValue::Boolean(_)
        | CompassValue::Integer(_)
        | CompassValue::Float(_) => base,
    }
}
