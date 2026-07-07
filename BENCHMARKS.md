# Benchmarks

These benchmarks were recorded on July 7, 2026, on Linux with 32 AMD EPYC 9V74 vCPUs and the
following versions:

- CPython 3.14.5
- Serc [`bd51c6d`](https://github.com/astral-sh/serc/commit/bd51c6d821094034750c084ac41f50d3e3ebfe2b)
- Hyperfine 1.20.0
- Rust 1.96.0
- NumPy [`v2.4.6`](https://github.com/numpy/numpy/tree/b832a09cf2a169c833dd2371e7c07aa00b293242)

Benchmark performance varies across operating systems, machines, and workloads. This benchmark
compares the complete sequential command-line workflow on NumPy's 505 Python files (9,443,546
bytes). Each measured command starts a fresh process, discovers and reads the sources, compiles
them at optimization level 0, marshals them, and writes timestamp-invalidated `.pyc` files.
Hyperfine removes generated `__pycache__` directories before each command, outside the timed
interval.

For setup, validation, and reproduction instructions, see the
[end-to-end benchmark harness](scripts/benchmarks/README.md).

## NumPy

```text
Benchmark 1: CPython 3.14.5 compileall
  Time (mean ± σ):      2.056 s ±  0.451 s    [User: 1.819 s, System: 0.207 s]
  Range (min … max):    1.729 s …  3.019 s    10 runs

Benchmark 2: Serc bd51c6d

  Warning: The first benchmarking run for this command was significantly slower than the rest (1.672 s). This could be caused by (filesystem) caches that were not filled until after the first run. You are already using both the '--warmup' option as well as the '--prepare' option. Consider re-running the benchmark on a quiet system. Maybe it was a random outlier. Alternatively, consider increasing the warmup count.
  Time (mean ± σ):      1.471 s ±  0.079 s    [User: 1.323 s, System: 0.138 s]
  Range (min … max):    1.413 s …  1.672 s    10 runs

Summary
  Serc bd51c6d ran
    1.40 ± 0.32 times faster than CPython 3.14.5 compileall
```

The runner preflighted both commands and verified that each compiled all 505 files and produced
505 cache files. This workload is a performance comparison, not the project's byte-for-byte
compatibility oracle.
