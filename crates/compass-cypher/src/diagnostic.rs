use std::fmt;

use serde::{Deserialize, Serialize};

use crate::Span;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Diagnostic {
    code: String,
    message: String,
    span: Span,
    help: Option<String>,
}

impl Diagnostic {
    #[must_use]
    pub fn new(code: impl Into<String>, message: impl Into<String>, span: Span) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            span,
            help: None,
        }
    }

    #[must_use]
    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    #[must_use]
    pub const fn span(&self) -> Span {
        self.span
    }

    #[must_use]
    pub fn help(&self) -> Option<&str> {
        self.help.as_deref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Diagnostics {
    items: Vec<Diagnostic>,
}

impl Diagnostics {
    #[must_use]
    pub fn single(item: Diagnostic) -> Self {
        Self { items: vec![item] }
    }

    #[must_use]
    pub fn from_items(items: Vec<Diagnostic>) -> Option<Self> {
        (!items.is_empty()).then_some(Self { items })
    }

    #[must_use]
    pub fn items(&self) -> &[Diagnostic] {
        &self.items
    }

    pub fn push(&mut self, item: Diagnostic) {
        self.items.push(item);
    }
}

impl fmt::Display for Diagnostics {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, item) in self.items.iter().enumerate() {
            if index > 0 {
                writeln!(formatter)?;
            }
            write!(
                formatter,
                "{} at {}..{}: {}",
                item.code, item.span.start, item.span.end, item.message
            )?;
        }
        Ok(())
    }
}

impl std::error::Error for Diagnostics {}
