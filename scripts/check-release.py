from __future__ import annotations

import argparse
import os
import re
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def release_version(root: Path) -> str:
    manifests = [
        ("pyproject.toml", "project"),
        ("crates/serc/Cargo.toml", "package"),
        ("crates/serc_cli/Cargo.toml", "package"),
    ]
    versions = {
        filename: tomllib.loads((root / filename).read_text())[section]["version"]
        for filename, section in manifests
    }
    cli = tomllib.loads((root / "crates/serc_cli/Cargo.toml").read_text())
    versions["CLI serc dependency"] = cli["dependencies"]["serc"]["version"]
    if len(set(versions.values())) != 1:
        raise ValueError(f"Release versions disagree: {versions}")
    return next(iter(versions.values()))


def main() -> None:
    parser = argparse.ArgumentParser(description="Validate a Serc release version.")
    parser.add_argument("tag", nargs="?", default="dry-run")
    args = parser.parse_args()
    version = release_version(ROOT)
    if not re.fullmatch(r"\d+\.\d+\.\d+", version):
        raise ValueError(f"Expected a stable release version, got {version!r}")
    publishing = args.tag != "dry-run"
    if publishing:
        if args.tag not in {version, f"v{version}"}:
            raise ValueError(f"Release tag must match version {version}")
        if os.environ.get("GITHUB_ACTIONS"):
            if os.environ.get("SERC_RELEASES_ENABLED") != "true":
                raise ValueError(
                    "Complete the release setup in docs/releasing.md first"
                )
            if os.environ.get("GITHUB_REPOSITORY") != "astral-sh/serc":
                raise ValueError("Publishing is restricted to astral-sh/serc")
            if os.environ.get("GITHUB_REF") != "refs/heads/main":
                raise ValueError("Publish releases from main")
    values = {
        "version": version,
        "tag": args.tag,
        "publishing": str(publishing).lower(),
    }
    for key, value in values.items():
        print(f"{key}={value}")
    if output := os.environ.get("GITHUB_OUTPUT"):
        with Path(output).open("a") as stream:
            for key, value in values.items():
                stream.write(f"{key}={value}\n")


if __name__ == "__main__":
    main()
