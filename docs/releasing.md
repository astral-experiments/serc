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
3. Add Serc to the release policies in
   [`astral-sh/github-policies`](https://github.com/astral-sh/github-policies/blob/main/releases/README.md):
   `environments/release-gate.json`, the custom-app entry in
   `environments/release.json`, and `rulesets/tags-require-release.json`. Grant
   `astral-sh/full-time` repository access and install the existing
   `release-environment-gate` GitHub App for Serc before applying the policies.
   `release-gate` requires approval from `astral-sh/full-time`, prevents self-review,
   and disallows administrator bypass. The custom App protects `release` after
   human approval of `release-gate`. Both environments permit only `main`.
   Both required reviewers and custom deployment protection rules require GitHub
   Enterprise for a private repository, or making the repository public.
4. Grant the existing `astral-automations-bot` GitHub App access to Serc. The broker
   reads `.github/secure-token-service.json` from Serc's `main` branch; the checked-in
   policy permits only `release-prepare.yml`, dispatched on `main` in the
   `automations` environment, to request `contents: write` and `pull_requests: write`
   for Serc. It pins Serc's repository ID and immutable OIDC identity. Add an
   `STS_API_URL` secret to the `automations` environment with the same base URL used
   by uv (without `/exchange`). Obtain it from the service administrator or the
   deployed stack's `PolicyAudience` output; GitHub does not reveal existing secret
   values. No new broker deployment or App private key is needed. This token allows
   the release PR to trigger ordinary CI.
5. Set the repository Actions variable `SERC_RELEASES_ENABLED` to `true` after
   registry registration and environment protection are complete. It starts as
   `false`; build-only dry runs work while publishing is disabled.

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
