//! Deterministic read-only CompassQL parser, semantic analyzer, and logical planner.

mod ast;
mod diagnostic;
mod lexer;
mod optimize;
mod parser;
mod plan;
mod semantic;
mod span;
mod support;
mod token;
mod value;

use sha2::{Digest, Sha256};

pub use ast::*;
pub use diagnostic::{Diagnostic, Diagnostics};
pub use lexer::lex;
pub use plan::{LogicalOperator, LogicalPlan, OptimizationRecord};
pub use span::Span;
pub use support::{SupportRecord, supported_features};
pub use token::{Token, TokenKind};
pub use value::*;

pub const LANGUAGE_VERSION: u16 = 1;
pub const PLANNER_VERSION: u16 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompileLimits {
    pub max_source_bytes: usize,
    pub max_tokens: usize,
    pub max_nesting: usize,
    pub max_path_depth: usize,
}

impl Default for CompileLimits {
    fn default() -> Self {
        Self {
            max_source_bytes: 1024 * 1024,
            max_tokens: 100_000,
            max_nesting: 256,
            max_path_depth: 32,
        }
    }
}

#[derive(Clone, Copy)]
pub struct CompileRequest<'a> {
    pub source_name: &'a str,
    pub source: &'a str,
    pub parameter_types: &'a ParameterTypes,
    pub schema: &'a compass_model::SchemaFingerprint,
    pub limits: CompileLimits,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PlanCacheKey([u8; 32]);

impl PlanCacheKey {
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CompiledQuery {
    pub plan: LogicalPlan,
    pub columns: Vec<Column>,
    pub profile: QueryProfileMode,
    pub cache_key: PlanCacheKey,
}

pub fn compile(request: CompileRequest<'_>) -> Result<CompiledQuery, Diagnostics> {
    if request.source.trim().is_empty() {
        return Err(Diagnostics::single(Diagnostic::new(
            "CQL1001",
            "query source is empty",
            Span::new(0, 0),
        )));
    }
    let ast = parser::parse(request)?;
    let profile = ast.mode;
    let plan = optimize::optimize(semantic::analyze(ast, request.parameter_types)?);
    let columns = plan.columns.clone();
    Ok(CompiledQuery {
        plan,
        columns,
        profile,
        cache_key: plan_cache_key(request),
    })
}

pub fn parse_only(request: CompileRequest<'_>) -> Result<QueryAst, Diagnostics> {
    if request.source.trim().is_empty() {
        return Err(Diagnostics::single(Diagnostic::new(
            "CQL1001",
            "query source is empty",
            Span::new(0, 0),
        )));
    }
    parser::parse(request)
}

#[must_use]
pub fn plan_cache_key(request: CompileRequest<'_>) -> PlanCacheKey {
    let mut digest = Sha256::new();
    digest.update(LANGUAGE_VERSION.to_le_bytes());
    digest.update(PLANNER_VERSION.to_le_bytes());
    digest.update((request.source.len() as u64).to_le_bytes());
    digest.update(request.source.as_bytes());
    digest.update(request.schema.as_bytes());
    digest.update((request.limits.max_path_depth as u64).to_le_bytes());
    for (name, value_type) in request.parameter_types {
        digest.update((name.len() as u64).to_le_bytes());
        digest.update(name.as_bytes());
        digest.update([*value_type as u8]);
    }
    PlanCacheKey(digest.finalize().into())
}
