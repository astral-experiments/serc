# CPython build-profile diagnostic

Two CPython 3.14.5 builds produce different folded complex NaN bits on the same Linux
x86-64 host. Both report magic number `2b0e0d0a`. Release and platform identity therefore
do not uniquely determine every native marshal byte. The target-build policy is unresolved
in [#576](https://github.com/astral-sh/serc/issues/576).

This diagnostic records native behavior. It does not select a supported build, change
Serc's compiler, normalize NaNs, or relax any existing compatibility comparison.

## Reproduce

Run the diagnostic with each exact oracle interpreter:

```console
/path/to/first/python3.14 -I scripts/diagnose_cpython_build_profile.py > first.json
/path/to/second/python3.14 -I scripts/diagnose_cpython_build_profile.py > second.json
```

The script requires CPython 3.14.5, verifies that each expression is fully folded, and
reports component bits and the constant's complete marshal encoding. It also records the
interpreter's version, compiler, magic number, configuration flags, executable hash, and
the shared library in its prefix when present. The library record identifies that file;
it does not verify arbitrary loader overrides. The recorded probes used isolated mode,
without `PYTHONPATH` or `LD_PRELOAD`.

## Confirmed native results

Fresh probes on September 22, 2026, produced the following IEEE-754 bit patterns, written
in big-endian hexadecimal. All four real components were `fff8000000000000`.

| Expression | GCC build imaginary bits | Clang build imaginary bits |
| --- | --- | --- |
| `(1e309 - 1e309 + 1j) / (1 + 2j)` | `fff8000000000000` | `7ff8000000000000` |
| `(1e309 - 1e309 + 1j) / (2 + 1j)` | `fff8000000000000` | `7ff8000000000000` |
| `(1e309j - 1e309j) / (1 + 2j)` | `fff8000000000000` | `fff8000000000000` |
| `(1e309j - 1e309j) / (2 + 1j)` | `fff8000000000000` | `fff8000000000000` |

For either differing expression, the complete marshalled constant was
`f9000000000000f8ff000000000000f8ff` in the GCC build and
`f9000000000000f8ff000000000000f87f` in the Clang build.

The GCC build is the exact [GitHub Actions release artifact](https://github.com/actions/python-versions/releases/download/3.14.5-25647354415/python-3.14.5-linux-24.04-x64.tar.gz)
downloaded by PR #570's Linux job.
Its release asset ID is `417089678` and its build reports
`3.14.5 (main, May 11 2026, 02:45:53) [GCC 13.3.0]`.

- Archive SHA256: `7248e965b3a3f97a13a68fa516bd43deaeb455236c51781ec0150e3d6f4092fb`.
- Executable SHA256: `d313c3746f1c59d827d53aa3f2e3b693aa77f279e2a6b1f499f057d2b754d729`.
- `libpython3.14.so.1.0` SHA256: `faf65e1b158faacd5a2c9878b04c1ab0fd36dee973648da9b94e2faad51dd102`.

The second build is the installed uv-managed Linux x86-64 GNU interpreter reporting
`3.14.5 (main, Jun  2 2026, 22:25:17) [Clang 22.1.3 ]`.

- Executable SHA256: `c27397da5541f67655be462e128ca3d3eb6e0a8eb0d39036896c9042529f4304`.
- Prefix `libpython3.14.so.1.0` SHA256: `c27f1d51c39073ec767350f19378ceacccb55c2a72af7da79e807a789f94bf88`.

The GCC configuration uses `-O3` and profile-guided optimization. The Clang configuration
uses `-O3`, profile-guided optimization, thin LTO, and BOLT. Inspection of their quotient
machine code found subtraction in the GCC library and negation followed by addition in
the Clang library. The Linux difference is not a fused-multiply-add operation. These
observations describe the two hashed builds; they do not establish a rule for every GCC
or Clang build.

## Policy still required

We need to decide whether to support one concrete reference build per target/platform or
explicitly represent and select multiple native build profiles. A version/OS condition
cannot distinguish these two oracles. Neither NaN normalization nor excluding these cases
would preserve the byte-for-byte contract.

This draft adds diagnostic coverage only. It does not fix #576 or extend existing corpus
results to arbitrary CPython builds. The separate nonfinite-folding issue [#559](https://github.com/astral-sh/serc/issues/559)
and its macOS correction retain their own regressions.
