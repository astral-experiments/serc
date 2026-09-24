# Releasing Serc

Serc uses the same Maturin packaging, GitHub release environments, and trusted
publishing pattern as uv. The Python package contains the native `serc` executable
and supports both `serc` and `python -m serc`. Cargo publishes the `serc` library and
the `serc-cli` package (`cargo install serc-cli`).

## One-time setup

1. On PyPI, register a [pending trusted publisher](https://docs.pypi.org/trusted-publishers/creating-a-project-through-oidc/)
   with project name `serc`, owner `astral-sh`, repository `serc`, workflow
   `release.yml`, and environment `release`. A pending publisher does not reserve
   the package name until the first upload. Transfer the project to Astral's PyPI
   organization after creation, as appropriate.
2. Merge the Serc policy in `astral-sh/crates-policies`, then run its **Apply**
   workflow with `confirm=true`. The utility publishes `0.0.0` placeholders for
   `serc` and `serc-cli`, then configures trusted publishing from repository owner
   `astral-sh`, repository `serc`, workflow `release.yml`, environment `release`,
   with `trustpub_only: true`. Its existing production token handles bootstrapping;
   Serc does not need a registry token. A successful dry run makes no changes.
3. Finish the GitHub environments to match uv. `release-gate` requires approval
   from `astral-sh/full-time`, prevents self-review, and disallows administrator
   bypass. Install the `release-environment-gate` GitHub App for Serc and enable its
   deployment protection rule on `release`. Both environments permit only `main`.
   GitHub currently requires a plan supporting required reviewers, or a public
   repository, to configure the reviewer rule for this repository.
4. Set the repository Actions variable `SERC_RELEASES_ENABLED` to `true` after
   registry registration and environment protection are complete. It starts as
   `false`; build-only dry runs work while publishing is disabled.
5. Enable the Astral automation token broker for `astral-sh/serc`, allowing
   `release-prepare.yml` in the `automations` environment to request `contents: write`
   and `pull_requests: write`. Make `STS_API_URL` available to that environment, as
   in uv, and ensure the automation App has access to Serc. This token allows the
   release PR to trigger ordinary CI.

The trusted publisher uses the calling workflow filename (`release.yml`), even
though the publishing jobs are implemented in reusable workflows. No persistent
registry tokens are needed after the crates.io bootstrap.

## Release procedure

Run **Prepare release** on `main`. Rooster generates the changelog and chooses the
next version from PR labels; an optional `version` input overrides that choice.
The first release defaults to the current package version, `0.1.0`. The workflow
updates Python and Cargo versions, the CLI's library dependency, and both lockfiles,
then opens a PR from `charlie/release-VERSION`. Review and merge that PR before
publishing. The same preparation is available locally through `scripts/release.sh`.

Run the **Release** workflow on `main` with `tag=dry-run` to build and check all
wheels, the source distribution, and Cargo packages without publishing. Inspect
the artifacts and successful checks, then run it with the package version as the
tag (for example, `0.1.0`) and approve the `release-gate` deployment. This publishes
the verified Python artifacts and both crates, then creates the GitHub release.

The crates publisher skips versions already on crates.io, allowing an interrupted
release to resume. PyPI's publisher likewise checks existing files when retrying.
Never move an existing release tag to a different commit.
