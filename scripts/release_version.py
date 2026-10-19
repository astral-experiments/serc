from __future__ import annotations

import re


def python_version(version: str) -> str:
    """Convert the shared Rust release version to its canonical Python spelling."""
    number = r"(?:0|[1-9]\d*)"
    match = re.fullmatch(
        rf"({number}\.{number}\.{number})(?:-(alpha|beta|rc|dev)\.({number}))?",
        version,
    )
    if match is None:
        raise ValueError(f"Unsupported release version: {version!r}")
    base, stage, serial = match.groups()
    if stage is None:
        return base
    suffix = {"alpha": "a", "beta": "b", "rc": "rc", "dev": ".dev"}[stage]
    return f"{base}{suffix}{serial}"
