# Tessera C implementation

An experimental standalone implementation of the Tessera parser in C.  
Focused on speed (peak - 4.7 GB/s on Intel(R) Core(TM) i7-4770HQ (8) @ 3.40 GHz): `mmap`, multithreading, AVX2, arena allocator, compact 24-byte records. The code is suitable to simply copy and use.

> This is not a replacement for the Rust library `tessera`. There is no public API, Tessera/JSON generation, or detailed error diagnostics here.

## Status

- Language: C11/POSIX.
- Platform: Linux/POSIX (`mmap`, `pthreads`, `madvise`, `/proc/cpuinfo`).
- Completeness: the main Tessera constructs are supported (`i`, `f`, `s`, `b`, `l`, nested tables, comments, auto-closing of blocks within a chunk). Error details and full merging of all nested chunks may differ from the Rust implementation.

## Build

```bash
cd impl/c
./compile.sh
```

`compile.sh` uses `-march=haswell`. If you have a different CPU, replace it with `-march=native` or remove the AVX2 flags:

```bash
cc -O3 -march=native -flto -funroll-loops \
   -fomit-frame-pointer -fno-plt -pthread -o tess tessera.c
```

## Running

The compiled binary runs a benchmark on a file by default.

```bash
./tess <file> [iterations] [threads]
```

- `<file>` — a file in Tessera format.
- `[iterations]` — number of runs, default `3`.
- `[threads]` — number of threads; by default determined by physical cores.

Example:

```bash
./tess bench.tess 10 8
```

Output goes to `stderr`: file size, number of threads, AVX2 availability, pre-scan time, time and MiB/s per iteration, best result.

## Build options

- `-DTESSERA_MERGE_MASTER=0` — do not build a shared master array for root records.

## Limitations

- No serialization to Tessera or JSON, no `from_json`.
- No detailed error list with line numbers, as in the Rust version.
- The parallel parser is designed for performance; for complex nested structures, verify correctness on your own data.
- The code uses POSIX calls and does not yet build on Windows without porting.
- `-march=haswell` requires AVX2; on older CPUs there may be startup problems.

## Performance

On the same machine, the C version can reach several GB/s on suitable data; the main README states up to `2.5 GB/s`. The result depends heavily on CPU (AVX2), number of threads, file size, and data structure.

## License

Apache-2.0, like the whole project.

See also [`docs/SPEC.md`](../../docs/SPEC.md) and [`docs/SPEC-ru.md`](../../docs/SPEC-ru.md).