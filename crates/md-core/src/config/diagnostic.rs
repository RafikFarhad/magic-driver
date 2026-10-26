use std::fmt;

use super::CONFIG_VERSION;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DiagnosticKind {
    #[error("invalid TOML: {0}")]
    Syntax(String),
    #[error("invalid value: {0}")]
    InvalidValue(String),
    #[error("value {value} is out of range, expected {expected}")]
    OutOfRange { value: String, expected: String },
    #[error("unsupported config version {0}, expected {CONFIG_VERSION}")]
    UnsupportedVersion(u32),
    #[error("unknown key, ignored")]
    UnknownKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub line: Option<usize>,
    pub key: Option<String>,
    pub kind: DiagnosticKind,
}

impl Diagnostic {
    pub(crate) fn new(
        source: &str,
        offset: Option<usize>,
        key: Option<&str>,
        kind: DiagnosticKind,
    ) -> Self {
        Diagnostic {
            line: offset.map(|offset| line_of_offset(source, offset)),
            key: key.map(str::to_owned),
            kind,
        }
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(line) = self.line {
            write!(f, "line {line}: ")?;
        }
        if let Some(key) = &self.key {
            write!(f, "{key}: ")?;
        }
        write!(f, "{}", self.kind)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{} config error(s)", errors.len())]
pub struct ConfigErrors {
    pub errors: Vec<Diagnostic>,
    pub warnings: Vec<Diagnostic>,
}

fn line_of_offset(source: &str, offset: usize) -> usize {
    let end = offset.min(source.len());
    source.get(..end).unwrap_or(source).matches('\n').count() + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_of_offset_counts_newlines_before_offset() {
        let source = "a = 1\nb = 2\n\nc = 3";
        assert_eq!(line_of_offset(source, 0), 1);
        assert_eq!(line_of_offset(source, 5), 1);
        assert_eq!(line_of_offset(source, 6), 2);
        assert_eq!(line_of_offset(source, 13), 4);
        assert_eq!(line_of_offset(source, 1_000), 4);
    }

    #[test]
    fn diagnostic_display_includes_line_and_key() {
        let diagnostic = Diagnostic::new(
            "x\ny = 3",
            Some(2),
            Some("scroll.y"),
            DiagnosticKind::UnknownKey,
        );
        assert_eq!(
            diagnostic.to_string(),
            "line 2: scroll.y: unknown key, ignored"
        );
    }
}
