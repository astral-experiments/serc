# Isolated-target Unicode parsing model

The generated `src/target/isolated_unicode.rs` tables describe a conditional
CPython target model. They do not inspect or qualify an installed interpreter.
Selecting this model is distinct from admitting a live comparator caller.

The model requires the existing isolated startup contract, standard source
decoding, unchanged native `unicodedata` normalization and name lookup, standard
import machinery, and retained canonical owners. Its prepared plan contains one
Unicode parsing event and no warning events. Arbitrary complete or partial
caller states, modified hooks/modules/capsules, and unsupported parser-warning
interleavings cannot acquire these facts implicitly. Ordinary library and
prepared-compilation behavior remains unchanged.

The native module and UCD descriptors retain the 17 listed canonical keys after
successful initialization. This remains a partial positive subset: other
additions are unknown. Native NFKC and named-character lookup construct their
results without interning them; subsequent parser interning remains separately
modeled, including normalized dotted import prefixes.

For Python 3.13 through 3.15, the existing source closure supplies a
conservative set of possible ASCII dotted names from the pinned core and frozen
code. Under the model's native import assumptions, the Unicode event can
preserve an incoming absence for a dotted identifier outside that set and the
namespaces `unicodedata.`, `UCD.`, `Segment.`, and `GraphemeBreakIterator.`.
These exclusions cover native type, capsule, and descriptor names. The fixed
top-level import loads the native module, not another Python source unit;
directory-cache entries and file paths are not interned by that route. Python
3.12 has no preservation table in this model.

The preservation rule never establishes absence, restores knowledge invalidated
by an earlier unknown event, or revises an earlier canonical owner. Apply it
only to finite candidates bound to the prepared input. Unlisted or unproved
strings stay unknown. Later warning or other unknown effects retain their usual
behavior.

`generate_cpython_isolated_unicode.py` reads whole-hash-pinned Unicode source
proofs, their independently reviewed dotted companions, and the default startup
metadata. It verifies target and helper bindings, then copies the exact retained
and possible-name arrays. A sorted union with target-membership bits stores the
possible names compactly; generation verifies that each reconstructed target set
exactly equals its original array. The startup files identify the preexisting
target model; this generator never derives an absence fact from their
observations. The source proofs' native runtime bindings remain provenance for
their original qualification and are not assertions that a new caller has been
qualified.

The output records all input hashes and the generator hash. Updating an upstream
source proof, default model, or helper requires reviewing the conditional model
again before changing its pins. No native execution is part of generation.
