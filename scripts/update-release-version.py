from __future__ import annotations

import re
import tomllib
from pathlib import Path

from release_version import python_version


def update_versions(root: Path) -> str:
    """Copy Rooster's changelog version into package manifests and release metadata."""
    heading = re.search(r"^## (\S+)\s*$", (root / "CHANGELOG.md").read_text(), re.M)
    if heading is None:
        raise ValueError("Missing release version in CHANGELOG.md")
    version = heading[1]
    normalized_python_version = python_version(version)
    for filename, section in [
        ("pyproject.toml", "project"),
        ("crates/serc/Cargo.toml", "package"),
        ("crates/serc_cli/Cargo.toml", "package"),
        ("dist-workspace.toml", "workspace"),
    ]:
        path = root / filename
        content = path.read_text()
        old = tomllib.loads(content)[section]["version"]
        new_version = (
            normalized_python_version if filename == "pyproject.toml" else version
        )
        content, count = re.subn(
            rf'^version = "{re.escape(old)}"$',
            f'version = "{new_version}"',
            content,
            count=1,
            flags=re.M,
        )
        if count != 1:
            raise ValueError(f"Could not update version in {filename}")
        if filename == "crates/serc_cli/Cargo.toml":
            old_pin = tomllib.loads(content)["dependencies"]["serc"]["version"]
            content, count = re.subn(
                rf'^(serc = \{{[^\n]*version = "){re.escape(old_pin)}(")',
                rf"\g<1>{version}\2",
                content,
                count=1,
                flags=re.M,
            )
            if count != 1:
                raise ValueError("Could not update the CLI's serc dependency")
        path.write_text(content)
    return version


if __name__ == "__main__":
    print(update_versions(Path(__file__).resolve().parent.parent))
