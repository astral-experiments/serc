"""Pinned production-source inputs for Serc's PGO training."""

from __future__ import annotations

import hashlib
import os
import re
import shutil
from dataclasses import asdict, dataclass
from pathlib import Path


@dataclass(frozen=True)
class Project:
    name: str
    repository: str
    revision: str
    source_directories: tuple[str, ...]

    def __post_init__(self):
        if re.fullmatch(r"[0-9a-f]{40}", self.revision) is None:
            raise ValueError(f"{self.name} requires a full Git commit SHA")

    @property
    def url(self):
        return f"https://github.com/{self.repository}.git"


# Same production-source subsets and pins as Ruff's PGO corpus at fb93595b28f3d38a209ed65c12fce114e249d715.
# Typeshed/.pyi inputs are omitted: Serc compiles executable Python modules.
PROJECTS = (
    Project(
        "pytest",
        "pytest-dev/pytest",
        "28e86a6c2ae0173831e4925a4af89b02a2936d09",
        ("src/_pytest",),
    ),
    Project(
        "httpx", "encode/httpx", "b5addb64f0161ff6bfe94c124ef76f6a1fba5254", ("httpx",)
    ),
    Project(
        "fastapi",
        "fastapi/fastapi",
        "a375f6b948b99fa4260129856bbf11d037f363ef",
        ("fastapi",),
    ),
    Project(
        "anyio",
        "agronholm/anyio",
        "ffe91331adb912c5d150f5d373f7cd28a0e96a62",
        ("src/anyio",),
    ),
    Project(
        "zulip",
        "zulip/zulip",
        "ccddbba7a3074283ccaac3bde35fd32b19faf042",
        ("zerver/views", "zerver/models"),
    ),
    Project(
        "warehouse",
        "pypi/warehouse",
        "5a4d2cadec641b5d6a6847d0127940e0f532f184",
        ("warehouse/accounts", "warehouse/oidc", "warehouse/forklift"),
    ),
    Project(
        "pip",
        "pypa/pip",
        "d1fd55753405fd728a0751a578e27c1054acdf48",
        ("src/pip/_internal",),
    ),
    Project(
        "sphinx",
        "sphinx-doc/sphinx",
        "b06d92e80eed130e1dd4e67cac4afa1267424f1a",
        ("sphinx/builders", "sphinx/ext/autodoc", "sphinx/domains/python"),
    ),
    Project(
        "astropy",
        "astropy/astropy",
        "b779108c7cec25c840c0f744fdf2a1550441e309",
        ("astropy/units",),
    ),
)
EXCLUDED_DIRECTORIES = {"test", "tests", "_tests", "vendor", "_vendor", "__pycache__"}


def tree_summary(root: Path, suffix: str):
    """Hash complete contents and relative names, independent of traversal order."""
    digest = hashlib.sha256()
    paths = sorted(path for path in root.rglob(f"*{suffix}") if path.is_file())
    total_bytes = 0
    for path in paths:
        name = path.relative_to(root).as_posix().encode()
        contents = path.read_bytes()
        digest.update(len(name).to_bytes(8, "big") + name)
        digest.update(len(contents).to_bytes(8, "big") + contents)
        total_bytes += len(contents)
    return {"files": len(paths), "bytes": total_bytes, "sha256": digest.hexdigest()}


def copy_sources(
    project: Project, checkout: Path, destination: Path, tracked_files: list[str]
):
    """Copy selected tracked sources; the fetched checkout is never compiled or modified."""
    for name in project.source_directories:
        if not (checkout / name).is_dir():
            raise ValueError(f"Missing {project.name} source directory: {name}")
    for name in sorted(tracked_files):
        relative = Path(name)
        source = checkout / relative
        if (
            relative.suffix != ".py"
            or EXCLUDED_DIRECTORIES.intersection(relative.parts[:-1])
            or not any(
                relative.is_relative_to(root) for root in project.source_directories
            )
            or source.is_symlink()
            or not source.is_file()
        ):
            continue
        target = destination / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, target)
    summary = tree_summary(destination, ".py")
    if not summary["files"]:
        raise ValueError(f"No Python training sources for {project.name}")
    return asdict(project) | summary


def prepare_corpus(cache: Path, destination: Path, run):
    """Fetch immutable sparse checkouts and create a private source-only training tree."""
    environment = {
        "GIT_CONFIG_GLOBAL": os.devnull,
        "GIT_CONFIG_NOSYSTEM": "1",
        "GIT_TERMINAL_PROMPT": "0",
        "GIT_LFS_SKIP_SMUDGE": "1",
    }
    projects = []
    for project in PROJECTS:
        checkout = cache / project.name
        checkout.mkdir(parents=True, exist_ok=True)
        git = [
            "git",
            "-c",
            f"core.hooksPath={os.devnull}",
            "-c",
            "core.autocrlf=false",
            "-C",
            checkout,
        ]
        if not (checkout / ".git").exists():
            run([*git, "init", "--quiet"], environment)
            run([*git, "remote", "add", "origin", project.url], environment)
        if run([*git, "remote", "get-url", "origin"], environment) != project.url:
            raise ValueError(f"Unexpected origin for cached {project.name} checkout")
        run(
            [*git, "sparse-checkout", "set", "--cone", *project.source_directories],
            environment,
        )
        run(
            [
                *git,
                "fetch",
                "--quiet",
                "--no-tags",
                "--no-recurse-submodules",
                "--depth=1",
                "--filter=blob:none",
                "origin",
                project.revision,
            ],
            environment,
        )
        run(
            [
                *git,
                "checkout",
                "--quiet",
                "--detach",
                "--force",
                "--no-recurse-submodules",
                project.revision,
            ],
            environment,
        )
        if run([*git, "rev-parse", "HEAD"], environment) != project.revision:
            raise ValueError(f"Unexpected revision for {project.name}")
        tracked = run(
            [*git, "ls-files", "-z", "--", *project.source_directories], environment
        )
        projects.append(
            copy_sources(
                project, checkout, destination / project.name, tracked.split("\0")
            )
        )
    return projects


def clean_bytecode(corpus: Path):
    for cache in corpus.rglob("__pycache__"):
        shutil.rmtree(cache)


def require_profiles(directory: Path, group: str):
    profiles = sorted(directory.glob(f"{group}-*.profraw"))
    if not profiles or any(path.stat().st_size == 0 for path in profiles):
        raise ValueError(f"Missing or empty profiles for training group {group}")
    return profiles


def bytecode_hashes(corpus: Path):
    return {
        path.relative_to(corpus).as_posix(): hashlib.sha256(
            path.read_bytes()
        ).hexdigest()
        for path in sorted(corpus.rglob("*.pyc"))
    }


def verify_bytecode(corpus: Path, expected, label="CPython"):
    actual = tree_summary(corpus, ".pyc")
    if actual != expected:
        raise ValueError(
            f"Corpus .pyc contents differ from {label}: expected {expected}, got {actual}"
        )
    return actual
