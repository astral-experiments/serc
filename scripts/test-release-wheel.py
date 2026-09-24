from __future__ import annotations

import argparse
import os
import subprocess
import sys
import tempfile
import venv
from pathlib import Path


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Install and smoke-test a release wheel."
    )
    parser.add_argument("directory", type=Path)
    args = parser.parse_args()
    wheels = list(args.directory.resolve().glob("*.whl"))
    if len(wheels) != 1:
        raise ValueError(f"Expected one wheel, found {wheels}")
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        venv.create(root / "venv", with_pip=True)
        scripts = root / "venv" / ("Scripts" if os.name == "nt" else "bin")
        python = scripts / ("python.exe" if os.name == "nt" else "python")
        binary = scripts / ("serc.exe" if os.name == "nt" else "serc")
        subprocess.run(
            [python, "-m", "pip", "install", "--no-index", "--no-deps", wheels[0]],
            cwd=root,
            check=True,
        )
        for command in ([binary], [python, "-m", "serc"]):
            subprocess.run([*command, "--version"], cwd=root, check=True)
            subprocess.run([*command, "--help"], cwd=root, check=True)
            result = subprocess.run(
                [*command, "--invalid-option"], cwd=root, capture_output=True
            )
            if result.returncode != 2:
                sys.exit(
                    f"Expected argument error exit code 2, got {result.returncode}"
                )


if __name__ == "__main__":
    main()
