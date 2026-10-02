//! UTF-8 byte budgets for completed human-readable command output.

use crate::OutputError;

#[derive(Debug, Eq, PartialEq)]
pub struct BudgetedText {
    pub text: String,
    pub end: usize,
}

/// Keep whole UTF-8 characters, including the continuation instruction and
/// the newline written by the CLI. Offsets are bytes in the immutable answer.
pub fn render_budgeted_text(
    text: &str,
    start: usize,
    tokens: usize,
    continuation: impl Fn(usize) -> String,
) -> Result<BudgetedText, OutputError> {
    if start > text.len() || !text.is_char_boundary(start) {
        return Err(OutputError::InvalidAgentTextPage(
            "invalid output offset".to_owned(),
        ));
    }
    let limit = tokens.saturating_mul(4).saturating_sub(1);
    if text.len().saturating_sub(start) <= limit {
        return Ok(BudgetedText {
            text: text[start..].to_owned(),
            end: text.len(),
        });
    }
    let mut end = start.saturating_add(limit).min(text.len());
    loop {
        while end > start && !text.is_char_boundary(end) {
            end -= 1;
        }
        let footer = continuation(end);
        if end
            .saturating_sub(start)
            .saturating_add(footer.len())
            .saturating_add(1)
            <= limit
        {
            return Ok(BudgetedText {
                text: format!("{}\n{footer}", &text[start..end]),
                end,
            });
        }
        if end == start {
            return Err(OutputError::InvalidAgentTextPage(
                "budget too small for continuation; use --budget 256".to_owned(),
            ));
        }
        end -= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_pages_respect_bytes_and_reconstruct_the_answer()
    -> Result<(), Box<dyn std::error::Error>> {
        let input = "é東京🙂\n".repeat(100);
        let mut restored = String::new();
        let mut start = 0;
        while start < input.len() {
            let page =
                render_budgeted_text(&input, start, 64, |end| format!("More: --offset {end}"))?;
            assert!(page.text.len() + 1 <= 256);
            assert!(page.end > start);
            restored.push_str(&input[start..page.end]);
            start = page.end;
        }
        assert_eq!(restored, input);
        assert!(render_budgeted_text("é", 1, 64, |_| String::new()).is_err());
        Ok(())
    }
}
