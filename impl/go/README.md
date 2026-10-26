# Tessera: a Go implementation

A pure Go implementation of a Tessera parser, serializer, and decoder into Go structs.
No cgo, no external dependencies. On a synthetic benchmark, around 80 MB/s.

Supported platforms: any platform for which the Go toolchain builds.
Only `encoding/base64`, `strconv`, `reflect`, and `sort` are used from the runtime.

> This is not a replacement for the Rust library `tessera`. The public API is in Go style
> (`Parse`, `ToTessera`, `Decode`), and errors are a flat `[]ParseError`
> or `[]error`, rather than the detailed `ParseError` enum.

## Status

- Language: Go 1.21.
- Completeness:
  - **Parser** for `i`, `f`, `s`, `b`, `l`, nested tables, comments,
    and automatic block closing.
  - **Serialization** back to Tessera via `ToTessera` / `TryToTessera`.
  - *(unique feature)* **Decoder** `Config → Go struct` with tags, the `Unmarshaler` interface,
    and the `ErrorMissing` / `ErrorUnused` / `CaseInsensitive` options.
  - Conversion to JSON is not supported.
- Map-based storage follows the spec: six typed maps on
  `Config` plus `Inners`. Maps are created lazily.

## Running

Benchmarks use Go's standard testing framework.

```bash
go test -bench=. -benchmem
```

Available benchmarks:

- `BenchmarkFlatInts` — 200,000 flat fields of type `int`.
- `BenchmarkMixed` — 100,000 groups of `i` + `s` + `l`.
- `BenchmarkNested` — 20,000 nested blocks with an inner block.
- `BenchmarkEscaped` — 100,000 quoted strings with escape sequences.

Example:

```bash
go test -bench=. -benchmem -benchtime=3x
```

`-benchtime=Nx` runs each benchmark for exactly `N` iterations — convenient
when you need stable MiB/s rather than time averaging.

Decoder tests are run with ordinary `go test .` and do not require benchmarks.

## API

### Parsing and generation

```go
cfg, errs := tessera.Parse(src)              // -> *Config, []ParseError

out   := tessera.ToTessera(cfg)              // Config -> Tessera (best effort)
out2, err := tessera.TryToTessera(cfg)       // Config -> Tessera (strict)
```

`ToTessera` silently discards fields that cannot be written on one line
(empty byte values, keys with spaces, keys starting with `<` or `#`).
If you need to track this, use `TryToTessera`.

### Decoding into a struct

```go
type Tls struct {
    Enabled bool   `tessera:"enabled"`
    Cert    []byte `tessera:"cert"`
}

type Server struct {
    Host  string `tessera:"host"`
    Port  uint16 `tessera:"port"`
    Tls   Tls    `tessera:"tls"`
    Debug *bool  `tessera:"debug,optional"`
}

var s Server
if errs := tessera.Decode(cfg, &s); len(errs) != 0 {
    // errs is a list of errors; decoding continues after each one
}
```

For fine-tuning, use `Decoder`:

```go
d := &tessera.Decoder{
    TagName:         "tessera", // default is "tessera"
    ErrorMissing:    true,      // error if Config has no key for a field
    ErrorUnused:     true,      // error if Config has an unused key
    CaseInsensitive: false,     // ignore case when matching keys
}
errs := d.Decode(cfg, &s)
```

`Decode` is simply `(&Decoder{}).Decode(cfg, dst)` with default settings.
All errors are collected in `[]error`; the decoder does not stop at the first one.

#### Tags

| Tag | Meaning |
|-----|---------|
| `tessera:"key"` | Key name in Config. |
| `tessera:"-"` | Skip the field. |
| `tessera:"key,optional"` | Do not require the key when `ErrorMissing` is set. |

If the tag is absent, the Go field name is used as is
(`HTTPPort` → `"HTTPPort"`, not `"httpport"`). Tessera is case-sensitive,
so an explicit tag is almost always needed for exported fields.
The alternative is `CaseInsensitive: true`.

#### Supported types

| Go type | Source in Config | Notes |
|---|---|---|
| `int8..int64`, `uint8..uint64` | `Ints` | Overflow check via `reflect.Overflow*`. |
| `float32`, `float64` | `Floats` | Range is checked for `float32`. |
| `string` | `Strs` | String copy. |
| `bool` | `Logicals` | |
| `[]byte` | `Bytes` | Slice copy. |
| `struct` | `Inners` | Recursively, field by field. |
| `*T` | like `T` | `nil` → `reflect.New`, then recursion. |
| No `Option` analog; use `*T` | | `nil` = key absent. |

#### Custom types: `Unmarshaler`

```go
type Duration time.Duration

func (d *Duration) UnmarshalTessera(cfg *tessera.Config, key string) error {
    n, ok := cfg.Ints[key]
    if !ok {
        return fmt.Errorf("want i for %q", key)
    }
    *d = Duration(time.Duration(n) * time.Millisecond)
    return nil
}
```

`Unmarshaler` is checked on the pointer `*T` and on the addressable value `T`.
If the field is a pointer, the decoder allocates it before calling the method.

#### What is NOT supported

Per the Tessera spec:

- `[]T` for any `T` except `byte`;
- `[N]T`;
- `interface{}`.

Such fields produce an error but do not interrupt decoding of the remaining fields.

#### One key — multiple types

Tessera allows one key to exist simultaneously in different typed
maps. The decoder routes values by the Go field type:

```go
type Thing struct {
    X int64  `tessera:"x"` // from cfg.Ints["x"]
    Y string `tessera:"x"` // from cfg.Strs["x"]
}
```

This works as long as the fields have different Go types corresponding to different typed maps.

## Limitations

- **No parallel parser.** One `Parse` call is one goroutine.
  Splitting a document across multiple goroutines is the caller's job;
  the caller must determine block boundaries. (The C version supports
  multithreading; Go does not.)
- **`i` = `int64`.** Out-of-range values yield `InvalidInt` and
  are skipped, as in the Rust reference.
- **Strings with escapes are copied.** Rust has a zero-copy path via
  `unescape_zero_copy`; Go does not.
- **Parser errors are flat.** They contain a line number and `ErrKind`, but not
  the value itself or source context. There is no analog of `ParseError::trace`
  from Rust here.
- **`float64` is printed in shortest round-trip form**
  (`strconv.FormatFloat(..., 'f', -1, 64)`), which matches Rust `Display`
  almost always; subnormals and `-0` may differ in edge cases.
- **The decoder does not distinguish** “key absent” and “zero value” — both
  give `X == 0` for `int64`. To distinguish them, use `*int64`.
  The exception is `ErrorMissing: true`: then a missing key produces an error,
  while a value of 0 passes.
- **`Decoder` is safe for concurrent use** provided its fields are not mutated
  after the first call. Do not change
  `ErrorMissing` / `ErrorUnused` / `CaseInsensitive` between `Decode` calls
  on the same copy.
- **Encoder (`struct` → `Config`) is not implemented.** For round-trip,
  use `ToTessera` directly on `*Config`, or manually populate
  `Config` from a struct.

## License

Apache-2.0, like the whole project.

See also [`docs/SPEC.md`](../../docs/SPEC.md) and [`docs/SPEC-ru.md`](../../docs/SPEC-ru.md).