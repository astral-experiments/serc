# Compatibility corpora

The [compatibility guide](../docs/compatibility.md) owns comparison commands, outcome baselines,
and limitations. The required Ruff gate uses `vendor/ruff`; this directory pins the larger,
opt-in mypy_primer diagnostic corpus.
The [latest measurement summary](results/compatibility-2026-09-22-public-3a09916.json) records
the compiler and corpus identities, complete outcome counts, and evidence hashes.

## mypy_primer snapshot

The catalog comes from [`hauntsaninja/mypy_primer`](https://github.com/hauntsaninja/mypy_primer)
commit `ff3d9531335dad605d43fcc208c9c159bb09fa81`. Project revisions were resolved at
`2026-07-07T15:14:02+00:00` and are recorded in [mypy-primer.lock.json](mypy-primer.lock.json).

- 162 catalog entries across 160 repositories
- 73,537 Python source and stub files; 691,638,412 source bytes
- Content SHA-256: `58699031040d02c1a4d18c9a5425d0250393316b04e0bee4819474c188f6484b`

The snapshot includes every tracked regular `.py` and `.pyi` file in each locked top-level
repository, regardless of mypy_primer's type-checking paths. It excludes submodule contents,
symlinks, untracked files, and other suffixes. The domain-separated content hash
covers file contents and sorted repository IDs and POSIX-relative paths; the lock separately
identifies Git revisions.

## Validate the snapshot

```console
python3 scripts/mypy_primer_corpus.py check-catalog --primer-checkout /path/to/mypy_primer
python3 -m unittest discover -s scripts -p 'test_*.py'
```

The catalog checkout must be at the pinned commit. These checks run in CI without downloading
or compiling the project corpus.

`./scripts/setup-mypy-primer-corpus` materializes sparse, detached checkouts under
`target/corpora/mypy-primer`. The compatibility check rejects missing files, revision or content
drift, tracked modifications, and untracked files before compilation. It freezes each outcome
category separately, not just aggregate failures; see the
[public corpus gate](../docs/compatibility.md#public-corpus-compatibility).

## Update the snapshot

From Serc's repository root, pass a checkout of the chosen mypy_primer revision:

```console
python3 scripts/mypy_primer_corpus.py update-lock --primer-checkout /path/to/mypy_primer
./scripts/setup-mypy-primer-corpus
python3 scripts/mypy_primer_corpus.py lock-content
python3 scripts/mypy_primer_corpus.py verify
```

Review every resolved repository revision. Keep the catalog revision in the lock,
`.github/workflows/ci.yml`, and this guide identical. Rerun the catalog check, manager tests,
and full compatibility comparison, explicitly updating the outcome baseline for any changed
counts. Ordinary validation must not refresh revisions or resolve moving upstream branches.
