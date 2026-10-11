# CPython native build profiles

CPython builds of the same release can produce different folded complex NaN
bits, even with the same platform and magic number. Serc provides explicit
instruction-order profiles for the two CPython 3.14.5 builds identified below.
The default profile is target-dependent. Profile selection requires the
[qualified runtime environment](#runtime-requirements) described below.

## Reproduce

Run the diagnostic with each exact oracle interpreter:

```console
/path/to/first/python3.14 -I scripts/diagnose_cpython_build_profile.py > first.json
/path/to/second/python3.14 -I scripts/diagnose_cpython_build_profile.py > second.json
```

The script requires CPython 3.14.5, verifies that each expression is fully
folded, and reports component bits and the constant's complete marshal encoding.
It also records the interpreter's version, compiler, magic number, configuration
flags, executable hash, and the shared library in its prefix when present. The
library record identifies that file; it does not verify arbitrary loader
overrides. Run without `LD_PRELOAD` or other loader overrides.

## Confirmed native results

For `(1e309 - 1e309 + 1j) / (1 + 2j)`, both builds fold the real component to
`fff8000000000000`. The imaginary component is `fff8000000000000` in the GCC
build and `7ff8000000000000` in the Clang build, expressed as big-endian
IEEE-754 bits. The quotient uses subtraction in the GCC library and negation
followed by addition in the Clang library, not fused multiply-add. These
profiles describe the specific artifacts below, not every build using either
compiler.

## Reference builds

The GCC profile targets this
[GitHub Actions release artifact](https://github.com/actions/python-versions/releases/download/3.14.5-25647354415/python-3.14.5-linux-24.04-x64.tar.gz)
of CPython 3.14.5, built with GCC 13.3.0.

- Archive SHA256:
  `7248e965b3a3f97a13a68fa516bd43deaeb455236c51781ec0150e3d6f4092fb`.
- Executable SHA256:
  `d313c3746f1c59d827d53aa3f2e3b693aa77f279e2a6b1f499f057d2b754d729`.
- `libpython3.14.so.1.0` SHA256:
  `faf65e1b158faacd5a2c9878b04c1ab0fd36dee973648da9b94e2faad51dd102`.

The Clang profile targets the uv-managed Linux x86-64 GNU build of CPython
3.14.5, built with Clang 22.1.3.

- Executable SHA256:
  `c27397da5541f67655be462e128ca3d3eb6e0a8eb0d39036896c9042529f4304`.
- Prefix `libpython3.14.so.1.0` SHA256:
  `c27f1d51c39073ec767350f19378ceacccb55c2a72af7da79e807a789f94bf88`.

## Selecting a profile

`CompileOptions::native_build_profile` and the CLI's `--native-build-profile`
select floating-point instruction ordering before constants are pooled. A
profile is immutable for one compilation, including nested scopes. Selecting
another profile does not change existing compiled modules.

| Selection                                      | Reference artifact                               |
| ---------------------------------------------- | ------------------------------------------------ |
| `default`                                      | Target-dependent profile                         |
| `cpython-3.14.5-actions-gcc-13.3-linux-x86_64` | The Actions GCC 13.3.0 artifact identified above |
| `cpython-3.14.5-uv-clang-22.1.3-linux-x86_64`  | The uv Clang 22.1.3 artifact identified above    |

For the library API, the two explicit selections are
`NativeBuildProfile::Cpython3145ActionsGcc` and
`NativeBuildProfile::Cpython3145UvClang`. These describe specific artifacts, not
generic GCC or Clang modes. Selecting one does not locate, identify, or
authenticate an interpreter. A different Python target or a host other than
GNU/Linux x86-64 is rejected.

No profile normalizes NaNs after compilation or serialization. NaN identity,
payload, constant ordering, and marshal reference handling remain part of the
byte-for-byte comparison.

## Runtime requirements

General powers still call the host math library. The caller must qualify the
selected reference build and Serc in the same compatible math-library,
CPU-dispatch, and floating-point environment. Changing libm, its selected
implementation, or floating-point control settings requires requalification
against the reference interpreter. Profile selection alone does not establish
byte-for-byte identity in an arbitrary environment.

The reference environment used GNU libc 2.39 (`Ubuntu GLIBC 2.39-0ubuntu8.9`).
Both reference interpreters dynamically used the same system `libm.so.6` and
`libc.so.6`; the uv executable contains its Python implementation, whereas the
Actions executable dynamically loads libpython. The libraries used for
qualification have these SHA256 identities:

| Library | SHA256                                                             |
| ------- | ------------------------------------------------------------------ |
| libm    | `fce00b6f25f459cf4ae0b7fae4257909a1a8a86f9149e7c029a5d617baf1ccd0` |
| libc    | `3a15d66867d83762c7f2f1e37359cb8f6c5743edb369c65285cb0b1c4f7498bf` |

## Linux CI reference builds for 3.12 and 3.13

The Linux CI jobs use these concrete Ubuntu 24.04 x86-64 artifacts for CPython
3.12.14 and 3.13.15. The shared setup action verifies each archive's SHA256
before extraction, binds the relocated interpreter to its own shared library,
and checks the exact release and magic number. The jobs run on `ubuntu-24.04`;
the installer also checks the OS and architecture.

| Release artifact                                                                                                                              | Archive SHA256                                                     |
| --------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------ |
| [3.12.14-31661455385](https://github.com/actions/python-versions/releases/download/3.12.14-31661455385/python-3.12.14-linux-24.04-x64.tar.gz) | `5a03168292516f6dd6dcf630f4bf5369b108abd7c699a7e1db37f1e622257fff` |
| [3.13.15-31064747964](https://github.com/actions/python-versions/releases/download/3.13.15-31064747964/python-3.13.15-linux-24.04-x64.tar.gz) | `4e544242f8a4ef647a6f511b67f9b00eefc9ef366644e3c40a27a6eff709ae2b` |

Both artifacts report GCC 13.3.0. For `(1e309 - 1e309) / 1j` and
`(1e309 - 1e309) / (2 + 1j)`, their imaginary NaN bits are `fff8000000000000`.
The corresponding uv 0.12.17 Linux interpreters (python-build-standalone build
20260901, Clang 22.1.3) produce `7ff8000000000000`.

The other CI interpreters come from uv. These pins define the concrete Linux CI
references only: they do not replace independently pinned full-corpus oracles,
establish binary identity with those oracles, or extend their results to other
builds. Named profile selection currently covers the two CPython 3.14.5
artifacts above.
