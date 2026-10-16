//! Serialization utilities for the Tessera language.
//!
//! This module converts between the in-memory [`Config`] and textual
//! representations:
//!
//! * [`to_tessera`] / [`try_to_tessera`] — [`Config`] → Tessera.
//! * [`to_json`] — [`Config`] → JSON.
//! * [`from_json`] — JSON → [`Config`].
//! * [`tessera_to_json`] — Tessera source → JSON (with parse errors).
//! * [`json_to_tessera`] — JSON → Tessera.
//!
//! ## Representability caveats
//!
//! Tessera is a line-oriented format with a tiny grammar, so a few
//! keys/values that can exist in a [`Config`] cannot be written back
//! verbatim:
//!
//! * **Field keys** must be non-empty, contain no ASCII space (the parser
//!   splits the line on the first space) and must not start with `<` or
//!   `#` (which would be interpreted as a block-open / comment).
//! * **Block keys** are trimmed by the parser; leading/trailing ASCII
//!   whitespace is therefore lost.
//! * **Empty byte values** cannot be encoded on a single line: base64 of
//!   `[]` is `""`, which after `trim_ascii` leaves a two-token line and is
//!   rejected with `MissingParts`.
//!
//! [`to_tessera`] silently skips such fields (best effort);
//! [`try_to_tessera`] returns a [`GenError`] instead.

use base64::Engine;
use compact_str::CompactString;
use std::fmt::Write;

use crate::{
    errors::ParseError,
    parsing::{end_block, parse_into, start_block},
    structures::Config,
};

const INDENT: &str = "  ";

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// A key or value that cannot be written back as Tessera text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GenError {
    /// A field key is empty, contains an ASCII space, or starts with `<`/`#`.
    InvalidFieldKey(String),
    /// A block key contains leading/trailing ASCII whitespace or is empty.
    InvalidBlockKey(String),
    /// A byte value is empty: base64 of `[]` is `""`, and after
    /// `trim_ascii` the line would have only two tokens.
    EmptyBytes(String),
}

impl std::fmt::Display for GenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GenError::InvalidFieldKey(k) => write!(f, "field key cannot be represented: {k:?}"),
            GenError::InvalidBlockKey(k) => write!(f, "block key cannot be represented: {k:?}"),
            GenError::EmptyBytes(k) => {
                write!(f, "empty byte value for key {k:?} cannot be encoded")
            }
        }
    }
}

impl std::error::Error for GenError {}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

#[inline]
fn write_indent(out: &mut String, level: usize) {
    for _ in 0..level {
        out.push_str(INDENT);
    }
}

/// Collect `(key, value)` pairs from a map-like collection and sort by key.
fn sorted_pairs<'a, K, V, I>(collection: I) -> Vec<(&'a K, &'a V)>
where
    I: IntoIterator<Item = (&'a K, &'a V)>,
    K: Ord + 'a,
{
    let mut items: Vec<(&'a K, &'a V)> = collection.into_iter().collect();
    items.sort_by(|a, b| a.0.cmp(b.0));
    items
}

/// A field key is representable iff it is non-empty, contains no ASCII
/// space (the parser splits on `' '`), and does not start with `<` or `#`.
fn is_valid_field_key(k: &str) -> bool {
    if k.is_empty() {
        return false;
    }
    let bytes = k.as_bytes();
    if bytes.contains(&b' ') {
        return false;
    }
    !matches!(bytes[0], b'<' | b'#')
}

// ---------------------------------------------------------------------------
// Tessera output
// ---------------------------------------------------------------------------

/// Best-effort conversion of `cfg` into Tessera text.
///
/// Fields/blocks that cannot be represented (see [`GenError`]) are
/// silently skipped. Use [`try_to_tessera`] if you need to detect them.
pub fn to_tessera(cfg: &Config) -> String {
    let mut out = String::new();
    let mut errors = Vec::new();
    write_tessera(cfg, &mut out, 0, &mut errors);
    out
}

/// Strict conversion of `cfg` into Tessera text.
pub fn try_to_tessera(cfg: &Config) -> Result<String, GenError> {
    let mut out = String::new();
    let mut errors = Vec::new();
    write_tessera(cfg, &mut out, 0, &mut errors);
    match errors.into_iter().next() {
        Some(e) => Err(e),
        None => Ok(out),
    }
}

fn write_tessera(cfg: &Config, out: &mut String, level: usize, errors: &mut Vec<GenError>) {
    for (k, v) in sorted_pairs(&cfg.ints) {
        if !is_valid_field_key(k) {
            errors.push(GenError::InvalidFieldKey(k.to_string()));
            continue;
        }
        write_indent(out, level);
        let _ = writeln!(out, "{k} i {v}");
    }

    for (k, v) in sorted_pairs(&cfg.strs) {
        if !is_valid_field_key(k) {
            errors.push(GenError::InvalidFieldKey(k.to_string()));
            continue;
        }
        write_indent(out, level);
        let _ = write!(out, "{k} s \"");
        escape_tessera_str(v, out);
        let _ = writeln!(out, "\"");
    }

    for (k, v) in sorted_pairs(&cfg.floats) {
        if !is_valid_field_key(k) {
            errors.push(GenError::InvalidFieldKey(k.to_string()));
            continue;
        }
        write_indent(out, level);
        // `format!("{}", f64)` produces a value that `str::parse::<f64>`
        // accepts back, including `NaN`, `inf`, `-inf`.
        let _ = writeln!(out, "{k} f {v}");
    }

    for (k, v) in sorted_pairs(&cfg.bools) {
        if !is_valid_field_key(k) {
            errors.push(GenError::InvalidFieldKey(k.to_string()));
            continue;
        }
        write_indent(out, level);
        let _ = writeln!(out, "{k} l {v}");
    }

    for (k, v) in sorted_pairs(&cfg.bytes) {
        if !is_valid_field_key(k) {
            errors.push(GenError::InvalidFieldKey(k.to_string()));
            continue;
        }
        let encoded = base64::engine::general_purpose::STANDARD.encode(v);
        if encoded.is_empty() {
            errors.push(GenError::EmptyBytes(k.to_string()));
            continue;
        }
        write_indent(out, level);
        let _ = writeln!(out, "{k} b {encoded}");
    }

    for (k, child) in sorted_pairs(&cfg.inners) {
        let trimmed = k.trim_ascii();
        if trimmed.is_empty() || trimmed != k.as_str() {
            errors.push(GenError::InvalidBlockKey(k.to_string()));
            continue;
        }
        write_indent(out, level);
        let _ = writeln!(out, "{start_block}{trimmed}");
        write_tessera(child, out, level + 1, errors);
        write_indent(out, level);
        let _ = writeln!(out, "{end_block}");
    }
}

fn escape_tessera_str(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                // All Unicode control characters fit in 4 hex digits.
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
}

// ---------------------------------------------------------------------------
// JSON output
// ---------------------------------------------------------------------------

/// Pretty-printed JSON from a [`Config`].
pub fn to_json(cfg: &Config) -> String {
    let mut out = String::new();
    write_json(cfg, &mut out, 0);
    out
}

fn write_json(cfg: &Config, out: &mut String, level: usize) {
    let is_empty = cfg.ints.is_empty()
        && cfg.strs.is_empty()
        && cfg.floats.is_empty()
        && cfg.bools.is_empty()
        && cfg.bytes.is_empty()
        && cfg.inners.is_empty();

    if is_empty {
        out.push_str("{}");
        return;
    }

    out.push('{');
    let child_level = level + 1;
    let mut first = true;

    for (k, v) in sorted_pairs(&cfg.ints) {
        json_sep(out, &mut first, child_level);
        write_json_key(k, out);
        let _ = write!(out, ": {v}");
    }

    for (k, v) in sorted_pairs(&cfg.strs) {
        json_sep(out, &mut first, child_level);
        write_json_key(k, out);
        out.push_str(": ");
        write_json_str(v, out);
    }

    for (k, v) in sorted_pairs(&cfg.floats) {
        json_sep(out, &mut first, child_level);
        write_json_key(k, out);
        let _ = write!(out, ": {}", format_json_float(*v));
    }

    for (k, v) in sorted_pairs(&cfg.bools) {
        json_sep(out, &mut first, child_level);
        write_json_key(k, out);
        let _ = write!(out, ": {v}");
    }

    // Bytes are encoded as an array of numbers so `from_json` round-trips them.
    for (k, v) in sorted_pairs(&cfg.bytes) {
        json_sep(out, &mut first, child_level);
        write_json_key(k, out);
        out.push_str(": [");
        for (i, b) in v.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            let _ = write!(out, "{b}");
        }
        out.push(']');
    }

    for (k, child) in sorted_pairs(&cfg.inners) {
        json_sep(out, &mut first, child_level);
        write_json_key(k, out);
        out.push_str(": ");
        write_json(child, out, child_level);
    }

    out.push('\n');
    write_indent(out, level);
    out.push('}');
}

fn json_sep(out: &mut String, first: &mut bool, level: usize) {
    if *first {
        *first = false;
    } else {
        out.push(',');
    }
    out.push('\n');
    write_indent(out, level);
}

fn write_json_key(k: &str, out: &mut String) {
    out.push('"');
    escape_json_str(k, out);
    out.push('"');
}

fn write_json_str(s: &str, out: &mut String) {
    out.push('"');
    escape_json_str(s, out);
    out.push('"');
}

fn escape_json_str(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
}

/// JSON has no representation for NaN / ±∞; emit `null` instead.
fn format_json_float(v: f64) -> String {
    if !v.is_finite() {
        "null".to_string()
    } else if v.fract() == 0.0 {
        format!("{v:.1}")
    } else {
        format!("{v}")
    }
}

// ---------------------------------------------------------------------------
// Convenience conversions
// ---------------------------------------------------------------------------

/// Parse Tessera source into a [`Config`] and serialize it as JSON.
pub fn tessera_to_json(code: &str) -> (String, Vec<ParseError>) {
    let mut cfg = Config::default();
    let errors = parse_into(&mut cfg, code);
    (to_json(&cfg), errors)
}

/// Parse JSON into a [`Config`] and serialize it as Tessera.
pub fn json_to_tessera(json: &str) -> Result<String, String> {
    let cfg = from_json(json)?;
    try_to_tessera(&cfg).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// JSON input
// ---------------------------------------------------------------------------

/// Parse JSON into a [`Config`].
pub fn from_json(json: &str) -> Result<Config, String> {
    let mut parser = JsonParser::new(json);
    parser.parse_config()
}

struct JsonParser<'a> {
    input: &'a str,
    pos: usize,
}

impl<'a> JsonParser<'a> {
    fn new(input: &'a str) -> Self {
        Self { input, pos: 0 }
    }

    fn peek(&self) -> Option<char> {
        self.input[self.pos..].chars().next()
    }

    fn next(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn skip_whitespace(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_whitespace() {
                self.next();
            } else {
                break;
            }
        }
    }

    fn expect(&mut self, expected: char) -> Result<(), String> {
        self.skip_whitespace();
        match self.next() {
            Some(c) if c == expected => Ok(()),
            Some(c) => Err(format!("expected '{expected}', found '{c}'")),
            None => Err(format!("expected '{expected}', found end of input")),
        }
    }

    fn parse_config(&mut self) -> Result<Config, String> {
        self.skip_whitespace();
        self.expect('{')?;
        let mut cfg = Config::default();
        self.skip_whitespace();
        if self.peek() == Some('}') {
            self.next();
            return Ok(cfg);
        }
        loop {
            self.skip_whitespace();
            let key = self.parse_string()?;
            self.skip_whitespace();
            self.expect(':')?;
            self.skip_whitespace();
            match self.peek() {
                Some('{') => {
                    let child = self.parse_config()?;
                    cfg.inners.insert(CompactString::from(key), child);
                }
                Some('"') => {
                    let val = self.parse_string()?;
                    cfg.strs
                        .insert(CompactString::from(key), CompactString::from(val));
                }
                Some('[') => {
                    self.next(); // consume '['
                    let mut buf: Vec<u8> = Vec::new();
                    self.skip_whitespace();
                    if self.peek() == Some(']') {
                        self.next();
                    } else {
                        loop {
                            self.skip_whitespace();
                            let n = self.parse_number()?;
                            let b: u8 = n
                                .parse()
                                .map_err(|e| format!("invalid byte value `{n}`: {e}"))?;
                            buf.push(b);
                            self.skip_whitespace();
                            match self.next() {
                                Some(',') => continue,
                                Some(']') => break,
                                Some(c) => {
                                    return Err(format!("expected ',' or ']', found '{c}'"))
                                }
                                None => return Err("unexpected end of input".to_string()),
                            }
                        }
                    }
                    cfg.bytes.insert(CompactString::from(key), buf);
                }
                Some('t') | Some('f') => {
                    let rest = &self.input[self.pos..];
                    if rest.starts_with("true") {
                        self.pos += 4;
                        cfg.bools.insert(CompactString::from(key), true);
                    } else if rest.starts_with("false") {
                        self.pos += 5;
                        cfg.bools.insert(CompactString::from(key), false);
                    } else {
                        return Err(format!(
                            "invalid boolean literal near `{}`",
                            &rest[..rest.len().min(8)]
                        ));
                    }
                }
                Some('n') => {
                    // `null` → пропускаем, значение не вставляем.
                    let rest = &self.input[self.pos..];
                    if rest.starts_with("null") {
                        self.pos += 4;
                    } else {
                        return Err(format!(
                            "invalid literal near `{}`",
                            &rest[..rest.len().min(8)]
                        ));
                    }
                }
                Some(c) if c == '-' || c.is_ascii_digit() => {
                    let num = self.parse_number()?;
                    if num.contains('.') || num.contains('e') || num.contains('E') {
                        let v: f64 = num.parse().map_err(|e| format!("invalid float: {e}"))?;
                        cfg.floats.insert(CompactString::from(key), v);
                    } else {
                        let v: i64 = num.parse().map_err(|e| format!("invalid int: {e}"))?;
                        cfg.ints.insert(CompactString::from(key), v);
                    }
                }
                Some(c) => {
                    return Err(format!("unexpected character '{c}' for value"));
                }
                None => return Err("unexpected end of input".to_string()),
            }
            self.skip_whitespace();
            match self.next() {
                Some(',') => continue,
                Some('}') => break,
                Some(c) => return Err(format!("expected ',' or '}}', found '{c}'")),
                None => return Err("unexpected end of input".to_string()),
            }
        }
        Ok(cfg)
    }

    fn parse_string(&mut self) -> Result<String, String> {
        self.expect('"')?;
        let mut s = String::new();
        loop {
            match self.next() {
                Some('"') => break,
                Some('\\') => match self.next() {
                    Some('"') => s.push('"'),
                    Some('\\') => s.push('\\'),
                    Some('/') => s.push('/'),
                    Some('b') => s.push('\u{08}'),
                    Some('f') => s.push('\u{0c}'),
                    Some('n') => s.push('\n'),
                    Some('r') => s.push('\r'),
                    Some('t') => s.push('\t'),
                    Some('u') => {
                        let c1 = self.parse_hex4()?;
                        // Учёт суррогатных пар: \uD83D\uDE00 → 😀
                        if (0xD800..=0xDBFF).contains(&c1) {
                            if self.next() != Some('\\') || self.next() != Some('u') {
                                return Err(
                                    "expected low surrogate after high surrogate".to_string()
                                );
                            }
                            let c2 = self.parse_hex4()?;
                            if !(0xDC00..=0xDFFF).contains(&c2) {
                                return Err("invalid low surrogate".to_string());
                            }
                            let code = 0x10000 + ((c1 - 0xD800) << 10) + (c2 - 0xDC00);
                            let ch = char::from_u32(code).ok_or_else(|| {
                                format!("invalid surrogate pair: {code:#x}")
                            })?;
                            s.push(ch);
                        } else {
                            let ch = char::from_u32(c1).ok_or_else(|| {
                                format!("invalid unicode code point: {c1:#x}")
                            })?;
                            s.push(ch);
                        }
                    }
                    Some(c) => return Err(format!("invalid escape character '{c}'")),
                    None => return Err("unexpected end of input in escape".to_string()),
                },
                Some(c) => s.push(c),
                None => return Err("unexpected end of input in string".to_string()),
            }
        }
        Ok(s)
    }

    fn parse_hex4(&mut self) -> Result<u32, String> {
        let mut code = 0u32;
        for _ in 0..4 {
            let c = self.next().ok_or("unexpected end in unicode escape")?;
            let digit = c
                .to_digit(16)
                .ok_or_else(|| format!("invalid hex digit '{c}'"))?;
            code = code * 16 + digit;
        }
        Ok(code)
    }

    fn parse_number(&mut self) -> Result<String, String> {
        let start = self.pos;
        if self.peek() == Some('-') {
            self.next();
        }
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || c == '.' || c == 'e' || c == 'E' || c == '+' || c == '-' {
                self.next();
            } else {
                break;
            }
        }
        let s = &self.input[start..self.pos];
        if s.is_empty() {
            return Err("expected number".to_string());
        }
        Ok(s.to_string())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parsing::parse_into;

    fn parse(text: &str) -> Config {
        let mut cfg = Config::default();
        let errors = parse_into(&mut cfg, text);
        assert!(errors.is_empty(), "parse errors: {errors:?}\n---\n{text}");
        cfg
    }

    #[test]
    fn empty_config_serializes() {
        let cfg = Config::default();
        assert_eq!(to_tessera(&cfg), "");
        assert_eq!(to_json(&cfg), "{}");
    }

    #[test]
    fn scalars_roundtrip_through_tessera() {
        let mut cfg = Config::default();
        cfg.ints.insert("i".into(), -42);
        cfg.strs.insert("s".into(), "hello \"world\"\n\tend".into());
        cfg.floats.insert("f".into(), 3.5);
        cfg.bools.insert("t".into(), true);
        cfg.bools.insert("f2".into(), false);
        cfg.bytes.insert("b".into(), vec![0u8, 1, 2, 255]);

        let text = to_tessera(&cfg);
        let back = parse(&text);

        assert_eq!(back.ints.get("i"), Some(&-42));
        assert_eq!(
            back.strs.get("s").map(|s| s.as_str()),
            Some("hello \"world\"\n\tend")
        );
        assert_eq!(back.floats.get("f"), Some(&3.5));
        assert_eq!(back.bools.get("t"), Some(&true));
        assert_eq!(back.bools.get("f2"), Some(&false));
        assert_eq!(back.bytes.get("b"), Some(&vec![0u8, 1, 2, 255]));
    }

    #[test]
    fn nested_roundtrip_through_tessera() {
        let mut cfg = Config::default();
        let mut child = Config::default();
        child.ints.insert("x".into(), 1);
        let mut grand = Config::default();
        grand.strs.insert("y".into(), "z".into());
        child.inners.insert("g".into(), grand);
        cfg.inners.insert("c".into(), child);

        let text = to_tessera(&cfg);
        let back = parse(&text);
        let c = back.inners.get("c").expect("c");
        assert_eq!(c.ints.get("x"), Some(&1));
        assert_eq!(
            c.inners.get("g").unwrap().strs.get("y").map(|s| s.as_str()),
            Some("z")
        );
    }

    #[test]
    fn empty_bytes_cannot_be_represented() {
        let mut cfg = Config::default();
        cfg.bytes.insert("b".into(), Vec::new());
        assert!(try_to_tessera(&cfg).is_err());
        // Best effort: поле отбрасывается, но вывод остаётся валидным Tessera.
        assert_eq!(to_tessera(&cfg), "");
    }

    #[test]
    fn invalid_field_key_is_reported() {
        let mut cfg = Config::default();
        cfg.ints.insert("a b".into(), 1);
        assert!(try_to_tessera(&cfg).is_err());
        assert_eq!(to_tessera(&cfg), "");
    }

    #[test]
    fn json_roundtrip() {
        let mut cfg = Config::default();
        cfg.ints.insert("i".into(), -42);
        cfg.strs.insert("s".into(), "he\"llo\n".into());
        cfg.floats.insert("f".into(), 1.5);
        cfg.bools.insert("t".into(), true);
        cfg.bytes.insert("b".into(), vec![1, 2, 3]);
        let mut child = Config::default();
        child.ints.insert("x".into(), 7);
        cfg.inners.insert("c".into(), child);

        let json = to_json(&cfg);
        let back = from_json(&json).expect("valid json");

        assert_eq!(back.ints.get("i"), Some(&-42));
        assert_eq!(back.strs.get("s").map(|s| s.as_str()), Some("he\"llo\n"));
        assert_eq!(back.floats.get("f"), Some(&1.5));
        assert_eq!(back.bools.get("t"), Some(&true));
        assert_eq!(back.bytes.get("b"), Some(&vec![1u8, 2, 3]));
        assert_eq!(back.inners.get("c").unwrap().ints.get("x"), Some(&7));
    }

    #[test]
    fn json_non_finite_floats_become_null_and_are_dropped() {
        let mut cfg = Config::default();
        cfg.floats.insert("nan".into(), f64::NAN);
        cfg.floats.insert("inf".into(), f64::INFINITY);
        let json = to_json(&cfg);
        assert!(json.contains("null"));
        let back = from_json(&json).unwrap();
        assert!(back.floats.is_empty());
    }

    #[test]
    fn json_handles_surrogate_pairs() {
        let json = r#"{"emoji": "\uD83D\uDE00"}"#;
        let cfg = from_json(json).unwrap();
        assert_eq!(cfg.strs.get("emoji").map(|s| s.as_str()), Some("😀"));
    }

    #[test]
    fn tessera_to_json_returns_errors() {
        let src = "a i 1\n<block\nb s \"x\"\n>\n";
        let (json, errors) = tessera_to_json(src);
        assert!(errors.is_empty());
        assert!(json.contains("\"a\": 1"));
        assert!(json.contains("\"block\""));
    }
}