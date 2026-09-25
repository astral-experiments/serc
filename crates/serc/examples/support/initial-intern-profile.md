# Explicit initial intern-state profiles

The comparator requires `--initial-intern-state PROFILE.json` with provenance
matching its actual oracle. A captured snapshot authenticates one process's
membership. Its static pool supplies guaranteed retained entries; dynamic
observations remain unknown unless a separate caller contract proves membership
and lifetime across inputs. Dynamic immortality within the captured process does
not establish membership in every fresh invocation.

Byte-relevant unknown membership is reported as `unresolved-intern-state` and
fails compatibility validation. It is neither a compiler failure nor a matched
syntax rejection. A run never chooses profiles from comparison results or
excludes mismatches.

`--diagnostic-assume-startup-intern-state` permits exploratory comparisons using
the neutral startup model. Output explicitly identifies this unverified
assumption, and the option cannot be combined with compatibility-gate counts,
`--require-all`, rejection-parity validation, a caller profile/model, or native
cache operations.

## Captured profile format

The [reader](initial_intern_profile.rs) defines the version 1 envelope and
rejects unknown fields:

- `schema_version`: `1`.
- `target`: `implementation`, `version`, `tag`, `commit`, and `magic_hex`
  matching `PythonVersion::target()`.
- `compiler_input_mode`: `source-bytes` with `--source-bytes`, otherwise
  `utf8-text`. The oracle always reads bytes.
- `oracle`: the exact script hash, `arguments` of `["-I", "-S", "-c"]`, byte
  input, `exec` mode, `dont_inherit: true`, `optimize: 0`, and the target's
  compile-entry capture boundary after argument and AST initialization, before
  parsing. This is `Py_CompileStringObject` for 3.12–3.14 and
  `_Py_CompileString` for 3.15.
- `executable`, `snapshot`, `capture`, `observer`, `layout`, `headers`,
  `runtime_inputs`, `process_maps`, and `runtime_capture`: artifact references
  with absolute `path` and lowercase `sha256`.

The snapshot is a unique array of `{pool,state,codepoints}` entries: state 3 for
static strings, 1 or 2 for dynamic strings. It includes the empty string and all
Latin-1 singletons, rejects surrogate codepoints, and agrees with the capture's
counts.

Capture evidence binds the executable and exact-tag headers, observer/layout,
oracle arguments and script, source, and snapshot. Preserve the sole-thread
check, two agreeing complete reads, function-memory comparison, and
extraction/timing reports. Captured entries do not implicitly supplement the
compiler's neutral startup model. The effective state is partial, and
compilation preserves source chronology.

`runtime_inputs` covers every installed prefix file and captured mapped native
file, recording absolute path, SHA-256, byte size, and symlink target. Directory
symlinks are unsupported. The validator rechecks full coverage and contents;
`runtime_capture` binds equal before/after runtime manifests to the capture and
process maps.

Invalid profiles fail before corpus discovery. Oracle identity, raw-output
comparison, outcome categories, and the denominator remain mandatory. Hashes
establish consistency, not authorship or proof of observer correctness; retain
reviewed extraction tooling and its evidence. A fixed profile does not establish
initial absence for unknown entries or trace subsequent mortal-string lifetimes.
Profiles are generated artifacts, not source-controlled corpus word lists.

## Standard caller source proofs

A `standard-caller-source-proof` is a separate version 1 document. The
[source-proof reader](standard_caller_profile.rs) requires an independently
reviewed artifact hash and state semantics, the exact target and unchanged
oracle, proved retained-entry ownership, and a matching runtime admission. A
document cannot grant itself admission by declaring its proof complete.

A `guaranteed-retained-partial` proof supplies only guaranteed retained strings;
every unproved entry remains unknown. A separately admitted
`retained-with-absence` proof also establishes a complete possible-ASCII-word
closure and can prove that words outside it are absent. Neither the possible
word inventory nor an incomplete closure can supply absence facts in partial
mode. Byte-relevant unknown membership always fails validation. Python 3.12 and
3.13 add the current filename's retained POSIX components independently for each
input; Python 3.14 and 3.15 do not apply that earlier pathlib behavior. All
standard profiles require filenames whose spelling is already canonical for the
unchanged pathlib oracle.

Runtime qualification occurs once per run. The preparer verifies the exact
interpreter, library, and standard-library files, then creates an owned
read-only copy without bytecode caches or site packages. It verifies the
expected frozen, source, and native module selections. The oracle uses that copy
under the admitted complete environment, preserving its script, arguments,
compile options, warnings, and raw-byte comparison. The original interpreter
installation is unchanged, and the temporary copy is removed when the run ends.

These profiles describe the qualified runtime and environment, not arbitrary
inherited process settings. They cannot be combined with legacy caller models or
native output caches. Existing captured-profile and cache invocations retain
their original runtime behavior.

The ordinary compatibility scripts select the target's reviewed standard
profile. `check-cpython-compatibility` accepts an explicit profile as its third
argument; the mypy_primer and PGO scripts accept `--initial-intern-state`. Their
outcome counts and rejection checks remain mandatory, including zero unresolved
intern states. Unsupported runtime builds fail qualification before any files
are compared.

Source-only target profiles bind a mandatory
`source-derived-caller-initial-state` artifact through the nonempty
`startup_profile_sha256`. The independent admission identifies this provenance
kind separately from captured native-owner evidence. The artifact must agree
with the outer profile's target, generator, source hashes, runtime admissions,
and exact retained strings and runtime-relative paths. Its generator interpreter
hash must match an admitted runtime executable. Missing artifacts, changed
positive arrays, or mixed provenance kinds reject admission. This artifact
proves only retained positives; every unlisted value remains unknown.
