from __future__ import annotations

import argparse
import email
import tarfile
import tomllib
import zipfile
from pathlib import Path


def main() -> None:
    parser = argparse.ArgumentParser(description="Check the complete release bundle.")
    parser.add_argument("directory", type=Path)
    parser.add_argument("--expected-wheels", type=int, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    version = tomllib.loads((root / "pyproject.toml").read_text())["project"]["version"]
    wheels = sorted(args.directory.glob("*.whl"))
    sdists = sorted(args.directory.glob("*.tar.gz"))
    if len(wheels) != args.expected_wheels or len(sdists) != 1:
        raise ValueError(
            f"Incomplete release: {len(wheels)} wheels, {len(sdists)} sdists"
        )
    if set(args.directory.iterdir()) != {*wheels, *sdists}:
        raise ValueError("Unexpected files in the release bundle")
    platforms = set()
    for wheel in wheels:
        if not wheel.name.startswith(f"serc-{version}-py3-none-"):
            raise ValueError(f"Unexpected wheel name: {wheel.name}")
        platforms.add(wheel.name.rsplit("-", 1)[1])
        with zipfile.ZipFile(wheel) as archive:
            names = set(archive.namelist())
            prefix = f"serc-{version}"
            required = {
                "serc/__init__.py",
                "serc/__main__.py",
                "serc/_find_serc.py",
                f"{prefix}.dist-info/licenses/LICENSE-APACHE",
                f"{prefix}.dist-info/licenses/LICENSE-MIT",
            }
            if missing := required - names:
                raise ValueError(f"{wheel.name}: missing {missing}")
            binary = f"{prefix}.data/scripts/serc"
            if binary not in names and f"{binary}.exe" not in names:
                raise ValueError(f"{wheel.name}: missing executable")
            metadata = email.message_from_bytes(
                archive.read(f"{prefix}.dist-info/METADATA")
            )
            if metadata["Name"] != "serc" or metadata["Version"] != version:
                raise ValueError(f"{wheel.name}: incorrect package metadata")
    if len(platforms) != args.expected_wheels:
        raise ValueError("Duplicate wheel platforms")
    if sdists[0].name != f"serc-{version}.tar.gz":
        raise ValueError(f"Unexpected source distribution: {sdists[0].name}")
    with tarfile.open(sdists[0]) as archive:
        names = set(archive.getnames())
        for filename in (
            "pyproject.toml",
            "Cargo.lock",
            "LICENSE-APACHE",
            "LICENSE-MIT",
        ):
            if f"serc-{version}/{filename}" not in names:
                raise ValueError(f"Source distribution is missing {filename}")
    print(f"Verified {len(wheels)} wheels and one source distribution for {version}")


if __name__ == "__main__":
    main()
