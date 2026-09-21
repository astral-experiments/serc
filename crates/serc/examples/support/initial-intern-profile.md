# Explicit initial intern-state profiles

The corpus comparator accepts `--initial-intern-state PROFILE.json`. Without this flag, its compilation, oracle invocation, comparison, and output remain unchanged. The flag supplies one immutable initial string-intern state to both comparison and mismatch-dump compilation.

The profile describes **one captured process**, including mortal strings. It is not a universal bootstrap state or an exclusion list. Mortal entries may vary between fresh processes or disappear during compilation; the library's initial-state model assumes supplied entries remain retained. Every raw marshal mismatch still counts as a failure. A run uses one fixed profile and never selects a profile according to the oracle result.

## Version 1 envelope

The JSON object has these fields; unknown fields are rejected:

- `schema_version`: `1`.
- `target`: `implementation`, dotted `version`, release `tag`, source `commit`, and `magic_hex`, all equal to the selected `PythonVersion::target()`.
- `compiler_input_mode`: `source-bytes` with `--source-bytes`, or `utf8-text` otherwise. The CPython oracle always reads bytes; this field selects the existing Rust compiler input path.
- `oracle`: exact `script_sha256`; `arguments` equal to `["-I", "-S", "-c"]`; `source_input` equal to `bytes`; `compile_mode` equal to `exec`; `dont_inherit` true; `optimize` zero; and `capture_boundary` equal to `Py_CompileStringObject:entry-after-argument-and-AST-init-before-parser` for Python 3.12–3.14 or `_Py_CompileString:entry-after-argument-and-AST-init-before-parser` for Python 3.15.
- Artifact references `executable`, `snapshot`, `capture`, `observer`, `layout`, `headers`, `runtime_inputs`, `process_maps`, and `runtime_capture`. Each reference has an absolute `path` and lowercase hexadecimal `sha256`.

The capture boundary follows normal compile argument handling, lazy AST-type initialization, and source conversion, but precedes target parser and arena allocation. Existing compiler startup knowledge and subsequent source chronology remain active; the supplied profile supplements them.

`snapshot` is the complete numeric-codepoint array of `{pool,state,codepoints}` entries. Static entries have state 3, dynamic entries state 1 or 2. Values must be unique and include the empty string and all Latin-1 singleton strings. The consumer rejects surrogate codepoints rather than dropping them. Counts must agree with the preserved capture metadata.

`capture` preserves the neutral observer's metadata: interpreter and binary hash, native context, exact argv and script hash, source hash, snapshot hash and counts, observer and layout hashes, sole-thread check, identical complete rereads, and function-memory versus executable comparison. The generic extraction source remains available for its content hash; it is unrelated to corpus source selection.

`runtime_inputs` contains `runtime_prefix` and `files`. Each file has an absolute `path`, content `sha256`, byte `size`, and `symlink_target` (null for regular files). It covers every installed prefix file and every captured mapped external native file. The validator rechecks full prefix coverage, contents, and file symlink targets. Directory symlinks are unsupported.

`headers` preserves installed-header hashes and their exact-tag comparison, generated layout hash, and executable hash. `process_maps` preserves the capture's native memory-map text. `runtime_capture` has `before_manifest_sha256`, `after_manifest_sha256`, `capture_sha256`, and `process_maps_sha256`, binding equal full runtime manifests before and after that capture. Preserve the separate timing and extraction reports with these artifacts.

## Scope of validation

Invalid or inconsistent profiles are configuration errors before corpus discovery. Validation does not change the authoritative oracle script, arguments, raw output, error classes, or denominator. A separate interpreter-resolution process verifies the requested executable and installation prefix.

Hashes validate integrity and consistency with declared local inputs; they are not signatures authenticating an artifact's author or independently proving the observer's behavior. Use reviewed extraction tooling and retain its complete evidence. Profile data are generated artifacts, not source-controlled lists of corpus words. This interface does not trace comparator children or claim that a fixed snapshot captures every subsequent mortal-string lifetime.
