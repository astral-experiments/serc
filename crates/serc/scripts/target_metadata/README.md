# CPython startup metadata

These fixtures supplement generated static interned strings on Linux for the
[pinned CPython targets](../../README.md#updating-the-cpython-target). Each JSON
file records its exact target, source or capture provenance, hashes, entry
counts, and filtering rules. Other process states can be supplied through
`InitialInternState`.

| Target            | Commit                                     |                Supplemental strings |
| ----------------- | ------------------------------------------ | ----------------------------------: |
| CPython 3.12.14   | `2abcf904b8dac8c999d2b3aac76681abb333798a` |                               2,899 |
| CPython 3.13.15   | `4061bc4c35f7c26f25264666d4ba083b93d2f6f9` |                               2,569 |
| CPython 3.14.5    | `5607950ef232dad16d75c0cf53101d9649d89115` | 2,537 (captured startup supplement) |
| CPython 3.15.0rc2 | `435c9e5a798c99653e3ab64ce29baed0e4f3dfee` |                               2,652 |

## Python 3.14 canonical startup profile

`cpython_3_14_5_startup.json` models the pinned CPython 3.14.5 Linux x86-64
native build with the GIL enabled and the isolated minimal-v2 C reference, with
UTF-8 mode, hash seed 0, and site and signal-handler initialization disabled.
Its executable, installed runtime, headers, C source, environment, and empty
source are bound by SHA-256. This is the declared default bootstrap, not
automatic detection of any host interpreter or a claim about every Linux build.
Different callers supply an explicit `InitialInternState`. The Rust host
architecture does not identify the caller's interpreter; the reference remains
the same across Linux hosts.

The complete capture contains 1,034 static entries, 1,906 immortal dynamic
entries, and 634 mortal dynamic entries. Every mortal entry has an independently
retained owner: the audit follows strong references from interpreter roots and
checks the actual canonical pointer, not merely an equal string. The fixture
records each owner path, the exact layout and audit hashes, and the verified
count. Roots include module and builtin-type dictionaries, AST and generic-type
state when initialized, any suspended caller frames, and the `posix` module's
cached `struct_rusage` name. Weak references, borrowed stack references, and
temporary capture references are not accepted as owners.

The default assumes this bootstrap remains unchanged throughout compilation; it
does not model arbitrary caller mutations, import hooks, or warning callbacks.
Absolute installation filenames are omitted and belong in explicit runtime
profiles. After subtracting generated static strings and cached single
characters, 2,537 strings supplement the default Linux lookup. This covers
native startup families such as `_io`, `posix`, `_codecs`, `_thread`,
`_weakref`, `sys`, and `time`, including builtin and extension type descriptors.
The independently source-derived bootstrap supplement below remains an
additional provenance check.

### Reviewing retained ownership

The offline audit is implemented by
[`audit_startup_owners.py`](../audit_startup_owners.py). Its layout generator,
[`generate_startup_owner_layout.py`](../generate_startup_owner_layout.py),
checks the exact committed CPython declarations and admitted installed headers
before compiling a standalone offset helper. The helper does not link
`libpython`.
[`cpython_3_14_5_startup_ownership.json`](cpython_3_14_5_startup_ownership.json)
records the source declarations, ownership finalizers, layout hash, and
generator hash used for the fixture.

Given the saved artifacts from a verified capture, repeat the offline proof:

```sh
python3 -I -B crates/serc/scripts/generate_startup_owner_layout.py --capture-dir /path/to/capture --cpython /path/to/cpython-3.14.5 --headers /path/to/installed/include/python3.14
python3 -I -B crates/serc/scripts/audit_startup_owners.py --capture-dir /path/to/capture --native-capture /path/to/saved-native-capture
```

The capture directory supplies `header-provenance.json` (installed and exact-tag
header hashes), `layout.json` (intern-table layouts), and `NATIVE-SOURCE.json`
(bindings to the native capture and layout generation). The native capture's
`COMPLETE.json` binds the unchanged reference and runtime, two equal table
reads, one runtime thread, and equal observed/unobserved output.
`canonical.json` records the sorted strings; `addresses.json` records canonical
pointers and intern states. `MEMORY-COMPLETE.json` binds the saved memory
observation, and `memory/REQUEST.json` binds the stopped interpreter address,
loaded images, and `memory/segments.json` with sorted address ranges and
saved-byte hashes. The audit verifies these hashes and the actual source,
header, and binary files, follows only strong object edges, and writes
`retained-owner-audit.json` with a named canonical owner for every mortal entry.
These saved-memory inputs are local audit artifacts and are not needed by
ordinary target generation.

## Source-derived Python 3.14 bootstrap supplement

`cpython_3_14_5_bootstrap.json` covers the complete code trees in
`Lib/importlib/_bootstrap.py`, `Lib/importlib/_bootstrap_external.py`,
`Lib/encodings/__init__.py`, and `Lib/encodings/aliases.py`. These modules
initialize before application code in the ordinary isolated Linux interpreter:
`Python/import.c` initializes both importlib bootstraps, `Python/pylifecycle.c`
initializes the codec registry, and `Python/codecs.c` imports `encodings`, which
imports `aliases`. Every member is also present in the captured minimal-C
startup table; the supplement retains an independent source-derived check and
does not assume caller imports such as `pathlib`.

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

A replacement requires verified captures from two fresh instances of that
target's isolated, default-GIL minimal-v2 C reference compiling empty input.
Follow the fixture's `capture_protocol` and pinned header layouts; read both
intern tables externally without executing diagnostic Python code in the
observed process. Require two complete reads to agree within each process,
identical canonical sets between processes, unchanged executable bytes, and
identical observed and unobserved marshal output. The fixture binds the exact
reference C source; changing it requires recapture. Never derive entries from
corpus strings or another release.

The capture stops at `Py_CompileStringObject`, after the reference creates its
retained filename argument. Decode that `PyUnicode` argument externally using
the pinned layout without assuming it is already interned. Startup optimization
is fixed at zero; the same captured startup profile applies to all three source
compilation optimization levels. Rebuild and qualify the reference on each
worker against the same runtime profile; do not independently regenerate or
combine startup fixtures from different worker environments.

Subtract generated static names and cached single-character strings. Direct C
compilation does not initialize builtin `compile()`'s AST classes; do not add or
subtract those aliases unless they are independently present in the capture.
Exclude installation paths from these sorted, unique supplements;
runtime-specific state can be supplied through `InitialInternState`. Corpus
comparisons use the
[minimal reference](../../../../docs/compatibility.md#comparing-another-corpus).
Update the fixture's pinned digest in
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
