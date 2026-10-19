# Tessera: Go implementation

A pure Go implementation of the Tessera parser and serializer.
No cgo, no external dependencies. On a synthetic benchmark, roughly 80 MB/s.

Supported platforms: any platform for which the Go toolchain builds.
Only `encoding/base64` and `strconv` are used from the runtime.

> This is not a replacement for the Rust library `tessera`. The public API is in Go style
> (`Parse`, `ToTessera`), and errors are a flat `[]ParseError`, not a detailed
> `ParseError` enum.

## Status

- Language: Go 1.21.
- Completeness: parser for `i`, `f`, `s`, `b`, `l`, nested tables, comments,
  and automatic block closing. Serialization back to Tessera via
  `ToTessera` / `TryToTessera`. JSON is not supported.
- Map-based storage follows the specification: six typed maps on
  `Config` plus `Inners`. Maps are created lazily.

## Running

The benchmarks use Go's standard testing framework.

```bash
go test -bench=. -benchmem
```

Available benchmarks:

- `BenchmarkFlatInts` - 200,000 flat fields of type `int`.
- `BenchmarkMixed` - 100,000 groups of `i` + `s` + `l`.
- `BenchmarkNested` - 20,000 nested blocks with an inner block.
- `BenchmarkEscaped` - 100,000 quoted strings with escape sequences.

Example:

```bash
go test -bench=. -benchmem -benchtime=3x
```

`-benchtime=Nx` runs each benchmark for exactly `N` iterations - convenient
when you need stable MiB/s rather than a time-based average.

## API

```go
cfg, errs := tessera.Parse(src)              // -> *Config, []ParseError

out   := tessera.ToTessera(cfg)              // Config -> Tessera (best effort)
out2, err := tessera.TryToTessera(cfg)       // Config -> Tessera (strict)
```

`ToTessera` silently discards fields that cannot be written on a single line
(empty byte values, keys with spaces, keys starting with `<` or `#`).
If you need to track this, use `TryToTessera`.

## Limitations

- There is no parallel parser: one `Parse` call is one goroutine. Splitting
  a document across multiple goroutines is the caller's responsibility, and
  the caller must determine block boundaries.
- `i` is `int64`; values outside the range produce `InvalidInt` and are
  skipped, as in the Rust reference.
- Strings with escape sequences are copied. Rust has a zero-copy
  path via `unescape_zero_copy`; Go does not.
- Errors contain a line number and `ErrKind`, but not the value itself
  or source context. There is no analog of Rust's `ParseError::trace` here.
- `float64` is printed in the shortest round-trip form
  (`strconv.FormatFloat(..., 'f', -1, 64)`), which matches Rust `Display`
  almost always; subnormals and `-0` may differ in edge cases.

## License

Apache-2.0, like the entire project.

See also [`docs/SPEC.md`](../../docs/SPEC.md) and [`docs/SPEC-ru.md`](../../docs/SPEC-ru.md).