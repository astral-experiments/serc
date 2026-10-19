from __future__ import annotations

import argparse
import os
import tomllib
from pathlib import Path

from release_version import python_version

ROOT = Path(__file__).resolve().parent.parent


def release_version(root: Path) -> str:
    manifests = [
        ("crates/serc/Cargo.toml", "package"),
        ("crates/serc_cli/Cargo.toml", "package"),
        ("dist-workspace.toml", "workspace"),
    ]
    versions = {
        filename: tomllib.loads((root / filename).read_text())[section]["version"]
        for filename, section in manifests
    }
    cli = tomllib.loads((root / "crates/serc_cli/Cargo.toml").read_text())
    versions["CLI serc dependency"] = cli["dependencies"]["serc"]["version"]
    if len(set(versions.values())) != 1:
        raise ValueError(f"Release versions disagree: {versions}")
    version = next(iter(versions.values()))
    python_project = tomllib.loads((root / "pyproject.toml").read_text())["project"]
    if python_project["version"] != python_version(version):
        raise ValueError(
            f"Release versions disagree: Python {python_project['version']}, Rust {version}"
        )
    return version


def main() -> None:
    parser = argparse.ArgumentParser(description="Validate a Serc release version.")
    parser.add_argument("tag", nargs="?", default="dry-run")
    args = parser.parse_args()
    version = release_version(ROOT)
    publishing = args.tag != "dry-run"
    if publishing:
        if args.tag not in {version, f"v{version}"}:
            raise ValueError(f"Release tag must match version {version}")
        if os.environ.get("GITHUB_ACTIONS"):
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
