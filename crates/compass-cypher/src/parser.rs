use std::sync::Arc;

use crate::CompassValue;
use crate::{
    BinaryOp, CaseExpr, Clause, CompileRequest, Diagnostic, Diagnostics, Direction, Expr, ExprKind,
    FunctionCall, ListPredicate, ListPredicateKind, MatchClause, NodePattern, PathSelector,
    Pattern, PatternChain, ProjectionClause, ProjectionItem, QueryAst, QueryPart, QueryProfileMode,
    RelationshipPattern, SortItem, Span, Token, TokenKind, UnaryOp, UnionKind, UnwindClause, lex,
};

pub fn parse(request: CompileRequest<'_>) -> Result<QueryAst, Diagnostics> {
    let tokens = lex(request.source, request.limits)?;
    Parser::new(request.source, tokens, request.limits).parse_query()
}

struct Parser<'a> {
    source: &'a str,
    tokens: Vec<Token>,
    position: usize,
    limits: crate::CompileLimits,
    nesting: usize,
}

impl<'a> Parser<'a> {
    fn new(source: &'a str, tokens: Vec<Token>, limits: crate::CompileLimits) -> Self {
        Self {
            source,
            tokens,
            position: 0,
            limits,
            nesting: 0,
        }
    }

    fn parse_query(mut self) -> Result<QueryAst, Diagnostics> {
        let start = self.current().span.start;
        let mode = if self.consume(TokenKind::Explain) {
            QueryProfileMode::Explain
        } else if self.consume(TokenKind::Profile) {
            QueryProfileMode::Profile
        } else {
            QueryProfileMode::Execute
        };
        self.reject_mutation()?;
        let mut parts = vec![self.parse_query_part(&[TokenKind::Union, TokenKind::Eof])?];
        let mut unions = Vec::new();
        while self.consume(TokenKind::Union) {
            let kind = if self.consume(TokenKind::All) {
                UnionKind::All
            } else {
                UnionKind::Distinct
            };
            unions.push(kind);
            parts.push(self.parse_query_part(&[TokenKind::Union, TokenKind::Eof])?);
        }
        self.consume(TokenKind::Semicolon);
        self.expect(TokenKind::Eof, "expected end of query")?;
        let end = self.previous().span.end;
        Ok(QueryAst {
            mode,
            parts,
            unions,
            span: Span::new(start, end),
        })
    }

    fn parse_query_part(&mut self, terminators: &[TokenKind]) -> Result<QueryPart, Diagnostics> {
        let start = self.current().span.start;
        let mut clauses = Vec::new();
        while !terminators.contains(&self.current().kind)
            && self.current().kind != TokenKind::RBrace
        {
            self.reject_mutation()?;
            let clause = match self.current().kind {
                TokenKind::Optional => {
                    self.advance();
                    self.expect(TokenKind::Match, "OPTIONAL must be followed by MATCH")?;
                    Clause::Match(self.parse_match(true)?)
                }
                TokenKind::Match => {
                    self.advance();
                    Clause::Match(self.parse_match(false)?)
                }
                TokenKind::Unwind => {
                    self.advance();
                    Clause::Unwind(self.parse_unwind()?)
                }
                TokenKind::With => {
                    self.advance();
                    Clause::With(self.parse_projection(true)?)
                }
                TokenKind::Return => {
                    self.advance();
                    Clause::Return(self.parse_projection(false)?)
                }
                _ => return Err(self.error_current("CQL1002", "expected a query clause")),
            };
            clauses.push(clause);
        }
        if clauses.is_empty() {
            return Err(self.error_current("CQL1002", "query part has no clauses"));
        }
        let end = clause_span(clauses.last()).map_or(start, |span| span.end);
        Ok(QueryPart {
            clauses,
            span: Span::new(start, end),
        })
    }

    fn parse_match(&mut self, optional: bool) -> Result<MatchClause, Diagnostics> {
        let start = self.previous().span.start;
        let mut patterns = vec![self.parse_pattern()?];
        while self.consume(TokenKind::Comma) {
            patterns.push(self.parse_pattern()?);
        }
        let predicate = if self.consume(TokenKind::Where) {
            Some(self.parse_expression(0)?)
        } else {
            None
        };
        let end = predicate.as_ref().map_or_else(
            || patterns.last().map_or(start, |value| value.span.end),
            |value| value.span.end,
        );
        Ok(MatchClause {
            optional,
            patterns,
            predicate,
            span: Span::new(start, end),
        })
    }

    fn parse_unwind(&mut self) -> Result<UnwindClause, Diagnostics> {
        let start = self.previous().span.start;
        let expression = self.parse_expression(0)?;
        self.expect(TokenKind::As, "UNWIND requires AS")?;
        let variable = self.expect_identifier("UNWIND alias")?;
        let end = self.previous().span.end;
        Ok(UnwindClause {
            expression,
            variable,
            span: Span::new(start, end),
        })
    }

    fn parse_projection(&mut self, is_with: bool) -> Result<ProjectionClause, Diagnostics> {
        let start = self.previous().span.start;
        let distinct = self.consume(TokenKind::Distinct);
        let mut items = vec![self.parse_projection_item()?];
        while self.consume(TokenKind::Comma) {
            items.push(self.parse_projection_item()?);
        }
        let predicate = if is_with && self.consume(TokenKind::Where) {
            Some(self.parse_expression(0)?)
        } else {
            None
        };
        let mut order_by = Vec::new();
        if self.consume(TokenKind::Order) {
            self.expect(TokenKind::By, "ORDER must be followed by BY")?;
            loop {
                let expression = self.parse_expression(0)?;
                let descending = if self.consume(TokenKind::Desc) {
                    true
                } else {
                    self.consume(TokenKind::Asc);
                    false
                };
                let span = expression.span;
                order_by.push(SortItem {
                    expression,
                    descending,
                    span,
                });
                if !self.consume(TokenKind::Comma) {
                    break;
                }
            }
        }
        let skip = if self.consume(TokenKind::Skip) {
            Some(self.parse_expression(0)?)
        } else {
            None
        };
        let limit = if self.consume(TokenKind::Limit) {
            Some(self.parse_expression(0)?)
        } else {
            None
        };
        let end = limit
            .as_ref()
            .or(skip.as_ref())
            .map_or_else(|| self.previous().span.end, |value| value.span.end);
        Ok(ProjectionClause {
            distinct,
            items,
            predicate,
            order_by,
            skip,
            limit,
            span: Span::new(start, end),
        })
    }

    fn parse_projection_item(&mut self) -> Result<ProjectionItem, Diagnostics> {
        let expression = if self.consume(TokenKind::Star) {
            Expr {
                kind: ExprKind::Wildcard,
                span: self.previous().span,
            }
        } else {
            self.parse_expression(0)?
        };
        let start = expression.span.start;
        let alias = if self.consume(TokenKind::As) {
            if matches!(expression.kind, ExprKind::Wildcard) {
                return Err(self.error_at(
                    "CQL1027",
                    "projection wildcard cannot have an alias",
                    expression.span,
                ));
            }
            Some(self.expect_identifier("projection alias")?)
        } else {
            None
        };
        let end = self.previous().span.end.max(expression.span.end);
        let source_name = self.source[expression.span.start..expression.span.end]
            .trim()
            .to_owned();
        Ok(ProjectionItem {
            expression,
            alias,
            source_name,
            span: Span::new(start, end),
        })
    }

    fn parse_pattern(&mut self) -> Result<Pattern, Diagnostics> {
        let start = self.current().span.start;
        let variable =
            if self.at(TokenKind::Identifier) && self.peek_kind(1) == Some(TokenKind::Equal) {
                let value = self.current().text.clone();
                self.advance();
                self.advance();
                Some(value)
            } else {
                None
            };
        let selector = if self.at_identifier_ci("shortestPath") {
            self.advance();
            self.expect(TokenKind::LParen, "shortestPath requires a pattern")?;
            PathSelector::Shortest
        } else if self.at_identifier_ci("allShortestPaths") {
            self.advance();
            self.expect(TokenKind::LParen, "allShortestPaths requires a pattern")?;
            PathSelector::AllShortest
        } else {
            PathSelector::All
        };
        let wrapped = selector != PathSelector::All;
        let start_node = self.parse_node_pattern()?;
        let mut chains = Vec::new();
        while matches!(self.current().kind, TokenKind::Minus | TokenKind::ArrowLeft) {
            let relationship = self.parse_relationship_pattern()?;
            let node = self.parse_node_pattern()?;
            chains.push(PatternChain { relationship, node });
        }
        if wrapped {
            self.expect(TokenKind::RParen, "shortest path pattern is missing ')'")?;
        }
        let end = self.previous().span.end;
        Ok(Pattern {
            variable,
            selector,
            start: start_node,
            chains,
            span: Span::new(start, end),
        })
    }

    fn parse_node_pattern(&mut self) -> Result<NodePattern, Diagnostics> {
        let open = self.expect(TokenKind::LParen, "node pattern must start with '('")?;
        self.enter_nesting(open.span)?;
        let variable = if self.at(TokenKind::Identifier) {
            let value = self.current().text.clone();
            self.advance();
            Some(value)
        } else {
            None
        };
        let mut labels = Vec::new();
        while self.consume(TokenKind::Colon) {
            labels.push(self.expect_identifier("node label")?);
        }
        let properties = if self.at(TokenKind::LBrace) {
            self.parse_properties()?
        } else {
            Vec::new()
        };
        let close = self.expect(TokenKind::RParen, "node pattern is missing ')'")?;
        self.leave_nesting();
        Ok(NodePattern {
            variable,
            labels,
            properties,
            span: open.span.merge(close.span),
        })
    }

    fn parse_relationship_pattern(&mut self) -> Result<RelationshipPattern, Diagnostics> {
        let start = self.current().span.start;
        let incoming = self.consume(TokenKind::ArrowLeft);
        if !incoming {
            self.expect(TokenKind::Minus, "relationship must start with '-'")?;
        }
        let mut variable = None;
        let mut types = Vec::new();
        let mut min_hops = 1;
        let mut max_hops = 1;
        let mut properties = Vec::new();
        if self.consume(TokenKind::LBracket) {
            self.enter_nesting(self.previous().span)?;
            if self.at(TokenKind::Identifier) {
                variable = Some(self.current().text.clone());
                self.advance();
            }
            if self.consume(TokenKind::Colon) {
                types.push(
                    self.expect_identifier("relationship type")?
                        .to_ascii_uppercase(),
                );
                while self.consume(TokenKind::Pipe) {
                    self.consume(TokenKind::Colon);
                    types.push(
                        self.expect_identifier("relationship type")?
                            .to_ascii_uppercase(),
                    );
                }
            }
            if self.consume(TokenKind::Star) {
                let lower = self.consume_usize()?;
                if self.consume(TokenKind::DotDot) {
                    min_hops = lower.unwrap_or(1);
                    max_hops = self.consume_usize()?.ok_or_else(|| {
                        self.error_current("CQL3002", "variable path requires an upper bound")
                    })?;
                } else if let Some(exact) = lower {
                    min_hops = exact;
                    max_hops = exact;
                } else {
                    return Err(self.error_current(
                        "CQL3002",
                        "variable path requires an explicit upper bound",
                    ));
                }
                if max_hops > self.limits.max_path_depth || min_hops > max_hops {
                    return Err(self.error_current(
                        "CQL3002",
                        format!(
                            "path bounds must satisfy 0 <= min <= max <= {}",
                            self.limits.max_path_depth
                        ),
                    ));
                }
            }
            if self.at(TokenKind::LBrace) {
                properties = self.parse_properties()?;
            }
            self.expect(TokenKind::RBracket, "relationship pattern is missing ']'")?;
            self.leave_nesting();
        }
        let direction = if incoming {
            self.expect(TokenKind::Minus, "incoming relationship must end with '-'")?;
            Direction::Incoming
        } else if self.consume(TokenKind::ArrowRight) {
            Direction::Outgoing
        } else {
            self.expect(TokenKind::Minus, "relationship must end with '-' or '->'")?;
            Direction::Undirected
        };
        Ok(RelationshipPattern {
            variable,
            types,
            direction,
            min_hops,
            max_hops,
            properties,
            span: Span::new(start, self.previous().span.end),
        })
    }

    fn parse_properties(&mut self) -> Result<Vec<(String, Expr)>, Diagnostics> {
        let open = self.expect(TokenKind::LBrace, "expected property map")?;
        self.enter_nesting(open.span)?;
        let mut properties = Vec::new();
        if !self.at(TokenKind::RBrace) {
            loop {
                let key = if matches!(
                    self.current().kind,
                    TokenKind::Identifier | TokenKind::String
                ) {
                    let value = self.current().text.clone();
                    self.advance();
                    value
                } else {
                    return Err(self.error_current("CQL1002", "expected property name"));
                };
                self.expect(TokenKind::Colon, "property name requires ':'")?;
                properties.push((key, self.parse_expression(0)?));
                if !self.consume(TokenKind::Comma) {
                    break;
                }
            }
        }
        self.expect(TokenKind::RBrace, "property map is missing '}'")?;
        self.leave_nesting();
        Ok(properties)
    }

    fn parse_expression(&mut self, minimum_binding: u8) -> Result<Expr, Diagnostics> {
        let mut left = self.parse_prefix()?;
        loop {
            if let Some(updated) = self.parse_postfix(left.clone())? {
                left = updated;
                continue;
            }
            let Some((operator, left_binding, right_binding, consumed)) = self.binary_operator()
            else {
                break;
            };
            if left_binding < minimum_binding {
                break;
            }
            for _ in 0..consumed {
                self.advance();
            }
            let right = self.parse_expression(right_binding)?;
            let span = left.span.merge(right.span);
            left = Expr {
                kind: ExprKind::Binary(Box::new(left), operator, Box::new(right)),
                span,
            };
        }
        Ok(left)
    }

    fn parse_prefix(&mut self) -> Result<Expr, Diagnostics> {
        let token = self.current().clone();
        self.advance();
        match token.kind {
            TokenKind::Null => Ok(literal(CompassValue::Null, token.span)),
            TokenKind::True => Ok(literal(CompassValue::Boolean(true), token.span)),
            TokenKind::False => Ok(literal(CompassValue::Boolean(false), token.span)),
            TokenKind::String => Ok(literal(
                CompassValue::String(Arc::from(token.text)),
                token.span,
            )),
            TokenKind::Integer => token
                .text
                .parse::<i64>()
                .map(|value| literal(CompassValue::Integer(value), token.span))
                .map_err(|_| {
                    self.error_at("CQL1009", "integer literal is out of range", token.span)
                }),
            TokenKind::Float => token
                .text
                .parse::<f64>()
                .ok()
                .filter(|value| value.is_finite())
                .map(|value| literal(CompassValue::Float(value), token.span))
                .ok_or_else(|| {
                    self.error_at("CQL1009", "invalid finite float literal", token.span)
                }),
            TokenKind::Parameter => Ok(Expr {
                kind: ExprKind::Parameter(token.text),
                span: token.span,
            }),
            TokenKind::Identifier | TokenKind::All => self.parse_identifier_expression(Token {
                kind: TokenKind::Identifier,
                ..token
            }),
            TokenKind::LParen => {
                self.enter_nesting(token.span)?;
                let expression = self.parse_expression(0)?;
                let close = self.expect(TokenKind::RParen, "expression is missing ')'")?;
                self.leave_nesting();
                Ok(Expr {
                    kind: expression.kind,
                    span: token.span.merge(close.span),
                })
            }
            TokenKind::LBracket => self.parse_list(token.span),
            TokenKind::LBrace => self.parse_map(token.span),
            TokenKind::Not => self.parse_unary(UnaryOp::Not, token.span, 4),
            TokenKind::Plus => self.parse_unary(UnaryOp::Positive, token.span, 8),
            TokenKind::Minus => self.parse_unary(UnaryOp::Negative, token.span, 8),
            TokenKind::Case => self.parse_case(token.span),
            TokenKind::Exists => self.parse_exists(token.span),
            _ => Err(self.error_at("CQL1002", "expected expression", token.span)),
        }
    }

    fn parse_identifier_expression(&mut self, token: Token) -> Result<Expr, Diagnostics> {
        if !self.consume(TokenKind::LParen) {
            return Ok(Expr {
                kind: ExprKind::Variable(token.text),
                span: token.span,
            });
        }
        self.enter_nesting(self.previous().span)?;
        let predicate_kind = match token.text.to_ascii_lowercase().as_str() {
            "any" => Some(ListPredicateKind::Any),
            "all" => Some(ListPredicateKind::All),
            "none" => Some(ListPredicateKind::None),
            "single" => Some(ListPredicateKind::Single),
            _ => None,
        };
        if let Some(kind) = predicate_kind {
            let variable = self.expect_identifier("list predicate variable")?;
            self.expect(TokenKind::In, "list predicate requires IN")?;
            let list = self.parse_expression(0)?;
            self.expect(TokenKind::Where, "list predicate requires WHERE")?;
            let predicate = self.parse_expression(0)?;
            let close = self.expect(TokenKind::RParen, "list predicate is missing ')'")?;
            self.leave_nesting();
            return Ok(Expr {
                kind: ExprKind::ListPredicate(ListPredicate {
                    kind,
                    variable,
                    list: Box::new(list),
                    predicate: Box::new(predicate),
                    span: token.span.merge(close.span),
                }),
                span: token.span.merge(close.span),
            });
        }
        let distinct = self.consume(TokenKind::Distinct);
        let star = self.consume(TokenKind::Star);
        let mut arguments = Vec::new();
        if !star && !self.at(TokenKind::RParen) {
            loop {
                arguments.push(self.parse_expression(0)?);
                if !self.consume(TokenKind::Comma) {
                    break;
                }
            }
        }
        let close = self.expect(TokenKind::RParen, "function call is missing ')'")?;
        self.leave_nesting();
        Ok(Expr {
            kind: ExprKind::Function(FunctionCall {
                name: token.text,
                distinct,
                star,
                arguments,
                span: token.span.merge(close.span),
            }),
            span: token.span.merge(close.span),
        })
    }

    fn parse_list(&mut self, open: Span) -> Result<Expr, Diagnostics> {
        self.enter_nesting(open)?;
        let mut items = Vec::new();
        if !self.at(TokenKind::RBracket) {
            loop {
                items.push(self.parse_expression(0)?);
                if !self.consume(TokenKind::Comma) {
                    break;
                }
            }
        }
        let close = self.expect(TokenKind::RBracket, "list is missing ']'")?;
        self.leave_nesting();
        Ok(Expr {
            kind: ExprKind::List(items),
            span: open.merge(close.span),
        })
    }

    fn parse_map(&mut self, open: Span) -> Result<Expr, Diagnostics> {
        self.position = self.position.saturating_sub(1);
        let properties = self.parse_properties()?;
        let end = self.previous().span.end;
        Ok(Expr {
            kind: ExprKind::Map(properties),
            span: Span::new(open.start, end),
        })
    }

    fn parse_unary(
        &mut self,
        operator: UnaryOp,
        start: Span,
        binding: u8,
    ) -> Result<Expr, Diagnostics> {
        let operand = self.parse_expression(binding)?;
        let span = start.merge(operand.span);
        Ok(Expr {
            kind: ExprKind::Unary(operator, Box::new(operand)),
            span,
        })
    }

    fn parse_case(&mut self, start: Span) -> Result<Expr, Diagnostics> {
        let operand = if self.at(TokenKind::When) {
            None
        } else {
            Some(Box::new(self.parse_expression(0)?))
        };
        let mut alternatives = Vec::new();
        while self.consume(TokenKind::When) {
            let condition = self.parse_expression(0)?;
            self.expect(TokenKind::Then, "CASE WHEN requires THEN")?;
            let result = self.parse_expression(0)?;
            alternatives.push((condition, result));
        }
        if alternatives.is_empty() {
            return Err(self.error_current("CQL1002", "CASE requires at least one WHEN"));
        }
        let fallback = if self.consume(TokenKind::Else) {
            Some(Box::new(self.parse_expression(0)?))
        } else {
            None
        };
        let end = self.expect(TokenKind::End, "CASE is missing END")?.span;
        Ok(Expr {
            kind: ExprKind::Case(CaseExpr {
                operand,
                alternatives,
                fallback,
            }),
            span: start.merge(end),
        })
    }

    fn parse_exists(&mut self, start: Span) -> Result<Expr, Diagnostics> {
        self.expect(TokenKind::LBrace, "EXISTS requires a pattern subquery")?;
        self.enter_nesting(self.previous().span)?;
        let part = if self.at(TokenKind::LParen) {
            let clause = Clause::Match(self.parse_match(false)?);
            let span = clause_span(Some(&clause)).unwrap_or(start);
            QueryPart {
                clauses: vec![clause],
                span,
            }
        } else {
            self.parse_query_part(&[TokenKind::RBrace])?
        };
        let end = self
            .expect(TokenKind::RBrace, "EXISTS is missing '}'")?
            .span;
        self.leave_nesting();
        Ok(Expr {
            kind: ExprKind::Exists(Box::new(part)),
            span: start.merge(end),
        })
    }

    fn parse_postfix(&mut self, left: Expr) -> Result<Option<Expr>, Diagnostics> {
        if self.consume(TokenKind::Dot) {
            let property = self.expect_identifier("property name")?;
            let span = left.span.merge(self.previous().span);
            return Ok(Some(Expr {
                kind: ExprKind::Property(Box::new(left), property),
                span,
            }));
        }
        if self.consume(TokenKind::Colon) {
            let label = self.expect_identifier("label name")?;
            let span = left.span.merge(self.previous().span);
            return Ok(Some(Expr {
                kind: ExprKind::LabelTest(Box::new(left), label),
                span,
            }));
        }
        if self.consume(TokenKind::LBracket) {
            let start_expr = if self.at(TokenKind::DotDot) || self.at(TokenKind::RBracket) {
                None
            } else {
                Some(self.parse_expression(0)?)
            };
            if self.consume(TokenKind::DotDot) {
                let end_expr = if self.at(TokenKind::RBracket) {
                    None
                } else {
                    Some(self.parse_expression(0)?)
                };
                let close = self.expect(TokenKind::RBracket, "slice is missing ']'")?;
                let span = left.span.merge(close.span);
                return Ok(Some(Expr {
                    kind: ExprKind::Slice(
                        Box::new(left),
                        start_expr.map(Box::new),
                        end_expr.map(Box::new),
                    ),
                    span,
                }));
            }
            let Some(index) = start_expr else {
                return Err(self.error_current("CQL1002", "list index is missing"));
            };
            let close = self.expect(TokenKind::RBracket, "index is missing ']'")?;
            let span = left.span.merge(close.span);
            return Ok(Some(Expr {
                kind: ExprKind::Index(Box::new(left), Box::new(index)),
                span,
            }));
        }
        if self.consume(TokenKind::Is) {
            let negated = self.consume(TokenKind::Not);
            let null = self.expect(TokenKind::Null, "IS only supports NULL or NOT NULL")?;
            let span = left.span.merge(null.span);
            return Ok(Some(Expr {
                kind: ExprKind::IsNull(Box::new(left), negated),
                span,
            }));
        }
        Ok(None)
    }

    fn binary_operator(&self) -> Option<(BinaryOp, u8, u8, usize)> {
        let left = |operator, precedence| Some((operator, precedence, precedence + 1, 1));
        match self.current().kind {
            TokenKind::Or => left(BinaryOp::Or, 1),
            TokenKind::Xor => left(BinaryOp::Xor, 2),
            TokenKind::And => left(BinaryOp::And, 3),
            TokenKind::Equal => left(BinaryOp::Equal, 4),
            TokenKind::NotEqual => left(BinaryOp::NotEqual, 4),
            TokenKind::Less => left(BinaryOp::Less, 4),
            TokenKind::LessEqual => left(BinaryOp::LessOrEqual, 4),
            TokenKind::Greater => left(BinaryOp::Greater, 4),
            TokenKind::GreaterEqual => left(BinaryOp::GreaterOrEqual, 4),
            TokenKind::In => left(BinaryOp::In, 4),
            TokenKind::Contains => left(BinaryOp::Contains, 4),
            TokenKind::RegexMatch => left(BinaryOp::RegexMatch, 4),
            TokenKind::Starts if self.peek_kind(1) == Some(TokenKind::With) => {
                Some((BinaryOp::StartsWith, 4, 5, 2))
            }
            TokenKind::Ends if self.peek_kind(1) == Some(TokenKind::With) => {
                Some((BinaryOp::EndsWith, 4, 5, 2))
            }
            TokenKind::Plus => left(BinaryOp::Add, 5),
            TokenKind::Minus => left(BinaryOp::Subtract, 5),
            TokenKind::Star => left(BinaryOp::Multiply, 6),
            TokenKind::Slash => left(BinaryOp::Divide, 6),
            TokenKind::Percent => left(BinaryOp::Modulo, 6),
            TokenKind::Caret => Some((BinaryOp::Power, 7, 7, 1)),
            _ => None,
        }
    }

    fn reject_mutation(&self) -> Result<(), Diagnostics> {
        if matches!(
            self.current().kind,
            TokenKind::Create
                | TokenKind::Merge
                | TokenKind::Delete
                | TokenKind::Detach
                | TokenKind::Set
                | TokenKind::Remove
                | TokenKind::Call
                | TokenKind::Load
                | TokenKind::Foreach
                | TokenKind::Use
                | TokenKind::Yield
        ) {
            return Err(self.error_current(
                "CQL1007",
                format!(
                    "{} is outside the read-only CompassQL surface",
                    self.current().text
                ),
            ));
        }
        Ok(())
    }

    fn consume_usize(&mut self) -> Result<Option<usize>, Diagnostics> {
        if !self.at(TokenKind::Integer) {
            return Ok(None);
        }
        let token = self.current().clone();
        self.advance();
        token
            .text
            .parse::<usize>()
            .map(Some)
            .map_err(|_| self.error_at("CQL1009", "path bound is out of range", token.span))
    }

    fn enter_nesting(&mut self, span: Span) -> Result<(), Diagnostics> {
        self.nesting = self.nesting.saturating_add(1);
        if self.nesting > self.limits.max_nesting {
            return Err(self.error_at("CQL3003", "query nesting limit exceeded", span));
        }
        Ok(())
    }

    fn leave_nesting(&mut self) {
        self.nesting = self.nesting.saturating_sub(1);
    }

    fn at(&self, kind: TokenKind) -> bool {
        self.current().kind == kind
    }

    fn at_identifier_ci(&self, expected: &str) -> bool {
        self.at(TokenKind::Identifier) && self.current().text.eq_ignore_ascii_case(expected)
    }

    fn consume(&mut self, kind: TokenKind) -> bool {
        if self.at(kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, kind: TokenKind, message: &str) -> Result<Token, Diagnostics> {
        if !self.at(kind) {
            return Err(self.error_current("CQL1002", message));
        }
        let token = self.current().clone();
        self.advance();
        Ok(token)
    }

    fn expect_identifier(&mut self, description: &str) -> Result<String, Diagnostics> {
        if !self.at(TokenKind::Identifier) {
            return Err(self.error_current("CQL1002", format!("expected {description}")));
        }
        let value = self.current().text.clone();
        self.advance();
        Ok(value)
    }

    fn advance(&mut self) {
        if self.position + 1 < self.tokens.len() {
            self.position += 1;
        }
    }

    fn current(&self) -> &Token {
        &self.tokens[self.position]
    }

    fn previous(&self) -> &Token {
        let index = self.position.saturating_sub(1);
        &self.tokens[index]
    }

    fn peek_kind(&self, offset: usize) -> Option<TokenKind> {
        self.tokens
            .get(self.position + offset)
            .map(|token| token.kind)
    }

    fn error_current(&self, code: &str, message: impl Into<String>) -> Diagnostics {
        self.error_at(code, message, self.current().span)
    }

    fn error_at(&self, code: &str, message: impl Into<String>, span: Span) -> Diagnostics {
        Diagnostics::single(Diagnostic::new(code, message, span))
    }
}

fn literal(value: CompassValue, span: Span) -> Expr {
    Expr {
        kind: ExprKind::Literal(value),
        span,
    }
}

fn clause_span(clause: Option<&Clause>) -> Option<Span> {
    match clause? {
        Clause::Match(value) => Some(value.span),
        Clause::Unwind(value) => Some(value.span),
        Clause::With(value) | Clause::Return(value) => Some(value.span),
    }
}
