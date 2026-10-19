from __future__ import annotations

import argparse
import io
import shutil
import tarfile
import tomllib
import zipfile
from pathlib import Path

# Match each release matrix target to its wheel's platform tag. Checking both prevents a
# mislabeled artifact from publishing a binary for the wrong target.
WHEEL_PLATFORMS = {
    "x86_64-apple-darwin": "macosx_10_12_x86_64",
    "aarch64-apple-darwin": "macosx_11_0_arm64",
    "x86_64-pc-windows-msvc": "win_amd64",
    "i686-pc-windows-msvc": "win32",
    "aarch64-pc-windows-msvc": "win_arm64",
    "x86_64-unknown-linux-gnu": "manylinux_2_17_x86_64.manylinux2014_x86_64",
    "i686-unknown-linux-gnu": "manylinux_2_17_i686.manylinux2014_i686",
    "aarch64-unknown-linux-gnu": "manylinux_2_28_aarch64",
    "armv7-unknown-linux-gnueabihf": "manylinux_2_17_armv7l.manylinux2014_armv7l",
    "powerpc64le-unknown-linux-gnu": "manylinux_2_17_ppc64le.manylinux2014_ppc64le",
    "s390x-unknown-linux-gnu": "manylinux_2_17_s390x.manylinux2014_s390x",
    "riscv64gc-unknown-linux-gnu": "manylinux_2_31_riscv64",
    "x86_64-unknown-linux-musl": "musllinux_1_1_x86_64",
    "i686-unknown-linux-musl": "musllinux_1_1_i686",
    "aarch64-unknown-linux-musl": "musllinux_1_1_aarch64",
    "armv7-unknown-linux-musleabihf": "musllinux_1_1_armv7l",
    "arm-unknown-linux-musleabihf": "linux_armv6l",
    "riscv64gc-unknown-linux-musl": "musllinux_1_1_riscv64",
}


def files_from_wheel(wheel: Path, target: str, version: str) -> dict[str, bytes]:
    expected = f"serc-{version}-py3-none-{WHEEL_PLATFORMS[target]}.whl"
    if wheel.name != expected:
        raise ValueError(f"{target}: expected {expected}, found {wheel.name}")
    executable = "serc.exe" if "-windows-" in target else "serc"
    paths = {
        executable: f"serc-{version}.data/scripts/{executable}",
        "LICENSE-MIT": f"serc-{version}.dist-info/licenses/LICENSE-MIT",
        "LICENSE-APACHE": f"serc-{version}.dist-info/licenses/LICENSE-APACHE",
    }
    with zipfile.ZipFile(wheel) as archive:
        if any(name.startswith("serc.libs/") for name in archive.namelist()):
            raise ValueError(
                f"{wheel.name}: standalone archives cannot bundle libraries"
            )
        for path in paths.values():
            if archive.namelist().count(path) != 1:
                raise ValueError(f"{wheel.name}: expected one {path}")
        files = {name: archive.read(path) for name, path in paths.items()}
    if "-windows-" in target:
        magic = b"MZ"
    elif "-apple-" in target:
        magic = b"\xcf\xfa\xed\xfe"
    else:
        magic = b"\x7fELF"
    if not files[executable].startswith(magic):
        raise ValueError(f"{wheel.name}: expected a native executable for {target}")
    return files


def create_archive(wheel: Path, target: str, version: str, output: Path) -> Path:
    files = files_from_wheel(wheel, target, version)
    if "-windows-" in target:
        path = output / f"serc-{target}.zip"
        with zipfile.ZipFile(path, "w") as archive:
            for name, contents in files.items():
                info = zipfile.ZipInfo(name)
                info.external_attr = (
                    0o100755 if name == "serc.exe" else 0o100644
                ) << 16
                info.compress_type = zipfile.ZIP_DEFLATED
                archive.writestr(info, contents)
    else:
        path = output / f"serc-{target}.tar.gz"
        with tarfile.open(path, "w:gz") as archive:
            for name, contents in files.items():
                info = tarfile.TarInfo(f"serc-{target}/{name}")
                info.mode = 0o755 if name == "serc" else 0o644
                info.size = len(contents)
                archive.addfile(info, io.BytesIO(contents))
    verify_archive(wheel, target, version, path)
    return path


def verify_archive(wheel: Path, target: str, version: str, path: Path) -> None:
    expected = files_from_wheel(wheel, target, version)
    if "-windows-" in target:
        if path.name != f"serc-{target}.zip":
            raise ValueError(f"Unexpected archive name: {path.name}")
        with zipfile.ZipFile(path) as archive:
            if archive.namelist() != list(expected):
                raise ValueError(f"{path.name}: unexpected archive contents")
            actual = {name: archive.read(name) for name in expected}
    else:
        if path.name != f"serc-{target}.tar.gz":
            raise ValueError(f"Unexpected archive name: {path.name}")
        with tarfile.open(path) as archive:
            members = archive.getmembers()
            if [member.name for member in members] != [
                f"serc-{target}/{name}" for name in expected
            ]:
                raise ValueError(f"{path.name}: unexpected archive contents")
            actual = {}
            for member in members:
                name = Path(member.name).name
                mode = 0o755 if name == "serc" else 0o644
                if not member.isfile() or member.mode != mode:
                    raise ValueError(f"{path.name}: unexpected file mode for {name}")
                contents = archive.extractfile(member)
                if contents is None:
                    raise ValueError(f"{path.name}: missing {name}")
                actual[name] = contents.read()
    if actual != expected:
        raise ValueError(f"{path.name}: contents differ from {wheel.name}")


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Create standalone archives from release wheels."
    )
    parser.add_argument(
        "wheels", type=Path, help="Downloaded wheel artifacts by target"
    )
    parser.add_argument(
        "distributions", type=Path, help="Directory for PyPI distributions"
    )
    parser.add_argument("archives", type=Path, help="Directory for standalone archives")
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    version = tomllib.loads((root / "pyproject.toml").read_text())["project"]["version"]
    expected = {f"serc-wheels-{target}" for target in WHEEL_PLATFORMS}
    actual = {entry.name for entry in args.wheels.iterdir()}
    if actual != expected:
        raise ValueError(
            f"Unexpected wheel artifacts: missing {expected - actual}, extra {actual - expected}"
        )
    args.distributions.mkdir(parents=True, exist_ok=True)
    args.archives.mkdir(parents=True, exist_ok=True)
    if any(args.archives.iterdir()):
        raise ValueError("Standalone archive directory is not empty")
    for target in WHEEL_PLATFORMS:
        directory = args.wheels / f"serc-wheels-{target}"
        entries = list(directory.iterdir())
        if len(entries) != 1 or not entries[0].is_file():
            raise ValueError(f"{target}: expected one wheel, found {entries}")
        wheel = entries[0]
        create_archive(wheel, target, version, args.archives)
        destination = args.distributions / wheel.name
        if destination.exists():
            raise ValueError(f"Duplicate distribution: {wheel.name}")
        shutil.copyfile(wheel, destination)
    print(f"Verified {len(WHEEL_PLATFORMS)} standalone archives for {version}")


if __name__ == "__main__":
    main()
