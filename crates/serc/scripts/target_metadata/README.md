# CPython startup metadata

These fixtures supplement generated static and compiler-initialized interned
strings on Linux for the
[pinned CPython targets](../../README.md#updating-the-cpython-target). Each JSON
file records its exact target, source or capture provenance, hashes, entry
counts, and filtering rules. Other process states can be supplied through
`InitialInternState`.

| Target            | Commit                                     |                        Supplemental strings |
| ----------------- | ------------------------------------------ | ------------------------------------------: |
| CPython 3.12.14   | `2abcf904b8dac8c999d2b3aac76681abb333798a` |                                       2,956 |
| CPython 3.13.15   | `4061bc4c35f7c26f25264666d4ba083b93d2f6f9` |                                       2,648 |
| CPython 3.14.5    | `5607950ef232dad16d75c0cf53101d9649d89115` | 1,045 (source-derived bootstrap supplement) |
| CPython 3.15.0rc2 | `435c9e5a798c99653e3ab64ce29baed0e4f3dfee` |                                       2,790 |

## Source-derived Python 3.14 bootstrap supplement

`cpython_3_14_5_bootstrap.json` covers the complete code trees in
`Lib/importlib/_bootstrap.py`, `Lib/importlib/_bootstrap_external.py`,
`Lib/encodings/__init__.py`, and `Lib/encodings/aliases.py`. These modules
initialize before application code in the ordinary isolated Linux interpreter:
`Python/import.c` initializes both importlib bootstraps, `Python/pylifecycle.c`
initializes the codec registry, and `Python/codecs.c` imports `encodings`, which
imports `aliases`. The supplement augments the existing static,
compiler-initialized, and historical startup names; it does not replace them or
assume caller imports such as `pathlib`.

The generator reads only files from the exact release commit and compiles the
four module sources with the pinned CPython 3.14.5 interpreter,
`dont_inherit=True`, and `optimize=0`. It never executes their code or reads
corpus inputs. `Programs/_freeze_module.c` uses the same optimization level for
frozen importlib. The fixture records every code object's relevant strings and
the qualified names and source lines of retained function owners, together with
the source and generator SHA-256 digests.

The lifetime rule follows `Objects/codeobject.c`:

- `_PyCode_New` interns every `co_names` and locals-plus identifier immortally.
  Include these names recursively even for a platform branch that Linux never
  executes: unmarshalling constructs its code objects too.
- `init_code` interns code names, qualified names, and filenames mortally;
  `intern_constants` also interns ASCII identifier constants mortally. A
  module-owned function or class method retains its code object, which retains
  all nested code objects and their constant containers. Follow that entire
  ownership tree, including nested classes. Include stable frozen filenames, but
  omit installation-dependent filenames.
- Classes retain their names, qualified names, methods, and literal attributes.
  Module assignments retain complete literal containers, including every key and
  value of the encoding alias dictionary. Later assignments release the earlier
  literal owner. The pinned alias lookup only adds previously absent aliases, so
  initial alias entries remain live.

The AST walk selects the pinned Linux branches and rejects unknown conditions,
decorators, deletion, or replacement of function owners. `classmethod`,
`staticmethod`, and properties retain their functions. The pinned
`_requires_builtin`, `_requires_frozen`, and `_check_name` decorators return
wrappers whose closures retain the original function; property setters retain
both accessors. Source digests bind these audited retention assumptions. This is
a source-derived supplement for these bootstrap families, not a complete runtime
intern-table snapshot.

Regenerate and verify the fixture using the exact target interpreter before
regenerating target metadata:

```sh
/path/to/python3.14.5 -I -S -B crates/serc/scripts/generate_cpython_bootstrap.py --cpython /path/to/cpython-3.14.5
/path/to/python3.14.5 -I -S -B crates/serc/scripts/generate_cpython_bootstrap.py --cpython /path/to/cpython-3.14.5 --check
```

Update the fixture's pinned SHA-256 in `generate_cpython_target.py` when
regenerating it. Ordinary target generation validates the fixture, helper
digest, and all pinned source digests without requiring the host Python to
compile 3.14 source. The generated lookup applies this new supplement on Linux;
an explicit `InitialInternState` continues to represent a different caller's
state.

## Captured startup supplements

A replacement requires a verified capture from that target's isolated,
default-GIL interpreter compiling empty input. Follow the fixture's
`capture_protocol` and pinned header layouts; read both intern tables externally
without executing diagnostic Python code in the observed process. Require two
complete reads to agree, unchanged executable bytes, and identical observed and
unobserved marshal output. Never derive startup entries from corpus strings or
another release.

Subtract generated static/AST names and cached single-character strings. Keep
installation paths in the
[explicit runtime profile](../../examples/support/initial-intern-profile.md),
not in these sorted, unique supplements. Update the fixture's pinned digest in
[`generate_cpython_target.py`](../generate_cpython_target.py), then
[regenerate and check the target](../../README.md#updating-the-cpython-target).

Source-codec intern strings are generated separately for each supported target.
Byte-source decoding may load a standard codec before compilation; the generated
sets describe names made immortal by its code objects and strings retained by
its codec module, plus the CJK mapping exports. Generation reads only files from
the exact pinned CPython commit and runs with that same CPython release. The CJK
supplement also derives retained native type descriptor names from the
referenced method, member, and getset tables and their clinic definitions. For
example, verify the 3.14 target with:

```sh
/path/to/python3.14.5 -I -S -B crates/serc/scripts/generate_source_codec_interns.py --target-version 3.14 --cpython /path/to/cpython-3.14.5 --check
```

Use `--target-version 3.12`, `3.13`, or `3.15` with the corresponding pinned
interpreter and checkout for the other tables. Omit `--check` to regenerate. The
compiler separately models successful codec-registry lookup names and the
built-in decoder shortcuts; custom registries can supply their own post-decode
intern state using `without_standard_source_codec_effects`.
