# Tessera

Tessera is a simple portable language for data storage.  
Its main advantage is simplicity. The entire parser and data structure fit in 200 lines of Rust.  
At the same time, speed reaches 160 MB/s at peak and about 120 MB/s on complex nested structures.

> Detailed language specifics can be found in [`SPEC (en)`](docs/SPEC.md) and [`SPEC (ru)`](docs/SPEC-ru.md)

## Why another format?

Because we want:
- human readability,
- strict typing,
- minimal syntax,
- speed,
- and for the whole core to be small.

Tessera is a compromise: no arrays (except bytes), no multiline notation, but parsing is simple and fast.

## Syntax

Only four constructs:

1. Field: `key type value`
2. Opening a nested table: `<name`
3. Closing a table: `>`
4. Comment: `# ...` (from the start of the line)

Example:

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

Everything before the first `<` goes into the root table.  
Unclosed blocks are automatically closed at the end of the file.

## Types

| Type | Notation | Description |
|-----|-------------|---------|
| `i` | `i64`       | integer |
| `f` | `f64`       | floating-point number |
| `s` | string      | UTF-8, can be quoted or unquoted |
| `b` | bytes       | Base64 |
| `l` | `bool`      | `true`, `false`, `1`, `0` |

## Nesting

Tables can be nested within each other without limits.  
The key under which a nested table is stored in its parent is its name.

## Performance

There is a benchmark in `src/main.rs`.  
On a synthetic file of 100,000 nested blocks (about 1 million lines), it achieves:

- ~160 MB/s on simple data,
- ~120 MB/s on complex nested structures.

At the same time, the format can be optimized almost to the limit of hardware performance, and the reference implementation in Rust is not the performance benchmark.

> In the [C version](impl/c) under development, however, the speed reaches 4.7 GB/s (on the same 10-year-old hardware).

## Usage

The library is called `tessera`.  
Parsing:

```rust
use tessera::{parsing::parse_into, structures::Config};

let mut cfg = Config::default();
let errors = parse_into(&mut cfg, code);
```

Generating Tessera:

```rust
use tessera::generating::to_tessera;

let text = to_tessera(&cfg);
```

Generating JSON:

```rust
use tessera::generating::to_json;

let json = to_json(&cfg);
```

There are also `tessera_to_json` and `from_json`.

--- 

License Apache 2.0, By Pt