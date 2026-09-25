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

## Profile format

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
