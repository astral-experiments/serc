The CLI's GNU/Linux x86-64 target models a fresh stock CPython process for each
file: `python [-O|-OO] -m compileall -q -f --invalidation-mode timestamp file.py`,
or `python [-O|-OO] -m py_compile file.pyi` for an explicit file without the
`.py` suffix. The pinned interpreter has no site customizations and its standard
library cache is populated for the requested optimization count. Library callers
retain their existing isolated startup model. Other platforms retain their
existing target metadata.

Imports performed before compilation retain canonical strings that can change
marshal sharing. The fixture records the exact target commits, imported source
hashes, complete-table capture receipts and strong-owner audits. Generated
profiles retain only proven canonical owners. Temporary enum construction keys
remain unknown; compilation reports unresolved state if one can affect output.
The table omits strings containing `/`: initial membership queries involve ASCII
constant strings and compiler-generated qualified names. Filename interning is
modeled separately by the compiler. Source-codec import effects remain enabled.

Optimization counts zero, one and two or greater select distinct profiles.
Repeated native `-OO` and `-OOO` captures and the pinned imported-module source
establish the last class; the raw flag count still determines the cache filename.

Regenerate the Rust table from the admitted fixture with:

```sh
python3 crates/serc_cli/scripts/generate_compileall_profiles.py \
  --cpython-312 /path/to/cpython-3.12.14 \
  --cpython-313 /path/to/cpython-3.13.15 \
  --cpython-314 /path/to/cpython-3.14.5 \
  --cpython-315 /path/to/cpython-3.15.0rc2
```

Add `--check` to verify the checked-in output. Regeneration verifies committed
source hashes and does not execute the target runtime. Refreshing the logical
fixture additionally requires native observations: warm each raw optimization
cache, stop a fresh process at source compilation before decoding, read complete
intern tables twice, capture owners, and compare observed output with an
unobserved invocation. Repeat with distinct public filenames and both callers.
Verify immutable executable/cache inputs and normal-startup flags. Audit mortal
owners from exact source-defined strong edges; derive temporary-key uncertainty
from enum constructors rather than the union of observed spellings. Raw memory
addresses and installation paths belong in capture receipts, outside the public
logical fixture.
