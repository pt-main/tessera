use base64::Engine;
use compact_str::CompactString;
use unescape_zero_copy::unescape_default;

use crate::{
    errors::ParseError,
    structures::{Config},
};

pub const start_block: &str = "<";
pub const end_block: &str = ">";


pub fn parse_into(cfg: &mut Config, code: &str) -> Vec<ParseError> {
    let mut errors = Vec::new();

    let mut stack: Vec<(CompactString, Config)> = Vec::with_capacity(8);
    stack.push((CompactString::default(), Config::default()));

    for (idx, raw) in code.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw.trim_ascii();
        if line.is_empty() {
            continue;
        }

        if let Some(rest) = line.strip_prefix(start_block) {
            let name = rest.trim_ascii();
            if name.is_empty() {
                errors.push(ParseError::MissingName { line: line_no });
                continue;
            }
            stack.push((CompactString::from(name), Config::default()));
            continue;
        }

        if line == end_block {
            if stack.len() < 2 {
                errors.push(ParseError::CantPopStack { line: line_no });
                continue;
            }
            let (name, child) = stack.pop().unwrap();
            let parent = &mut stack.last_mut().unwrap().1;
            parent.inners.insert(name, child);
            continue;
        }

        if line.starts_with('#') {
            continue;
        }

        let mut it = line.splitn(3, ' ');
        let (name, vtype, value) = match (it.next(), it.next(), it.next()) {
            (Some(a), Some(b), Some(c)) => {
                let b = b.trim();
                if b.len() != 1 {
                    errors.push(ParseError::UnknownType { line: line_no, name: a.to_string(), ty: b.to_string() });
                        continue;
                }
                let b = (b.chars().next().unwrap());
                (a, b, c)
            },
            _ => {
                errors.push(ParseError::MissingParts {
                    line: line_no,
                    content: line.to_string(),
                });
                continue;
            }
        };

        let last = &mut stack.last_mut().unwrap().1;

        match vtype {
            'i' => match value.parse::<i64>() {
                Ok(v) => {
                    last.ints.insert(CompactString::from(name), v);
                }
                Err(source) => errors.push(ParseError::InvalidInt {
                    line: line_no,
                    name: name.to_string(),
                    value: value.to_string(),
                    source,
                }),
            },

            's' => {
                let raw = match value.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
                    Some(inner) => inner,
                    None => value,
                };
                match unescape_default(raw) {
                    Ok(s) => {
                        last.strs
                            .insert(CompactString::from(name), CompactString::from(s.as_ref()));
                    }
                    Err(e) => errors.push(ParseError::BadEscape {
                        line: line_no,
                        name: name.to_string(),
                        value: value.to_string(),
                        source: e.to_string(),
                    }),
                }
            }

            'f' => match value.parse::<f64>() {
                Ok(v) => {
                    last.floats.insert(CompactString::from(name), v);
                }
                Err(source) => errors.push(ParseError::InvalidFloat {
                    line: line_no,
                    name: name.to_string(),
                    value: value.to_string(),
                    source,
                }),
            },

            'b' => match base64::engine::general_purpose::STANDARD.decode(value) {
                Ok(v) => {
                    last.bytes.insert(CompactString::from(name), v);
                }
                Err(source) => errors.push(ParseError::BadBase64 {
                    line: line_no,
                    name: name.to_string(),
                    value: value.to_string(),
                    source,
                }),
            },
            
            'l' => match value.trim() {
                "true" | "1" => {
                    last.bools.insert(CompactString::from(name), true);
                }
                "false" | "0" => {
                    last.bools.insert(CompactString::from(name), false);
                }
                other => {
                    errors.push(ParseError::InvalidBool {
                        line: line_no,
                        name: name.to_string(),
                        value: other.to_string(),
                    });
                }
            },

            other => {
                errors.push(ParseError::UnknownType {
                    line: line_no,
                    name: name.to_string(),
                    ty: other.to_string(),
                });
            }
        }
    }

    while stack.len() > 1 {
        let (name, child) = stack.pop().unwrap();
        stack.last_mut().unwrap().1.inners.insert(name, child);
    }

    *cfg = stack.pop().unwrap().1;
    errors
}