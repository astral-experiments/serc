# Compatibility corpora

Serc's required compatibility gate remains the Ruff fixture corpus documented
in the repository root. The mypy_primer-derived corpus is a larger, separately
pinned diagnostic for exercising Serc against real-world Python projects.

## mypy_primer snapshot

The catalog is derived from
[`hauntsaninja/mypy_primer`](https://github.com/hauntsaninja/mypy_primer) at
commit `ff3d9531335dad605d43fcc208c9c159bb09fa81`. Project revisions were resolved
at `2026-07-07T15:14:02+00:00` and are recorded in
[`mypy-primer.lock.json`](mypy-primer.lock.json). The frozen materialized corpus
has the following identity:

- Catalog entries: 162
- Distinct repositories: 160
- Python source and stub files: 73,537
- Source bytes: 691,638,412
- SHA-256: `58699031040d02c1a4d18c9a5425d0250393316b04e0bee4819474c188f6484b`

The corpus contains every tracked regular file ending in `.py` or `.pyi` in
each locked top-level repository. It does not use mypy_primer's configured type
checking paths. Submodule contents, symlink files and directories, generated or
untracked files, and files with other suffixes are excluded.

The SHA-256 is a domain-separated aggregate over the included file contents and
their sorted repository IDs and POSIX-relative paths. It does not identify the
lock file or Git trees themselves; the lock separately records each exact
project revision. The lock, rather than current upstream default branches,
defines the corpus, and ordinary validation must not refresh those revisions.

## Validate the catalog

Given a checkout of mypy_primer at the pinned commit, validate that the checked-in
catalog still matches it:

```console
python3 scripts/mypy_primer_corpus.py check-catalog \
  --primer-checkout /path/to/mypy_primer
```

Run the corpus manager's focused unit tests with:

```console
python3 -m unittest discover -s scripts -p 'test_*.py'
```

These lightweight checks run in CI. They do not download or compile the full
project corpus.

## Update the snapshot

Snapshot updates are explicit maintainer operations. Check out the chosen
mypy_primer revision, resolve its current repository heads, materialize those
exact commits, and then freeze their content identity:

```console
python3 scripts/mypy_primer_corpus.py update-lock \
  --primer-checkout /path/to/mypy_primer
./scripts/setup-mypy-primer-corpus
python3 scripts/mypy_primer_corpus.py lock-content
python3 scripts/mypy_primer_corpus.py verify
```

Review every repository revision change. Keep the catalog revision in
`mypy-primer.lock.json`, the checkout revision in `.github/workflows/ci.yml`,
and the revision documented above identical. Then rerun the catalog check,
manager tests, and full compatibility comparison before committing the new
lock. Normal CI validation never resolves moving upstream branches.

## Materialize and compare

Materialize sparse, detached checkouts of the exact locked project revisions
under the ignored `target/` directory:

```console
./scripts/setup-mypy-primer-corpus
```

Then run the frozen CPython 3.14.5 outcome comparison:

```console
./scripts/check-mypy-primer-compatibility /path/to/python3.14.5
```

The check first rejects missing files, content-identity drift, tracked
modifications, and untracked files. The initial outcome baseline requires 52,080
byte-identical files, 21,298 failures, and 159 files rejected by CPython. The
failures comprise 21,276 byte mismatches, 12 parse or decode gaps, nine compiler
panics, and one oracle failure.

A full run exits unsuccessfully when either exact matches or CPython rejections
change, so improvements and regressions both require an intentional baseline
update. Failure categories remain diagnostic; the baseline freezes their aggregate.

For a faster local smoke diagnostic, limit the sorted corpus prefix explicitly:

```console
./scripts/check-mypy-primer-compatibility /path/to/python3.14.5 --limit 100
```

A limited run is not a full corpus result. The complete comparison is opt-in;
it supplements the always-on Ruff gate and does not alter Ruff's frozen file or
outcome counts.
