# Explicit initial intern-state profiles

`--initial-intern-state PROFILE.json` supplies one immutable captured intern
state to comparison and mismatch-dump compilation. It describes one process,
including mortal strings, which the model assumes remain retained. A run never
chooses profiles from oracle results or excludes mismatches.

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
extraction/timing reports. The profile supplements the compiler's startup
knowledge and source chronology.

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

## Comparator admission

`--require-all` requires this profile instead of silently using isolated startup
state for the oracle's imported modules. On Python 3.12 it also requires
`--caller-intern-model`, whose validated per-file state includes the current
filename's retained path components. Merely supplying a profile or model path
does not bypass artifact, runtime, or oracle validation.

The explicit `--diagnostic-startup-state` override permits `--require-all` under
startup assumptions and conflicts with profile/model inputs. It preserves all
outcome checks, but does not establish caller-state compatibility.

Comparisons without `--require-all` remain available for diagnostic counts and
mismatch inspection. Their output states whether they use startup assumptions, a
captured process snapshot, a per-file model, or an explicit absence assumption.
Raw byte comparisons and failure categories are unchanged; these labels do not
turn mismatches into matches or establish stronger claims than the supplied
state's contract.
