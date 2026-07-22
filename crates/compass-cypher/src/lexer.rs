use crate::{CompileLimits, Diagnostic, Diagnostics, Span, Token, TokenKind};

pub fn lex(source: &str, limits: CompileLimits) -> Result<Vec<Token>, Diagnostics> {
    if source.len() > limits.max_source_bytes {
        return Err(Diagnostics::single(Diagnostic::new(
            "CQL3000",
            format!("query source exceeds {} bytes", limits.max_source_bytes),
            Span::new(0, source.len()),
        )));
    }
    Lexer::new(source, limits).lex()
}

struct Lexer<'a> {
    source: &'a str,
    offset: usize,
    limits: CompileLimits,
    tokens: Vec<Token>,
}

impl<'a> Lexer<'a> {
    fn new(source: &'a str, limits: CompileLimits) -> Self {
        Self {
            source,
            offset: 0,
            limits,
            tokens: Vec::new(),
        }
    }

    fn lex(mut self) -> Result<Vec<Token>, Diagnostics> {
        while self.offset < self.source.len() {
            self.skip_space_and_comments()?;
            if self.offset >= self.source.len() {
                break;
            }
            if self.tokens.len() >= self.limits.max_tokens {
                return Err(self.error("CQL3001", "query token limit exceeded", self.offset));
            }
            let start = self.offset;
            let character = self.current_char().ok_or_else(|| {
                self.error("CQL1002", "invalid UTF-8 cursor boundary", self.offset)
            })?;
            if character == '`' {
                self.lex_escaped_identifier(start)?;
            } else if character == '\'' || character == '"' {
                self.lex_string(start, character)?;
            } else if character == '$' {
                self.lex_parameter(start)?;
            } else if character.is_ascii_digit() {
                self.lex_number(start);
            } else if is_identifier_start(character) {
                self.lex_identifier(start);
            } else {
                self.lex_symbol(start, character)?;
            }
        }
        self.tokens.push(Token {
            kind: TokenKind::Eof,
            text: String::new(),
            span: Span::new(self.source.len(), self.source.len()),
        });
        Ok(self.tokens)
    }

    fn skip_space_and_comments(&mut self) -> Result<(), Diagnostics> {
        loop {
            while self.current_char().is_some_and(char::is_whitespace) {
                self.advance_char();
            }
            if self.remaining().starts_with("//") {
                while self.current_char().is_some_and(|value| value != '\n') {
                    self.advance_char();
                }
            } else if self.remaining().starts_with("/*") {
                let start = self.offset;
                self.offset += 2;
                let Some(relative_end) = self.remaining().find("*/") else {
                    return Err(self.error_at(
                        "CQL1004",
                        "unterminated block comment",
                        Span::new(start, self.source.len()),
                    ));
                };
                self.offset += relative_end + 2;
            } else {
                return Ok(());
            }
        }
    }

    fn lex_escaped_identifier(&mut self, start: usize) -> Result<(), Diagnostics> {
        self.advance_char();
        let mut text = String::new();
        loop {
            let Some(character) = self.current_char() else {
                return Err(self.error_at(
                    "CQL1005",
                    "unterminated escaped identifier",
                    Span::new(start, self.source.len()),
                ));
            };
            self.advance_char();
            if character == '`' {
                if self.current_char() == Some('`') {
                    self.advance_char();
                    text.push('`');
                } else {
                    break;
                }
            } else {
                text.push(character);
            }
        }
        self.push(TokenKind::Identifier, text, start);
        Ok(())
    }

    fn lex_string(&mut self, start: usize, quote: char) -> Result<(), Diagnostics> {
        self.advance_char();
        let mut text = String::new();
        loop {
            let Some(character) = self.current_char() else {
                return Err(self.error_at(
                    "CQL1003",
                    "unterminated string literal",
                    Span::new(start, self.source.len()),
                ));
            };
            self.advance_char();
            if character == quote {
                break;
            }
            if character == '\\' {
                let Some(escaped) = self.current_char() else {
                    return Err(self.error_at(
                        "CQL1003",
                        "unterminated string escape",
                        Span::new(start, self.source.len()),
                    ));
                };
                self.advance_char();
                match escaped {
                    'n' => text.push('\n'),
                    'r' => text.push('\r'),
                    't' => text.push('\t'),
                    '\\' => text.push('\\'),
                    '\'' => text.push('\''),
                    '"' => text.push('"'),
                    other => text.push(other),
                }
            } else {
                text.push(character);
            }
        }
        self.push(TokenKind::String, text, start);
        Ok(())
    }

    fn lex_parameter(&mut self, start: usize) -> Result<(), Diagnostics> {
        self.advance_char();
        let Some(character) = self.current_char() else {
            return Err(self.error("CQL1006", "parameter name is missing", start));
        };
        if !is_identifier_start(character) {
            return Err(self.error("CQL1006", "invalid parameter name", start));
        }
        let name_start = self.offset;
        while self.current_char().is_some_and(is_identifier_continue) {
            self.advance_char();
        }
        self.push(
            TokenKind::Parameter,
            self.source[name_start..self.offset].to_owned(),
            start,
        );
        Ok(())
    }

    fn lex_number(&mut self, start: usize) {
        while self
            .current_char()
            .is_some_and(|value| value.is_ascii_digit())
        {
            self.advance_char();
        }
        let mut kind = TokenKind::Integer;
        if self.current_char() == Some('.') && !self.remaining().starts_with("..") {
            kind = TokenKind::Float;
            self.advance_char();
            while self
                .current_char()
                .is_some_and(|value| value.is_ascii_digit())
            {
                self.advance_char();
            }
        }
        if self
            .current_char()
            .is_some_and(|value| matches!(value, 'e' | 'E'))
        {
            kind = TokenKind::Float;
            self.advance_char();
            if self
                .current_char()
                .is_some_and(|value| matches!(value, '+' | '-'))
            {
                self.advance_char();
            }
            while self
                .current_char()
                .is_some_and(|value| value.is_ascii_digit())
            {
                self.advance_char();
            }
        }
        self.push(kind, self.source[start..self.offset].to_owned(), start);
    }

    fn lex_identifier(&mut self, start: usize) {
        self.advance_char();
        while self.current_char().is_some_and(is_identifier_continue) {
            self.advance_char();
        }
        let text = &self.source[start..self.offset];
        self.push(keyword(text), text.to_owned(), start);
    }

    fn lex_symbol(&mut self, start: usize, character: char) -> Result<(), Diagnostics> {
        let (kind, bytes) = match self.remaining() {
            value if value.starts_with("<-") => (TokenKind::ArrowLeft, 2),
            value if value.starts_with("->") => (TokenKind::ArrowRight, 2),
            value if value.starts_with("<=") => (TokenKind::LessEqual, 2),
            value if value.starts_with(">=") => (TokenKind::GreaterEqual, 2),
            value if value.starts_with("<>") || value.starts_with("!=") => (TokenKind::NotEqual, 2),
            value if value.starts_with("=~") => (TokenKind::RegexMatch, 2),
            value if value.starts_with("..") => (TokenKind::DotDot, 2),
            _ => match character {
                '(' => (TokenKind::LParen, 1),
                ')' => (TokenKind::RParen, 1),
                '[' => (TokenKind::LBracket, 1),
                ']' => (TokenKind::RBracket, 1),
                '{' => (TokenKind::LBrace, 1),
                '}' => (TokenKind::RBrace, 1),
                ',' => (TokenKind::Comma, 1),
                '.' => (TokenKind::Dot, 1),
                ':' => (TokenKind::Colon, 1),
                '|' => (TokenKind::Pipe, 1),
                '+' => (TokenKind::Plus, 1),
                '-' => (TokenKind::Minus, 1),
                '*' => (TokenKind::Star, 1),
                '/' => (TokenKind::Slash, 1),
                '%' => (TokenKind::Percent, 1),
                '^' => (TokenKind::Caret, 1),
                '=' => (TokenKind::Equal, 1),
                '<' => (TokenKind::Less, 1),
                '>' => (TokenKind::Greater, 1),
                ';' => (TokenKind::Semicolon, 1),
                _ => {
                    return Err(self.error(
                        "CQL1002",
                        format!("unexpected character '{character}'"),
                        start,
                    ));
                }
            },
        };
        self.offset += bytes;
        self.push(kind, self.source[start..self.offset].to_owned(), start);
        Ok(())
    }

    fn push(&mut self, kind: TokenKind, text: String, start: usize) {
        self.tokens.push(Token {
            kind,
            text,
            span: Span::new(start, self.offset),
        });
    }

    fn current_char(&self) -> Option<char> {
        self.remaining().chars().next()
    }

    fn advance_char(&mut self) {
        if let Some(character) = self.current_char() {
            self.offset += character.len_utf8();
        }
    }

    fn remaining(&self) -> &str {
        &self.source[self.offset..]
    }

    fn error(&self, code: &str, message: impl Into<String>, offset: usize) -> Diagnostics {
        self.error_at(code, message, Span::new(offset, offset.saturating_add(1)))
    }

    fn error_at(&self, code: &str, message: impl Into<String>, span: Span) -> Diagnostics {
        Diagnostics::single(Diagnostic::new(code, message, span))
    }
}

fn is_identifier_start(character: char) -> bool {
    character == '_' || character.is_alphabetic()
}

fn is_identifier_continue(character: char) -> bool {
    character == '_' || character.is_alphanumeric()
}

fn keyword(text: &str) -> TokenKind {
    match text.to_ascii_uppercase().as_str() {
        "MATCH" => TokenKind::Match,
        "OPTIONAL" => TokenKind::Optional,
        "WHERE" => TokenKind::Where,
        "RETURN" => TokenKind::Return,
        "WITH" => TokenKind::With,
        "UNWIND" => TokenKind::Unwind,
        "AS" => TokenKind::As,
        "UNION" => TokenKind::Union,
        "ALL" => TokenKind::All,
        "DISTINCT" => TokenKind::Distinct,
        "ORDER" => TokenKind::Order,
        "BY" => TokenKind::By,
        "ASC" | "ASCENDING" => TokenKind::Asc,
        "DESC" | "DESCENDING" => TokenKind::Desc,
        "SKIP" => TokenKind::Skip,
        "LIMIT" => TokenKind::Limit,
        "AND" => TokenKind::And,
        "OR" => TokenKind::Or,
        "XOR" => TokenKind::Xor,
        "NOT" => TokenKind::Not,
        "IN" => TokenKind::In,
        "IS" => TokenKind::Is,
        "NULL" => TokenKind::Null,
        "TRUE" => TokenKind::True,
        "FALSE" => TokenKind::False,
        "STARTS" => TokenKind::Starts,
        "ENDS" => TokenKind::Ends,
        "CONTAINS" => TokenKind::Contains,
        "CASE" => TokenKind::Case,
        "WHEN" => TokenKind::When,
        "THEN" => TokenKind::Then,
        "ELSE" => TokenKind::Else,
        "END" => TokenKind::End,
        "EXISTS" => TokenKind::Exists,
        "EXPLAIN" => TokenKind::Explain,
        "PROFILE" => TokenKind::Profile,
        "CREATE" => TokenKind::Create,
        "MERGE" => TokenKind::Merge,
        "DELETE" => TokenKind::Delete,
        "DETACH" => TokenKind::Detach,
        "SET" => TokenKind::Set,
        "REMOVE" => TokenKind::Remove,
        "CALL" => TokenKind::Call,
        "LOAD" => TokenKind::Load,
        "FOREACH" => TokenKind::Foreach,
        "USE" => TokenKind::Use,
        "YIELD" => TokenKind::Yield,
        _ => TokenKind::Identifier,
    }
}
