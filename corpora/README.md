# Compatibility corpora

This directory pins the opt-in mypy_primer corpus. The
[compatibility guide](../docs/compatibility.md) provides comparison commands,
current results, and limitations; the required Ruff gate uses `vendor/ruff`.

## Snapshot

[mypy-primer.lock.json](mypy-primer.lock.json) records the mypy_primer catalog
revision, exact project revisions, file count, and source-content digest. The
snapshot includes every tracked regular `.py` and `.pyi` file in each locked
top-level repository, regardless of mypy_primer's type-checking paths. It
excludes submodule contents, symlinks, untracked files, and other suffixes. The
domain-separated content hash covers file contents and sorted repository IDs and
POSIX-relative paths.

## Validate the snapshot

```console
python3 scripts/mypy_primer_corpus.py check-catalog --primer-checkout /path/to/mypy_primer
python3 -m unittest discover -s scripts -p 'test_*.py'
```

The catalog checkout must be at the locked commit. These checks run in CI
without downloading or compiling the project corpus.

`./scripts/setup-mypy-primer-corpus` materializes sparse, detached checkouts
under `target/corpora/mypy-primer`. The
[compatibility check](../docs/compatibility.md#public-corpus-compatibility)
rejects missing files, revision or content drift, tracked modifications, and
untracked files before compilation.

## Update the snapshot

From Serc's repository root, pass a checkout of the chosen mypy_primer revision:

```console
python3 scripts/mypy_primer_corpus.py update-lock --primer-checkout /path/to/mypy_primer
./scripts/setup-mypy-primer-corpus
python3 scripts/mypy_primer_corpus.py lock-content
python3 scripts/mypy_primer_corpus.py verify
```

Review every resolved repository revision. Keep the catalog revision in the lock
and `.github/workflows/ci.yml` identical. Rerun the catalog check, manager
tests, and full compatibility comparison, explicitly updating the outcome
baseline and
[compatibility guide](../docs/compatibility.md#public-corpus-compatibility) when
results change. Ordinary validation must not refresh revisions or resolve moving
branches.
