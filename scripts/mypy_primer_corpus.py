#!/usr/bin/env python3
"""Manage Serc's pinned mypy_primer-derived compatibility corpus."""

from __future__ import annotations

import argparse
import ast
import concurrent.futures
import datetime as dt
import hashlib
import json
import os
import re
import subprocess
import sys
from collections import defaultdict
from pathlib import Path
from typing import Any, Iterable, Sequence
from urllib.parse import urlparse


REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_LOCK = REPO_ROOT / "corpora" / "mypy-primer.lock.json"
DEFAULT_CORPUS = REPO_ROOT / "target" / "corpora" / "mypy-primer"
CATALOG_URL = "https://github.com/hauntsaninja/mypy_primer"
SOURCE_SUFFIXES = (".py", ".pyi")
LOCK_FORMAT = 1
EXPECTED_FAILURES = 6_611
EXPECTED_CPYTHON_REJECTED = 160
HEX_REVISION = re.compile(r"[0-9a-f]{40}\Z")
SAFE_ID = re.compile(r"[A-Za-z0-9._-]+\Z")


class CorpusError(RuntimeError):
    """A deterministic corpus operation failed."""


def run(
    arguments: Sequence[str | os.PathLike[str]],
    *,
    cwd: Path | None = None,
    check: bool = True,
) -> subprocess.CompletedProcess[str]:
    command = [os.fspath(argument) for argument in arguments]
    process = subprocess.run(
        command,
        cwd=cwd,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        encoding="utf-8",
        errors="replace",
        check=False,
    )
    if check and process.returncode:
        detail = process.stderr.strip() or process.stdout.strip()
        raise CorpusError(f"`{' '.join(command)}` failed: {detail}")
    return process


def git(
    arguments: Sequence[str], *, cwd: Path | None = None, check: bool = True
) -> subprocess.CompletedProcess[str]:
    return run(["git", *arguments], cwd=cwd, check=check)


def read_lock(path: Path) -> dict[str, Any]:
    try:
        lock = json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError as error:
        raise CorpusError(f"corpus lock is missing: {path}") from error
    except json.JSONDecodeError as error:
        raise CorpusError(f"invalid corpus lock {path}: {error}") from error
    validate_lock(lock)
    return lock


def validate_lock(lock: dict[str, Any]) -> None:
    if not isinstance(lock, dict):
        raise CorpusError("corpus lock must be an object")
    if lock.get("format") != LOCK_FORMAT:
        raise CorpusError(
            f"unsupported corpus lock format {lock.get('format')!r}; expected {LOCK_FORMAT}"
        )
    catalog = lock.get("catalog")
    if not isinstance(catalog, dict) or catalog.get("url") != CATALOG_URL:
        raise CorpusError("corpus lock has an invalid mypy_primer catalog")
    if not is_revision(catalog.get("revision")):
        raise CorpusError("corpus lock has an invalid mypy_primer revision")
    entries = catalog.get("entries")
    repository_count = catalog.get("repositories")
    if type(entries) is not int or entries <= 0:
        raise CorpusError("corpus lock has an invalid catalog entry count")
    if type(repository_count) is not int or repository_count <= 0:
        raise CorpusError("corpus lock has an invalid catalog repository count")

    resolved_at = lock.get("resolved_at")
    if not isinstance(resolved_at, str):
        raise CorpusError("corpus lock has an invalid resolution timestamp")
    try:
        resolved_time = dt.datetime.fromisoformat(resolved_at)
    except ValueError as error:
        raise CorpusError("corpus lock has an invalid resolution timestamp") from error
    if resolved_time.tzinfo is None:
        raise CorpusError("corpus lock resolution timestamp must include a timezone")
    if lock.get("source_suffixes") != list(SOURCE_SUFFIXES):
        raise CorpusError("corpus lock source suffixes do not match the corpus policy")

    repositories = lock.get("repositories")
    if not isinstance(repositories, list) or not repositories:
        raise CorpusError("corpus lock contains no repositories")
    identifiers: set[str] = set()
    urls: set[str] = set()
    for repository in repositories:
        if not isinstance(repository, dict):
            raise CorpusError("corpus lock repository entries must be objects")
        identifier = repository.get("id")
        url = repository.get("url")
        revision = repository.get("revision")
        logical_projects = repository.get("logical_projects")
        if not isinstance(identifier, str) or not SAFE_ID.fullmatch(identifier):
            raise CorpusError(f"invalid repository id: {identifier!r}")
        if identifier in identifiers:
            raise CorpusError(f"duplicate repository id: {identifier}")
        if not isinstance(url, str) or not url.startswith("https://"):
            raise CorpusError(f"invalid repository URL for {identifier}: {url!r}")
        if url in urls:
            raise CorpusError(f"duplicate repository URL: {url}")
        if not is_revision(revision):
            raise CorpusError(f"invalid revision for {identifier}: {revision!r}")
        if (
            not isinstance(logical_projects, list)
            or not logical_projects
            or any(
                not isinstance(project, str) or not project
                for project in logical_projects
            )
            or len(logical_projects) != len(set(logical_projects))
        ):
            raise CorpusError(f"invalid logical projects for {identifier}")
        identifiers.add(identifier)
        urls.add(url)

    if repositories != sorted(repositories, key=lambda repository: repository["id"]):
        raise CorpusError("corpus lock repositories are not sorted by id")
    if len(repositories) != repository_count:
        raise CorpusError("catalog repository count does not match the repository lock")
    if sum(len(repository["logical_projects"]) for repository in repositories) != entries:
        raise CorpusError("catalog entry count does not match the logical project lock")

    content = lock.get("content")
    if content is not None:
        if not isinstance(content, dict):
            raise CorpusError("corpus lock content must be an object or null")
        if type(content.get("files")) is not int or content["files"] <= 0:
            raise CorpusError("corpus lock has an invalid content file count")
        if type(content.get("bytes")) is not int or content["bytes"] <= 0:
            raise CorpusError("corpus lock has an invalid content byte count")
        if not isinstance(content.get("sha256"), str) or not re.fullmatch(
            r"[0-9a-f]{64}", content["sha256"]
        ):
            raise CorpusError("corpus lock has an invalid content digest")


def is_revision(value: object) -> bool:
    return isinstance(value, str) and HEX_REVISION.fullmatch(value) is not None


def write_lock(path: Path, lock: dict[str, Any]) -> None:
    validate_lock(lock)
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(
        json.dumps(lock, indent=2, sort_keys=False) + "\n", encoding="utf-8"
    )
    temporary.replace(path)


def literal_keyword(call: ast.Call, name: str) -> str | None:
    for keyword in call.keywords:
        if keyword.arg == name:
            try:
                value = ast.literal_eval(keyword.value)
            except (TypeError, ValueError) as error:
                raise CorpusError(
                    f"Project.{name} must be a string literal or None"
                ) from error
            if value is not None and not isinstance(value, str):
                raise CorpusError(f"Project.{name} must be a string literal or None")
            return value
    return None


def read_catalog(checkout: Path) -> tuple[str, list[tuple[str, str]]]:
    projects_file = checkout / "mypy_primer" / "projects.py"
    try:
        source = projects_file.read_text(encoding="utf-8")
    except FileNotFoundError as error:
        raise CorpusError(f"mypy_primer project catalog is missing: {projects_file}") from error
    tree = ast.parse(source, filename=str(projects_file))
    calls: list[ast.Call] | None = None
    for node in tree.body:
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)) and node.name == "get_projects":
            for statement in node.body:
                if (
                    isinstance(statement, ast.Assign)
                    and any(
                        isinstance(target, ast.Name) and target.id == "projects"
                        for target in statement.targets
                    )
                    and isinstance(statement.value, ast.List)
                ):
                    calls = []
                    for element in statement.value.elts:
                        if not (
                            isinstance(element, ast.Call)
                            and isinstance(element.func, ast.Name)
                            and element.func.id == "Project"
                        ):
                            raise CorpusError(
                                "mypy_primer catalog must be a static list of Project calls"
                            )
                        calls.append(element)
                    break
    if not calls:
        raise CorpusError("could not find the static Project list in mypy_primer")

    entries: list[tuple[str, str]] = []
    for call in calls:
        location = literal_keyword(call, "location")
        if location is None:
            raise CorpusError("mypy_primer Project entry has no literal location")
        override = literal_keyword(call, "name_override")
        name = override or Path(location.rstrip("/")).name.removesuffix(".git")
        entries.append((location, name))

    revision = git(["rev-parse", "HEAD"], cwd=checkout).stdout.strip()
    if not is_revision(revision):
        raise CorpusError(f"invalid mypy_primer checkout revision: {revision!r}")
    dirty = git(
        [
            "status",
            "--porcelain",
            "--untracked-files=no",
            "--",
            str(projects_file.relative_to(checkout)),
        ],
        cwd=checkout,
    ).stdout
    if dirty:
        raise CorpusError(f"mypy_primer project catalog has local changes: {dirty.strip()}")
    return revision, entries


def group_catalog(entries: Iterable[tuple[str, str]]) -> dict[str, list[str]]:
    by_url: dict[str, list[str]] = defaultdict(list)
    for url, name in entries:
        if name not in by_url[url]:
            by_url[url].append(name)
    return dict(by_url)


def check_catalog(args: argparse.Namespace) -> None:
    lock = read_lock(args.lock)
    revision, entries = read_catalog(args.primer_checkout.resolve())
    expected_revision = lock["catalog"]["revision"]
    if revision != expected_revision:
        raise CorpusError(
            f"mypy_primer revision mismatch: {revision} != {expected_revision}"
        )
    actual = group_catalog(entries)
    expected = {
        repository["url"]: repository["logical_projects"]
        for repository in lock["repositories"]
    }
    if actual != expected:
        missing = sorted(expected.keys() - actual.keys())
        added = sorted(actual.keys() - expected.keys())
        changed = sorted(
            url for url in actual.keys() & expected.keys() if actual[url] != expected[url]
        )
        raise CorpusError(
            "mypy_primer catalog does not match the corpus lock: "
            f"missing={missing}, added={added}, changed={changed}"
        )
    if lock["catalog"].get("entries") != len(entries):
        raise CorpusError("mypy_primer catalog entry count does not match the corpus lock")
    if lock["catalog"].get("repositories") != len(actual):
        raise CorpusError("mypy_primer repository count does not match the corpus lock")
    print(
        f"Verified {len(entries)} mypy_primer projects in {len(actual)} repositories "
        f"at {revision}"
    )


def repository_id(url: str) -> str:
    parsed = urlparse(url)
    path = parsed.path.strip("/").removesuffix(".git")
    identifier = f"{parsed.netloc}--{path.replace('/', '--')}"
    identifier = re.sub(r"[^A-Za-z0-9._-]", "-", identifier)
    if not identifier or not SAFE_ID.fullmatch(identifier):
        raise CorpusError(f"could not form a safe repository id for {url}")
    return identifier


def resolve_remote(url: str) -> tuple[str, str | None]:
    output = git(["ls-remote", "--symref", url, "HEAD"]).stdout.splitlines()
    revision: str | None = None
    default_ref: str | None = None
    for line in output:
        fields = line.split("\t")
        if len(fields) != 2 or fields[1] != "HEAD":
            continue
        if fields[0].startswith("ref: "):
            default_ref = fields[0].removeprefix("ref: ")
        elif is_revision(fields[0]):
            revision = fields[0]
    if revision is None:
        raise CorpusError(f"could not resolve remote HEAD for {url}")
    return revision, default_ref


def update_lock(args: argparse.Namespace) -> None:
    checkout = args.primer_checkout.resolve()
    revision, entries = read_catalog(checkout)
    by_url = group_catalog(entries)

    resolved: dict[str, tuple[str, str | None]] = {}
    failures: list[str] = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as executor:
        futures = {executor.submit(resolve_remote, url): url for url in by_url}
        for future in concurrent.futures.as_completed(futures):
            url = futures[future]
            try:
                resolved[url] = future.result()
            except Exception as error:  # report every failed remote together
                failures.append(f"{url}: {error}")
    if failures:
        raise CorpusError("failed to resolve repository revisions:\n  " + "\n  ".join(failures))

    repositories = []
    for url, names in by_url.items():
        project_revision, default_ref = resolved[url]
        repository: dict[str, Any] = {
            "id": repository_id(url),
            "url": url,
            "revision": project_revision,
            "logical_projects": names,
        }
        if default_ref is not None:
            repository["default_ref"] = default_ref
        repositories.append(repository)
    repositories.sort(key=lambda repository: repository["id"])

    lock = {
        "format": LOCK_FORMAT,
        "catalog": {
            "url": CATALOG_URL,
            "revision": revision,
            "entries": len(entries),
            "repositories": len(repositories),
        },
        "resolved_at": dt.datetime.now(dt.UTC).replace(microsecond=0).isoformat(),
        "source_suffixes": list(SOURCE_SUFFIXES),
        "content": None,
        "repositories": repositories,
    }
    write_lock(args.lock, lock)
    print(
        f"Locked {len(entries)} mypy_primer projects in {len(repositories)} repositories "
        f"at catalog revision {revision}"
    )


def checkout_path(corpus_root: Path, repository: dict[str, Any]) -> Path:
    return corpus_root / "checkouts" / repository["id"]


def current_revision(checkout: Path) -> str | None:
    process = git(["rev-parse", "--verify", "HEAD"], cwd=checkout, check=False)
    return process.stdout.strip() if process.returncode == 0 else None


def require_clean(checkout: Path) -> None:
    status = git(["status", "--porcelain", "--untracked-files=all"], cwd=checkout).stdout
    if status:
        raise CorpusError(f"checkout has local changes: {checkout}\n{status.rstrip()}")


def setup_repository(corpus_root: Path, repository: dict[str, Any]) -> str:
    checkout = checkout_path(corpus_root, repository)
    checkout.parent.mkdir(parents=True, exist_ok=True)
    if checkout.exists() and not (checkout / ".git").is_dir():
        raise CorpusError(f"corpus path exists but is not a Git checkout: {checkout}")
    if not checkout.exists():
        git(["init", "--quiet", str(checkout)])
        git(["remote", "add", "origin", repository["url"]], cwd=checkout)
        git(["sparse-checkout", "init", "--no-cone"], cwd=checkout)
        git(
            ["sparse-checkout", "set", "--no-cone", *SOURCE_SUFFIXES_AS_GLOBS],
            cwd=checkout,
        )
    else:
        origin = git(["remote", "get-url", "origin"], cwd=checkout).stdout.strip()
        if origin != repository["url"]:
            raise CorpusError(
                f"checkout origin mismatch for {checkout}: {origin!r} != {repository['url']!r}"
            )
        require_clean(checkout)

    revision = repository["revision"]
    if current_revision(checkout) != revision:
        fetch = git(
            ["fetch", "--depth", "1", "--filter=blob:none", "origin", revision],
            cwd=checkout,
            check=False,
        )
        if fetch.returncode:
            # Some servers forbid fetching an object by ID. Fetch their refs without blobs,
            # then retry the locked object locally; the final identity check remains exact.
            git(["fetch", "--filter=blob:none", "--no-tags", "origin"], cwd=checkout)
        git(["checkout", "--quiet", "--detach", "--force", revision], cwd=checkout)

    actual = current_revision(checkout)
    if actual != revision:
        raise CorpusError(f"checkout revision mismatch for {checkout}: {actual} != {revision}")
    require_clean(checkout)
    return repository["id"]


SOURCE_SUFFIXES_AS_GLOBS = tuple(f"*{suffix}" for suffix in SOURCE_SUFFIXES)


def setup(args: argparse.Namespace) -> None:
    lock = read_lock(args.lock)
    args.corpus_root.mkdir(parents=True, exist_ok=True)
    failures: list[str] = []
    completed: list[str] = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as executor:
        futures = {
            executor.submit(setup_repository, args.corpus_root, repository): repository["id"]
            for repository in lock["repositories"]
        }
        for future in concurrent.futures.as_completed(futures):
            identifier = futures[future]
            try:
                completed.append(future.result())
            except Exception as error:
                failures.append(f"{identifier}: {error}")
    if failures:
        raise CorpusError("failed to set up corpus repositories:\n  " + "\n  ".join(failures))
    print(f"Prepared {len(completed)} locked repositories under {args.corpus_root}")


def iter_sources(
    corpus_root: Path, repositories: Iterable[dict[str, Any]]
) -> Iterable[tuple[str, Path, Path]]:
    for repository in repositories:
        checkout = checkout_path(corpus_root, repository)
        for directory, names, filenames in os.walk(checkout, followlinks=False):
            names[:] = sorted(
                name
                for name in names
                if name != ".git" and not (Path(directory) / name).is_symlink()
            )
            for filename in sorted(filenames):
                path = Path(directory) / filename
                if path.suffix in SOURCE_SUFFIXES and path.is_file() and not path.is_symlink():
                    yield repository["id"], path.relative_to(checkout), path


def inspect_content(
    corpus_root: Path, repositories: Sequence[dict[str, Any]]
) -> dict[str, Any]:
    digest = hashlib.sha256()
    digest.update(b"serc-mypy-primer-corpus-v1\0")
    files = 0
    source_bytes = 0
    for identifier, relative, path in iter_sources(corpus_root, repositories):
        content = path.read_bytes()
        identifier_bytes = identifier.encode("utf-8")
        relative_bytes = relative.as_posix().encode("utf-8", "surrogateescape")
        digest.update(len(identifier_bytes).to_bytes(4, "big"))
        digest.update(identifier_bytes)
        digest.update(len(relative_bytes).to_bytes(4, "big"))
        digest.update(relative_bytes)
        digest.update(len(content).to_bytes(8, "big"))
        digest.update(content)
        files += 1
        source_bytes += len(content)
    if files == 0:
        raise CorpusError(f"corpus contains no Python sources under {corpus_root}")
    return {"files": files, "bytes": source_bytes, "sha256": digest.hexdigest()}


def verify_checkouts(
    corpus_root: Path, lock: dict[str, Any], *, verify_content: bool = True
) -> dict[str, Any] | None:
    for repository in lock["repositories"]:
        checkout = checkout_path(corpus_root, repository)
        if not (checkout / ".git").is_dir():
            raise CorpusError(
                f"locked checkout is missing: {checkout}\n"
                f"Run `{REPO_ROOT / 'scripts' / 'setup-mypy-primer-corpus'}` first."
            )
        origin = git(["remote", "get-url", "origin"], cwd=checkout).stdout.strip()
        if origin != repository["url"]:
            raise CorpusError(f"checkout origin mismatch for {checkout}: {origin!r}")
        actual = current_revision(checkout)
        if actual != repository["revision"]:
            raise CorpusError(
                f"checkout revision mismatch for {checkout}: "
                f"{actual} != {repository['revision']}"
            )
        require_clean(checkout)

    if not verify_content:
        return None
    actual_content = inspect_content(corpus_root, lock["repositories"])
    expected_content = lock.get("content")
    if expected_content is None:
        raise CorpusError(
            "corpus lock has no frozen content identity; run "
            "`scripts/mypy-primer-corpus.py lock-content` as part of an intentional update"
        )
    if actual_content != expected_content:
        raise CorpusError(
            "corpus content mismatch:\n"
            f"  expected {expected_content}\n"
            f"  actual   {actual_content}"
        )
    return actual_content


def verify(args: argparse.Namespace) -> None:
    lock = read_lock(args.lock)
    content = verify_checkouts(args.corpus_root, lock)
    assert content is not None
    print(
        f"Verified {len(lock['repositories'])} repositories, {content['files']} files, "
        f"{content['bytes']} bytes, sha256 {content['sha256']}"
    )


def lock_content(args: argparse.Namespace) -> None:
    lock = read_lock(args.lock)
    verify_checkouts(args.corpus_root, {**lock, "content": None}, verify_content=False)
    content = inspect_content(args.corpus_root, lock["repositories"])
    lock["content"] = content
    write_lock(args.lock, lock)
    print(
        f"Locked {content['files']} files, {content['bytes']} bytes, "
        f"sha256 {content['sha256']}"
    )


def compatibility_command(
    python: str,
    roots: Sequence[Path],
    files: int,
    limit: int | None,
) -> list[str]:
    command = [
        "cargo",
        "run",
        "--locked",
        "--manifest-path",
        str(REPO_ROOT / "Cargo.toml"),
        "-p",
        "serc",
        "--example",
        "compare_cpython_corpus",
        "--",
        "--python",
        python,
        "--expected-files",
        str(files),
        "--source-bytes",
        "--extension",
        "pyi",
    ]
    if limit is None:
        expected_exact = files - EXPECTED_FAILURES - EXPECTED_CPYTHON_REJECTED
        command.extend(
            [
                "--expected-exact",
                str(expected_exact),
                "--expected-cpython-rejected",
                str(EXPECTED_CPYTHON_REJECTED),
            ]
        )
    else:
        command.extend(["--limit", str(limit), "--require-all"])
    command.extend(map(str, roots))
    return command


def check_compatibility(args: argparse.Namespace) -> None:
    lock = read_lock(args.lock)
    content = verify_checkouts(args.corpus_root, lock)
    assert content is not None
    roots = [
        checkout_path(args.corpus_root, repository)
        for repository in lock["repositories"]
    ]
    command = compatibility_command(args.python, roots, content["files"], args.limit)
    os.execvp(command[0], command)


def positive_int(value: str) -> int:
    number = int(value)
    if number <= 0:
        raise argparse.ArgumentTypeError("must be positive")
    return number


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--lock", type=Path, default=DEFAULT_LOCK)
    result.add_argument("--corpus-root", type=Path, default=DEFAULT_CORPUS)
    subparsers = result.add_subparsers(dest="command", required=True)

    update_parser = subparsers.add_parser(
        "update-lock", help="resolve a pinned mypy_primer catalog to exact repository commits"
    )
    update_parser.add_argument("--primer-checkout", type=Path, required=True)
    update_parser.add_argument("--jobs", type=positive_int, default=min(8, os.cpu_count() or 1))
    update_parser.set_defaults(handler=update_lock)

    catalog_parser = subparsers.add_parser(
        "check-catalog", help="verify the lock against its pinned mypy_primer catalog"
    )
    catalog_parser.add_argument("--primer-checkout", type=Path, required=True)
    catalog_parser.set_defaults(handler=check_catalog)

    setup_parser = subparsers.add_parser("setup", help="fetch every locked repository")
    setup_parser.add_argument("--jobs", type=positive_int, default=min(8, os.cpu_count() or 1))
    setup_parser.set_defaults(handler=setup)

    verify_parser = subparsers.add_parser("verify", help="verify revisions and frozen source bytes")
    verify_parser.set_defaults(handler=verify)

    content_parser = subparsers.add_parser(
        "lock-content", help="record source count, byte count, and aggregate digest after an update"
    )
    content_parser.set_defaults(handler=lock_content)

    check_parser = subparsers.add_parser(
        "check", help="run byte-for-byte CPython compatibility over the frozen corpus"
    )
    check_parser.add_argument("--python", default=os.environ.get("CPYTHON", "python3.14"))
    check_parser.add_argument("--limit", type=positive_int)
    check_parser.set_defaults(handler=check_compatibility)
    return result


def main(argv: Sequence[str] | None = None) -> int:
    args = parser().parse_args(argv)
    args.lock = args.lock.resolve()
    args.corpus_root = args.corpus_root.resolve()
    try:
        args.handler(args)
    except (CorpusError, OSError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
