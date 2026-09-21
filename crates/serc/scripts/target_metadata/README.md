# CPython startup metadata

The target generator reads opcode, operand, cache, stack and static string metadata from the exact pinned CPython commit. It separately verifies the SHA-256 of the startup fixture in this directory. Existing Python 3.14/3.15 instruction identities and physical encodings remain unchanged; legacy instructions have separate semantic identities and generated classification metadata.

| Target | Commit | Supplemental strings |
| --- | --- | ---: |
| CPython 3.12.14 | `2abcf904b8dac8c999d2b3aac76681abb333798a` | 2,956 |
| CPython 3.13.15 | `4061bc4c35f7c26f25264666d4ba083b93d2f6f9` | 2,648 |
| CPython 3.15.0rc2 | `435c9e5a798c99653e3ab64ce29baed0e4f3dfee` | 2,790 |

`cpython_3_15_0rc2_startup.json` records the additional canonical strings present in an isolated Linux CPython 3.15.0rc2 process immediately before the compatibility oracle compiles an empty source file. These entries supplement the generated static and compiler-initialized names. The lookup applies this captured supplement on Linux; other bootstrap states can be supplied through `InitialInternState`.

The fixture contains the exact release commit, oracle script, script digest, empty-input digest, original canonical snapshot digest, entry counts and capture procedure. To reproduce the capture, run the pinned default-GIL interpreter with `-I -S -c <oracle_script> <filename>` and empty standard input. Stop immediately upon entry to `_Py_CompileString` using an external hardware execution breakpoint. Read both intern tables from the stopped process using the layouts from the pinned CPython headers. Do not call Python diagnostic functions or evaluate additional source in the process. Require two complete canonical reads to agree, executable bytes to remain unchanged, and observed and unobserved runs to emit identical marshal bytes.

The 3.12 and 3.13 fixtures follow the same independently verified procedure, stopping at their `Py_CompileStringObject` entry. Each fixture is derived from that version's clean startup snapshot and exact source-derived static/AST names. The 3.12 capture contained no absolute installation filenames; the 3.13 supplement excludes four observed filenames. The full runtime profiles retain those process-specific entries. Neither fixture uses corpus strings or another release's startup state.

For the checked-in supplement, remove names already generated from `pycore_global_strings.h` or `Python-ast.c`, single-character cached strings, and the three absolute filenames under the interpreter installation's `encodings` package. Keep installation-specific strings in an explicit runtime profile. The resulting sorted, unique list contains 2,790 entries and contains no installation paths. Changing the fixture requires a new verified capture and updating its pinned digest in `generate_cpython_target.py`.

Python 3.12 opcode selection comes from `Lib/opcode.py`, with stack metadata in `Python/opcode_metadata.h`; Python 3.13 uses `Lib/_opcode_metadata.py` and `pycore_opcode_metadata.h`. The generator cross-checks the selected ordinary instructions against the generated C opcode IDs and rejects incomplete stack/property metadata. It does not infer ordinary instructions from a numeric cutoff, which would include specialized instructions occupying low-numbered holes. Both releases share marshal format 4; `v4.rs` records both pinned source digests and must regenerate identically from either checkout.

Verify all target versions with:

```sh
python3 crates/serc/scripts/generate_cpython_target.py --cpython /path/to/cpython-3.12.14 --target-version 3.12 --check
python3 crates/serc/scripts/generate_cpython_target.py --cpython /path/to/cpython-3.13.15 --target-version 3.13 --check
python3 crates/serc/scripts/generate_cpython_target.py --cpython /path/to/cpython-3.14.5 --check
python3 crates/serc/scripts/generate_cpython_target.py --cpython /path/to/cpython-3.15.0rc2 --target-version 3.15 --check
```
