from __future__ import annotations

import os
import sys
import sysconfig


class SercNotFound(FileNotFoundError): ...


def find_serc_bin() -> str:
    """Return the serc binary path."""

    serc_exe = "serc" + sysconfig.get_config_var("EXE")

    targets = [
        # Above the package root, e.g., from `pip install --prefix`
        (
            # On Windows, with module path `<prefix>/Lib/site-packages/serc`
            _join(
                _matching_parents(_module_path(), "Lib/site-packages/serc"), "Scripts"
            )
            if sys.platform == "win32"
            # On Unix,  with module path `<prefix>/lib/python3.13/site-packages/serc`
            else _join(
                _matching_parents(_module_path(), "lib/python*/site-packages/serc"),
                "bin",
            )
        ),
        # Adjacent to the package root, e.g., from `pip install --target`
        # with module path `<target>/serc`
        _join(_matching_parents(_module_path(), "serc"), "bin"),
        # The scripts directory for the current Python
        sysconfig.get_path("scripts"),
        # The scripts directory for the base prefix
        sysconfig.get_path("scripts", vars={"base": sys.base_prefix}),
        # The user scheme scripts directory, e.g., `~/.local/bin`
        sysconfig.get_path("scripts", scheme=_user_scheme()),
    ]

    seen = []
    for target in targets:
        if not target:
            continue
        if target in seen:
            continue
        seen.append(target)
        path = os.path.join(target, serc_exe)
        if os.path.isfile(path):
            return path

    locations = "\n".join(f" - {target}" for target in seen)
    raise SercNotFound(
        f"Could not find the serc binary in any of the following locations:\n{locations}\n"
    )


def _module_path() -> str | None:
    path = os.path.dirname(__file__)
    return path


def _matching_parents(path: str | None, match: str) -> str | None:
    """
    Return the parent directory of `path` after trimming a `match` from the end.
    The match is expected to contain `/` as a path separator, while the `path`
    is expected to use the platform's path separator (e.g., `os.sep`). The path
    components are matched using `fnmatch`, which normalizes case according to
    the platform and supports shell-style wildcards in the `match`.
    """
    from fnmatch import fnmatch

    if not path:
        return None
    parts = path.split(os.sep)
    match_parts = match.split("/")
    if len(parts) < len(match_parts):
        return None

    if not all(
        fnmatch(part, match_part)
        for part, match_part in zip(reversed(parts), reversed(match_parts))
    ):
        return None

    return os.sep.join(parts[: -len(match_parts)])


def _join(path: str | None, *parts: str) -> str | None:
    if not path:
        return None
    return os.path.join(path, *parts)


def _user_scheme() -> str:
    if sys.version_info >= (3, 10):
        user_scheme = sysconfig.get_preferred_scheme("user")
    elif os.name == "nt":
        user_scheme = "nt_user"
    elif sys.platform == "darwin" and sys._framework:
        user_scheme = "osx_framework_user"
    else:
        user_scheme = "posix_user"
    return user_scheme
