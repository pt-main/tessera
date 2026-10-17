# Tessera C implementation

Experimental standalone implementation of the Tessera parser in C.  
Focused on speed (peak 4.7 GB/s on Intel(R) Core(TM) i7-4770HQ (8) @ 3.40 GHz): mmap, multithreading, AVX2, arena allocator, compact 24-byte records. The code is suitable to simply copy and use.

Supported platforms: Linux, macOS, FreeBSD/NetBSD/OpenBSD/DragonFly, Windows (x64). Single source file, no external dependencies. On POSIX, performance is identical to the original version: all hot macros expand to the same `__builtin_*`.

> This is not a replacement for the Rust library `tessera`. There is no public API, no Tessera/JSON generation, and no detailed error diagnostics.

## Status

- Language: C11.
- Platforms: Linux, macOS, BSD, Windows (x64).
- Completeness: the main Tessera constructs are supported (`i`, `f`, `s`, `b`, `l`, nested tables, comments, auto-closing of blocks within a chunk). Error details and full merging of all nested chunks may differ from the Rust implementation.

## Build

### Linux / macOS / BSD (GCC or Clang)

```bash
cc -O3 -march=haswell -mtune=haswell -flto -funroll-loops \
   -fomit-frame-pointer -fno-plt -pthread -o tess tessera.c
```

If you have a different CPU, replace `-march=haswell` with `-march=native` or remove the AVX2 flags:

```bash
cc -O3 -march=native -flto -funroll-loops \
   -fomit-frame-pointer -fno-plt -pthread -o tess tessera.c
```

On ARM (including Apple Silicon), AVX2 is unavailable, and the parser automatically uses the scalar path. It is enough to remove `-march=haswell`:

```bash
cc -O3 -flto -funroll-loops -fomit-frame-pointer -pthread -o tess tessera.c
```

### Windows

MinGW-w64 (x64):

```bash
cc -O3 -march=haswell -mtune=haswell -flto -funroll-loops \
   -fomit-frame-pointer -o tess.exe tessera.c
```

MSVC (requires VS 2022 17.3+ for C11 atomics):

```cmd
cl /O2 /arch:AVX2 /fp:precise /std:c17 /GL tessera.c
```

## Running

Basically, the compiled binary runs a benchmark on a file.

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

Output goes to `stderr`: file size, number of threads, platform, AVX2 availability, pre-scan time, time and MiB/s per iteration, best result.

## Build options

- `-DTESSERA_MERGE_MASTER=0` — do not build a shared master array for root records.

## Platforms

| Subsystem     | POSIX                                      | Windows                                              |
|---------------|--------------------------------------------|------------------------------------------------------|
| File mapping  | mmap/madvise                               | CreateFileMapping/MapViewOfFile                      |
| Threading     | pthread                                    | SRWLOCK + CONDITION_VARIABLE + CreateThread          |
| Barrier       | pthread_mutex/pthread_cond                 | SRWLOCK + CONDITION_VARIABLE                        |
| Timer         | clock_gettime(CLOCK_MONOTONIC)             | QueryPerformanceCounter                              |
| CPU count     | /proc/cpuinfo or sysctl                    | GetLogicalProcessorInformationEx                    |
| Aligned alloc | posix_memalign + MADV_HUGEPAGE             | _aligned_malloc                                      |

## Limitations

- No serialization to Tessera and JSON, no `from_json`.
- No detailed error list with line numbers, as in the Rust version.
- The parallel parser is designed for performance; for complex nested structures, verify correctness on your own data.
- `-march=haswell` requires AVX2; on older CPUs or ARM, use `-march=native` or remove the flag, and the parser will automatically switch to the scalar path.
- On Windows, large pages are not used (requires SeLockMemoryPrivilege).

## License

Apache-2.0, like the whole project.

See also [`docs/SPEC.md`](../../docs/SPEC.md) and [`docs/SPEC-ru.md`](../../docs/SPEC-ru.md).