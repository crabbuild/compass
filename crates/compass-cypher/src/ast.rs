use crate::{CompassValue, Span};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueryProfileMode {
    Execute,
    Explain,
    Profile,
}

#[derive(Clone, Debug, PartialEq)]
pub struct QueryAst {
    pub mode: QueryProfileMode,
    pub parts: Vec<QueryPart>,
    pub unions: Vec<UnionKind>,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnionKind {
    Distinct,
    All,
}

#[derive(Clone, Debug, PartialEq)]
pub struct QueryPart {
    pub clauses: Vec<Clause>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Clause {
    Match(MatchClause),
    Unwind(UnwindClause),
    With(ProjectionClause),
    Return(ProjectionClause),
}

#[derive(Clone, Debug, PartialEq)]
pub struct MatchClause {
    pub optional: bool,
    pub patterns: Vec<Pattern>,
    pub predicate: Option<Expr>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UnwindClause {
    pub expression: Expr,
    pub variable: String,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProjectionClause {
    pub distinct: bool,
    pub items: Vec<ProjectionItem>,
    pub predicate: Option<Expr>,
    pub order_by: Vec<SortItem>,
    pub skip: Option<Expr>,
    pub limit: Option<Expr>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProjectionItem {
    pub expression: Expr,
    pub alias: Option<String>,
    pub source_name: String,
    pub span: Span,
}

impl ProjectionItem {
    #[must_use]
    pub const fn is_wildcard(&self) -> bool {
        matches!(self.expression.kind, ExprKind::Wildcard)
    }

    #[must_use]
    pub fn output_name(&self) -> String {
        self.alias
            .clone()
            .unwrap_or_else(|| self.source_name.clone())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SortItem {
    pub expression: Expr,
    pub descending: bool,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pattern {
    pub variable: Option<String>,
    pub selector: PathSelector,
    pub start: NodePattern,
    pub chains: Vec<PatternChain>,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PathSelector {
    All,
    Shortest,
    AllShortest,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PatternChain {
    pub relationship: RelationshipPattern,
    pub node: NodePattern,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NodePattern {
    pub variable: Option<String>,
    pub labels: Vec<String>,
    pub properties: Vec<(String, Expr)>,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Direction {
    Outgoing,
    Incoming,
    Undirected,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RelationshipPattern {
    pub variable: Option<String>,
    pub types: Vec<String>,
    pub direction: Direction,
    pub min_hops: usize,
    pub max_hops: usize,
    pub properties: Vec<(String, Expr)>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

impl Expr {
    #[must_use]
    pub fn display_name(&self) -> String {
        match &self.kind {
            ExprKind::Wildcard => "*".to_owned(),
            ExprKind::Variable(name) => name.clone(),
            ExprKind::Property(target, property) => {
                format!("{}.{}", target.display_name(), property)
            }
            ExprKind::Function(call) => call.name.clone(),
            _ => format!("expr@{}", self.span.start),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExprKind {
    Wildcard,
    Literal(CompassValue),
    Variable(String),
    Parameter(String),
    Property(Box<Expr>, String),
    LabelTest(Box<Expr>, String),
    Index(Box<Expr>, Box<Expr>),
    Slice(Box<Expr>, Option<Box<Expr>>, Option<Box<Expr>>),
    List(Vec<Expr>),
    Map(Vec<(String, Expr)>),
    Unary(UnaryOp, Box<Expr>),
    Binary(Box<Expr>, BinaryOp, Box<Expr>),
    IsNull(Box<Expr>, bool),
    Function(FunctionCall),
    ListPredicate(ListPredicate),
    Case(CaseExpr),
    Exists(Box<QueryPart>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ListPredicateKind {
    Any,
    All,
    None,
    Single,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ListPredicate {
    pub kind: ListPredicateKind,
    pub variable: String,
    pub list: Box<Expr>,
    pub predicate: Box<Expr>,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnaryOp {
    Not,
    Positive,
    Negative,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BinaryOp {
    Or,
    Xor,
    And,
    Equal,
    NotEqual,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
    In,
    StartsWith,
    EndsWith,
    Contains,
    RegexMatch,
    Add,
    Subtract,
    Multiply,
    Divide,
    Modulo,
    Power,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FunctionCall {
    pub name: String,
    pub distinct: bool,
    pub star: bool,
    pub arguments: Vec<Expr>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CaseExpr {
    pub operand: Option<Box<Expr>>,
    pub alternatives: Vec<(Expr, Expr)>,
    pub fallback: Option<Box<Expr>>,
}
