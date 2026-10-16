# Tessera

Tessera is a simple portable data storage language. And its simplicity is its main advantage. The canonical implementation of the language is written in Rust; dependencies can be viewed in the project's `Cargo.toml`. The entire core — the structure and the parser into this structure — fits in 200 lines. At the same time, the speed is currently around 160 MB/s at peak, and about 120 MB/s on complex tasks (1M+ lines of nested structures, for example). It was created for maximum simplicity of working with data while remaining human-readable and allowing optimization. Let's break down the concepts and syntax of the language.

## Semantics and idea

The language was developed primarily, as mentioned, for maximum simplicity and at the same time speed. Therefore, the following trade-offs were used:

- Everything is a table: The language intentionally has no arrays (if you don't count bytes). This is done to simplify parsing and because it is unnecessary within the concept.
- Values are strictly typed (for parsing speed, obviously).
- The syntax is line-based and does not support writing on one line (for simplicity of parsing and because it is unnecessary).

## Data model

A table is an unordered set of named values grouped by type. One key can simultaneously reside in different type tables. If a key repeats in one table, the last value wins.

| Table | Value type |
| :- | :- |
| ints   | `i64` |
| strs   | UTF-8 string |
| floats | `f64` |
| bytes  | bytes, `Vec<u8>` |
| logicals  | `bool` |
| inners | nested table |

The root table is implicit and always exists. Nesting is unlimited. Key order is not preserved.

## Syntax

The syntax of the language is minimal and very simple, which allows all processing logic to fit in 150 lines of Rust code. Everything that can be written in a file is four constructs.

**Constructs:**

| Syntax | What it does | Nuances | Why this way |
| :- | :- | :- | :- |
| `key type value` | Field of the current table. | `key` and `type` cannot contain spaces. Type is only one character. `value` is everything after the second space, so it can contain spaces. Separator is exactly one space `' '`. Duplicates are overwritten. | The basic unit of data. |
| `<name` | Opens a nested table `name`. | `name` is the remainder of the line after `<`, trimmed at the edges. It can contain spaces. Empty name — `MissingName`. | Nesting without braces and indentation. |
| `>` | Closes the current nested table. | The line must be exactly `>` after trim. If there is nothing to close — `CantPopStack`. | Symmetric to `<`. |
| `# ...` | Comment to end of line. | Only if `#` is the first non-whitespace character. No inline comments. | Simplicity. |
| empty line | Ignored. | After trim. | Formatting convenience. |
| EOF | Automatically closes all open tables. | Not an error. | Simplifies the parser and manual writing. |

## Types

| Type | Rust type | How it is parsed | Nuances |
| :- | :- | :- | :- |
| `i` | `i64` | `value.parse::<i64>()` | Decimal number. Leading `+`/`-` is allowed. Spaces inside the value are not allowed. Error — `InvalidInt`. |
| `f` | `f64` | `value.parse::<f64>()` | `1.0`, `-2.5`, `1e10`, `2.5E-3`. Spaces inside the value are not allowed. Error — `InvalidFloat`. |
| `s` | `CompactString` | outer `"` are removed, then `unescape_default` | If there are no quotes, the value is taken as is, but escape is still processed. Error — `BadEscape`. |
| `b` | `Vec<u8>` | `base64::engine::general_purpose::STANDARD.decode` | Standard Base64 with padding `=`. Spaces inside the value are not allowed. Error — `BadBase64`. |
| `l` | `bool` | comparison with string | `true` / `false` / `1` / `0`. The value is trimmed at the edges. Other values — `InvalidBool`. |

### Escape in `f`

| Escape | Value |
| :- | :- |
| `\\` | backslash |
| `\"` | double quote |
| `\n` | newline |
| `\r` | carriage return |
| `\t` | tab |
| `\uXXXX` | Unicode code point, 4 hex digits |

## Nesting

Opening a table:

```tessera
<name
```

`name` is the remainder of the line after `<`, trimmed at the edges. It can contain spaces.

Closing a table:

```tessera
>
```

The line must be exactly `>` after trim.

Rules:

- everything before the first `<` goes into the root table;
- the key under which the nested table is placed into the parent is `name`;
- if a table with such a key already exists in the parent's `inners`, it is overwritten;
- if blocks are not closed by the end of the file, they are closed automatically at EOF. This is not an error.

## Parsing

The parser is line-based. It uses `code.lines()`. Each line is trimmed via `trim_ascii()`.

Algorithm:

1. Empty line — skip.
2. Line starts with `<` — open block:
   - name = remainder after `<`, trimmed at the edges;
   - if name is empty — `MissingName`;
   - otherwise a new `Config` is pushed onto the stack.
3. Line equals `>` — close block:
   - if the nesting stack is empty (only the root remains) — `CantPopStack`;
   - otherwise the top `Config` is popped from the stack and inserted into the parent's `inners`.
4. Line starts with `#` — comment, skip.
5. Otherwise the line is split with `splitn(3, ' ')`:
   - if there are not three parts — `MissingParts`;
   - `name`, `vtype`, `value`.
6. By `vtype`:
   - `i` -> `parse::<i64>()`;
   - `s` -> remove outer `"`, then `unescape_default`;
   - `f` -> `parse::<f64>()`;
   - `b` -> Base64 decode;
   - `l` -> `true` / `false` / `1` / `0`;
   - otherwise -> `UnknownType`.
7. At the end of the file, all unclosed blocks are automatically closed:
   - while stack > 1, the top `Config` is inserted into the parent's `inners`;
   - the root `Config` is written into `cfg`.

An erroneous field is not added to `Config`.  
An erroneous block is not opened.  
An erroneous close is ignored.

## Errors

The parser does not fail on the first error. It collects a list of errors and continues parsing.

| Error | When it occurs |
| :- | :- |
| `MissingName` | `<` without a name. |
| `CantPopStack` | `>` with an empty nesting stack. |
| `MissingParts` | Line does not match `key type value`. |
| `InvalidInt` | Value does not parse as `i64`. |
| `InvalidFloat` | Value does not parse as `f64`. |
| `InvalidBool` | Value is not `true`, `false`, `1`, or `0`. |
| `BadEscape` | Escape error in `s`. |
| `BadBase64` | Base64 error in `b`. |
| `UnknownType` | Unknown type. |

## Brief grammar

```ebnf
document    = { line } ;
line        = ws, (comment | block_open | block_close | field | empty), ws ;
block_open  = "<", ws, name, ws ;
block_close = ">" ;
field       = key, " ", type, " ", value ;
type        = "i" | "f" | "s" | "b" | "l" ;
comment     = "#", { any } ;
```

Where:

- `key` — non-empty string without spaces;
- `value` — remainder of the line after the second space;
- `name` — remainder after `<`, trimmed at the edges;
- `empty` — empty line or a line consisting only of whitespace characters.

## Example of a complete document

```tessera
# Root fields
version i 1
title s "Example"
debug l true

<server
  host s "example.com"
  port i 8080
  <tls
    enabled l true
    cert b TUlJQ...
  >
>

<users
  <alice
    age i 30
    roles s "admin,user"
  >
>
```

At EOF all open blocks will be closed automatically.