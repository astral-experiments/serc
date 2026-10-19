# Releasing Serc

Serc follows ty's release workflow: Cargo Dist plans the release and generates
standalone installers and manifests; Maturin builds the native executables,
Python wheels, and source distribution. Releases publish standalone archives and
SHA-256 checksums to GitHub and Astral's R2 mirror, Python packages to PyPI, and
`serc` and `serc-cli` to crates.io. The Python package supports both `serc` and
`python -m serc`; `cargo install serc-cli` installs the Rust CLI.

Release wheels and standalone archives use profile-guided optimization on macOS
ARM64, Windows x64, and GNU Linux x64 and ARM64. Each platform trains on the
pinned production-source and Ruff corpora with CPython 3.14.5, then builds the
wheel with the same Rust toolchain and merged profile. Before archiving, the
final executable must reproduce the ordinary build's bytecode in serial and
parallel compilation. Training manifests, profiles, and logs are retained as
workflow artifacts for seven days.

Native archives are unsigned. GitHub build-provenance attestations identify the
workflow and source commit that produced the release artifacts.

## One-time setup

The settings below are managed outside this repository. Configure them before
publishing with this workflow; a dry run does not verify publishing credentials
or deployment protection rules.

1. Apply the same release protection as ty through
   [`astral-sh/github-policies`](https://github.com/astral-sh/github-policies/blob/main/releases/README.md).
   Add Serc to `environments/release-gate.json`, the `release-environment-gate`
   entry in `environments/release.json`, and
   `rulesets/tags-require-release.json`. Grant the `release-environment-gate`
   GitHub App access to Serc before applying the policy. Both environments must
   permit only `main`. The `release-gate` environment requires a reviewer from
   Astral's `full-time` team, prevents self-review, and disallows administrator
   bypass. Its approval unlocks the `release` environment's custom deployment
   rule for the remaining publishing jobs. Keep the `release` environment name:
   the registry trusted publishers are bound to it.
2. Add these secrets to the `release` environment for the same R2 bucket used by
   ty. Obtain the values from the service administrator; GitHub does not reveal
   existing secret values:
   - `MIRROR_R2_ACCESS_KEY_ID`
   - `MIRROR_R2_SECRET_ACCESS_KEY`
   - `MIRROR_R2_CLOUDFLARE_ACCOUNT_ID`
   - `MIRROR_R2_BUCKET_NAME`

   Versioned downloads are mirrored under
   `https://releases.astral.sh/github/serc/releases/download/VERSION/` with
   immutable caching. Stable releases also update
   `installers/serc/latest/serc-installer.sh` and
   `installers/serc/latest/serc-installer.ps1` with five-minute caching.

   Configure `astral.sh/serc/install.sh` and `astral.sh/serc/install.ps1` as
   permanent redirects to their corresponding latest installer URLs under
   `https://releases.astral.sh/installers/serc/latest/`. These redirects are
   managed outside this repository. Versioned URLs,
   `astral.sh/serc/VERSION/install.{sh,ps1}`, must redirect to
   `https://releases.astral.sh/github/serc/releases/download/VERSION/serc-installer.{sh,ps1}`.

3. Grant `astral-releases-bot` access to Serc and configure
   `RELEASES_STS_API_URL` in the `release` environment with the existing release
   broker's base URL. Serc's `.github/secure-token-service-release.json` permits
   `release.yml` on `main` to request `contents: write` and `workflows: write`.
   The latter allows tagging the selected commit even if `main` advances across
   workflow changes while artifacts are building.
4. Configure Serc authorization in
   [`astral-sh/versions`](https://github.com/astral-sh/versions/blob/main/.github/secure-token-service-release.json)
   before publishing version metadata. The policy must identify Serc by
   repository ID `1385584662`, owner ID `115962839`, and its immutable OIDC
   subject. It permits `release.yml` to call `publish-versions.yml` from `main`
   in the `release` environment and request `contents: write` and
   `pull_requests: write` for `astral-sh/versions`. The release bot must also
   have access to that repository. The first release creates `v1/serc.ndjson`;
   the versions repository's existing workflow mirrors it to R2.
5. Configure PyPI trusted publishing for project `serc`, owner `astral-sh`,
   repository `serc`, workflow `release.yml`, and environment `release`. For a
   new project, register a
   [pending trusted publisher](https://docs.pypi.org/trusted-publishers/creating-a-project-through-oidc/)
   and transfer the project to Astral's PyPI organization after creation.
6. Apply the Serc policy in `astral-sh/crates-policies` with `confirm=true`. Its
   existing production token bootstraps `serc` and `serc-cli`, then enables
   trusted publishing from `astral-sh/serc`, workflow `release.yml`, environment
   `release`, with `trustpub_only: true`. Serc does not need a registry token.
7. For release preparation, grant `astral-automations-bot` access to Serc and
   configure `STS_API_URL` in the `automations` environment. The broker reads
   `.github/secure-token-service.json` from `main`; its policy permits
   `release-prepare.yml` on `main` to request `contents: write` and
   `pull_requests: write` for Serc. This token allows the release PR to trigger
   ordinary CI. Configure `OPENAI_API_KEY` in that environment for the changelog
   rewrite step.

The registry trusted publishers use the calling workflow filename,
`release.yml`, even when publishing runs in reusable workflows.

## Release procedure

Run **Prepare release** on `main`. Rooster generates the changelog and chooses
the next version from PR labels; an optional `version` input overrides that
choice. The workflow updates Python and Cargo versions, Cargo Dist metadata, the
CLI's library dependency, and both lockfiles, then opens a PR from
`release/VERSION`. Review and merge that PR before publishing. The same
preparation is available locally through `scripts/release.sh`.

Run **Release** on `main` with its default **Release Tag**, `dry-run`, to build
and check the wheels, source distribution, standalone archives, installers,
release manifest, and Cargo packages without publishing. To publish, enter the
version from the merged release PR. The workflow checks that the tag matches the
packages being released and builds from the selected commit.

Publishing requires another full-time Astral employee to approve the
`release-gate` job.

After the build checks pass, the workflow publishes to PyPI and crates.io and
creates the GitHub release with its manifest, checksums, and provenance
attestations. It then mirrors standalone downloads and installers to R2 and
updates the versions repository. A crates.io failure does not block the GitHub
release.

The crates publisher skips versions already on crates.io, allowing an
interrupted release to resume. PyPI's publisher likewise checks existing files
when retrying. Never move an existing release tag to a different commit.
