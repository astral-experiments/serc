# CPython startup metadata

The target generator reads opcode, operand, cache, stack and static string metadata from the exact pinned CPython commit. It separately verifies the SHA-256 of the startup fixture in this directory. The Python 3.14.5 output remains unchanged.

`cpython_3_15_0rc2_startup.json` records the additional canonical strings present in an isolated Linux CPython 3.15.0rc2 process immediately before the compatibility oracle compiles an empty source file. These entries supplement the generated static and compiler-initialized names. The lookup applies this captured supplement on Linux; other bootstrap states can be supplied through `InitialInternState`.

The fixture contains the exact release commit, oracle script, script digest, empty-input digest, original canonical snapshot digest, entry counts and capture procedure. To reproduce the capture, run the pinned default-GIL interpreter with `-I -S -c <oracle_script> <filename>` and empty standard input. Stop immediately upon entry to `_Py_CompileString` using an external hardware execution breakpoint. Read both intern tables from the stopped process using the layouts from the pinned CPython headers. Do not call Python diagnostic functions or evaluate additional source in the process. Require two complete canonical reads to agree, executable bytes to remain unchanged, and observed and unobserved runs to emit identical marshal bytes.

For the checked-in supplement, remove names already generated from `pycore_global_strings.h` or `Python-ast.c`, single-character cached strings, and the three absolute filenames under the interpreter installation's `encodings` package. Keep installation-specific strings in an explicit runtime profile. The resulting sorted, unique list contains 2,790 entries and contains no installation paths. Changing the fixture requires a new verified capture and updating its pinned digest in `generate_cpython_target.py`.

Verify both target versions with:

```sh
python3 crates/serc/scripts/generate_cpython_target.py --cpython /path/to/cpython-3.14.5 --check
python3 crates/serc/scripts/generate_cpython_target.py --cpython /path/to/cpython-3.15.0rc2 --target-version 3.15 --check
```
