use core::fmt;

use tap::tooling::colors::set::colorize;

#[derive(Debug)]
pub enum ParseError {
    MissingName {
        line: usize,
    },
    CantPopStack {
        line: usize,
    },
    MissingParts {
        line: usize,
        content: String,
    },
    InvalidInt {
        line: usize,
        name: String,
        value: String,
        source: std::num::ParseIntError,
    },
    InvalidFloat {
        line: usize,
        name: String,
        value: String,
        source: std::num::ParseFloatError,
    },
    InvalidBool {
        line: usize,
        name: String,
        value: String,
    },
    BadEscape {
        line: usize,
        name: String,
        value: String,
        source: String,
    },
    BadBase64 {
        line: usize,
        name: String,
        value: String,
        source: base64::DecodeError,
    },
    UnknownType {
        line: usize,
        name: String,
        ty: String,
    },
}

impl ParseError {
    pub fn line(&self) -> usize {
        match self {
            ParseError::MissingName { line }
            | ParseError::CantPopStack { line }
            | ParseError::MissingParts { line, .. }
            | ParseError::InvalidInt { line, .. }
            | ParseError::InvalidFloat { line, .. }
            | ParseError::InvalidBool { line, .. }
            | ParseError::BadEscape { line, .. }
            | ParseError::BadBase64 { line, .. }
            | ParseError::UnknownType { line, .. } => *line,
        }
    }

    fn message(&self) -> String {
        match self {
            ParseError::MissingName { .. } => "missing name in block header".into(),
            ParseError::CantPopStack { .. } => "cannot close block: stack is empty".into(),
            ParseError::MissingParts { content, .. } => {
                format!("expected `<name> <type> <value>`, got `{content}`")
            }
            ParseError::InvalidInt {
                name, value, source, ..
            } => {
                format!("field `{name}`: invalid int `{value}` ({source})")
            }
            ParseError::InvalidFloat {
                name, value, source, ..
            } => {
                format!("field `{name}`: invalid float `{value}` ({source})")
            }
            ParseError::InvalidBool { name, value, .. } => {
                format!("field `{name}`: invalid bool `{value}` (expected `true` or `false`)")
            }
            ParseError::BadEscape {
                name, value, source, ..
            } => {
                format!("field `{name}`: bad escape in `{value}` ({source})")
            }
            ParseError::BadBase64 {
                name, value, source, ..
            } => {
                format!("field `{name}`: invalid base64 `{value}` ({source})")
            }
            ParseError::UnknownType { name, ty, .. } => {
                format!("field `{name}`: unknown type `{ty}`")
            }
        }
    }

    fn highlight_span(&self, src_line: &str) -> (usize, usize) {
        let needle: Option<&str> = match self {
            ParseError::InvalidInt { value, .. }
            | ParseError::InvalidFloat { value, .. }
            | ParseError::InvalidBool { value, .. }
            | ParseError::BadEscape { value, .. }
            | ParseError::BadBase64 { value, .. } => Some(value),
            ParseError::UnknownType { ty, .. } => Some(ty),
            _ => None,
        };

        if let Some(n) = needle {
            if let Some(byte_idx) = src_line.find(n) {
                let start = src_line[..byte_idx].chars().count();
                let len = n.chars().count();
                return (start, len);
            }
        }

        (0, src_line.chars().count())
    }

    pub fn trace(&self, source: &str) -> String {
        use core::fmt::Write;

        let line_no = self.line();
        let mut out = String::new();

        let _ = writeln!(
            out,
            "{} {}: {}",
            colorize("[?BD][?RD]error[?RT]:[?YW] line"),
            line_no,
            self.message()
        );

        let lines: Vec<&str> = source.lines().collect();
        let src_line = match line_no.checked_sub(1).and_then(|i| lines.get(i)) {
            Some(l) => *l,
            None => return out.trim_end().to_string(),
        };

        let line_label = line_no.to_string();
        let gutter = line_label.len();

        let color_red = colorize("[?BKBK][?TL]");
        let rt = colorize("[?RT]");

        let _ = write!(out, "{}", color_red);
        let _ = writeln!(out, "{:width$} |{}", "", rt, width = gutter);
        let _ = writeln!(
            out,
            "{}{:>width$} |{} {}",
            color_red,
            line_label,
            rt,
            src_line,
            width = gutter
        );

        let (start, len) = self.highlight_span(src_line);
        let mut caret = String::with_capacity(start + len.max(1));
        for _ in 0..start {
            caret.push(' ');
        }
        for idx in 0..len.max(1) {
            let c = src_line.chars().nth(start + idx);
            if matches!(c, Some(ch) if ch.is_whitespace()) {
                caret.push(' ');
            } else {
                caret.push('^');
            }
        }
        caret = colorize(format!("[?RD][?BD]{}[?RT]", caret).as_str());

        let _ = write!(out, "{}", color_red);
        let _ = writeln!(out, "{:width$} |{} {}", "", rt, caret, width = gutter);
        let _ = writeln!(out, "{}", rt);

        out.trim_end().to_string()
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line(), self.message())
    }
}

impl std::error::Error for ParseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ParseError::InvalidInt { source, .. } => Some(source),
            ParseError::InvalidFloat { source, .. } => Some(source),
            ParseError::BadBase64 { source, .. } => Some(source),
            _ => None,
        }
    }
}

pub fn format_errors(errors: &[ParseError]) -> String {
    use core::fmt::Write;

    let mut out = String::new();
    for (i, e) in errors.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let _ = write!(out, "{e}");
    }
    out
}

pub fn format_errors_traced(errors: &[ParseError], source: &str) -> String {
    let mut out = String::new();
    for (i, e) in errors.iter().enumerate() {
        if i > 0 {
            out.push_str("\n\n");
        }
        out.push_str(&e.trace(source));
    }
    out
}