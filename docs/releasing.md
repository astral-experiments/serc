# Releasing Serc

Serc uses uv's Maturin packaging and trusted publishing pattern. Releases are
manually dispatched from `main` and publish after successful build checks, with
no separate environment approval. The GitHub repository can remain private. The
Python package contains the native `serc` executable and supports both `serc`
and `python -m serc`. Cargo publishes the `serc` library and the `serc-cli`
package (`cargo install serc-cli`).

## One-time setup

1. On PyPI, register a
   [pending trusted publisher](https://docs.pypi.org/trusted-publishers/creating-a-project-through-oidc/)
   with project name `serc`, owner `astral-sh`, repository `serc`, workflow
   `release.yml`, and environment `release`. A pending publisher does not
   reserve the package name until the first upload. Transfer the project to
   Astral's PyPI organization after creation, as appropriate.
2. Merge the Serc policy in `astral-sh/crates-policies`, then run its **Apply**
   workflow with `confirm=true`. The utility publishes `0.0.0` placeholders for
   `serc` and `serc-cli`, then configures trusted publishing from repository
   owner `astral-sh`, repository `serc`, workflow `release.yml`, environment
   `release`, with `trustpub_only: true`. Its existing production token handles
   bootstrapping; Serc does not need a registry token. A successful dry run
   makes no changes.
3. Add Serc to the release policies in
   [`astral-sh/github-policies`](https://github.com/astral-sh/github-policies/blob/main/releases/README.md):
   the Serc entry in `environments/release.json` and
   `rulesets/tags-require-release.json`. The `release` environment permits only
   `main`, with no required reviewers or custom deployment protection rules.
   Keep this environment name because the registry trusted publishers are bound
   to it. Serc does not use a `release-gate` environment or require the
   `release-environment-gate` App. This is an explicit exception to uv's
   separate approval step while Serc remains private.
4. Grant the existing `astral-automations-bot` GitHub App access to Serc. The
   broker reads `.github/secure-token-service.json` from Serc's `main` branch;
   the checked-in policy permits only `release-prepare.yml`, dispatched on
   `main` in the `automations` environment, to request `contents: write` and
   `pull_requests: write` for Serc. It pins Serc's repository ID and immutable
   OIDC identity. Add an `STS_API_URL` secret to the `automations` environment
   with the same base URL used by uv (without `/exchange`). Obtain it from the
   service administrator or the deployed stack's `PolicyAudience` output; GitHub
   does not reveal existing secret values. No new broker deployment or App
   private key is needed. This token allows the release PR to trigger ordinary
   CI.
5. Grant the existing `astral-releases-bot` GitHub App access to Serc and add
   `RELEASES_STS_API_URL` to the `release` environment, using the same release
   broker base URL as uv. Its policy is
   `.github/secure-token-service-release.json` and permits `release.yml` on
   `main` to request `contents: write` and `workflows: write`. The latter allows
   the release to tag its selected commit even if `main` advances across
   workflow changes while artifacts are building.

The trusted publisher uses the calling workflow filename (`release.yml`), even
though the publishing jobs are implemented in reusable workflows. No persistent
registry tokens are needed after the crates.io bootstrap.

## Release procedure

Run **Prepare release** on `main`. Rooster generates the changelog and chooses
the next version from PR labels; an optional `version` input overrides that
choice. The first release defaults to the current package version, `0.1.0`. The
workflow updates Python and Cargo versions, the CLI's library dependency, and
both lockfiles, then opens a PR from `charlie/release-VERSION`. Review and merge
that PR before publishing. The same preparation is available locally through
`scripts/release.sh`.

Like
[uv's release workflow](https://github.com/astral-sh/uv/blob/c73db78f0b00580b3c67279b59b85153a9d0264d/.github/workflows/release.yml),
**Release** is manually dispatched with a **Release Tag** input that defaults to
`dry-run`. Run it on `main` with that default to build and check all wheels, the
source distribution, and Cargo packages without publishing. To publish, enter
the version from the merged release PR (for example, `0.1.0`). The workflow
checks that the tag matches the packages being released, as Cargo Dist does for
uv.

After successful builds, the workflow publishes to PyPI, then starts crates.io
publishing and creates the GitHub release using the prepared changelog. A
crates.io failure does not block the GitHub release, matching uv. There is no
separate approval job. Published packages, including the source distribution and
crate sources, are public even when the GitHub repository is private.

The crates publisher skips versions already on crates.io, allowing an
interrupted release to resume. PyPI's publisher likewise checks existing files
when retrying. Never move an existing release tag to a different commit.
