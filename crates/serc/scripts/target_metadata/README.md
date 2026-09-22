# CPython startup metadata

These fixtures supplement generated static and compiler-initialized interned strings on Linux
for the [pinned CPython targets](../../README.md#updating-the-cpython-target). Each JSON file
records its exact target, oracle script, input and snapshot hashes, entry counts, filtering rules,
and capture procedure. Other process states can be supplied through `InitialInternState`.

A replacement requires a verified capture from that target's isolated, default-GIL interpreter
compiling empty input. Follow the fixture's `capture_protocol` and pinned header layouts; read
both intern tables externally without executing diagnostic Python code in the observed process.
Require two complete reads to agree, unchanged executable bytes, and identical observed and
unobserved marshal output. Never derive startup entries from corpus strings or another release.

Subtract generated static/AST names and cached single-character strings. Keep installation paths
in the [explicit runtime profile](../../examples/support/initial-intern-profile.md), not in these
sorted, unique supplements. Update the fixture's pinned digest in
[`generate_cpython_target.py`](../generate_cpython_target.py), then
[regenerate and check the target](../../README.md#updating-the-cpython-target).
