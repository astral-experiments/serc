"""Independently recompile function definitions in partially invalid pycdc output."""

from __future__ import annotations

import inspect
import io
import re
import token
import tokenize
from collections import Counter

from check_decompile_corpus import fingerprint
from decompile_flow import scopes


def statements(source):
    """First token on each code line. String contents and comments cannot start a def."""
    lines = {}
    at_start = True
    try:
        for item in tokenize.generate_tokens(io.StringIO(source).readline):
            if item.type == token.NEWLINE:
                at_start = True
                continue
            if item.type in {
                token.INDENT,
                token.DEDENT,
                token.NL,
                token.COMMENT,
                token.ENDMARKER,
            }:
                continue
            row, column = item.start
            name = item.string if item.type == token.NAME else None
            # A malformed expression can leave the tokenizer inside an open
            # bracket at the next definition. Try that header independently.
            if (at_start or name in {"async", "class", "def"}) and not item.line[
                :column
            ].strip():
                lines.setdefault(row - 1, (column, name))
            at_start = False
    except (SyntaxError, tokenize.TokenError):
        # An unfinished string or an invalid indent can stop the tokenizer.
        # Earlier complete definitions can still compile independently.
        pass
    return list(lines.items())


def compiled_functions(output, filename):
    try:
        source = output.decode("utf-8")
    except UnicodeError:
        return
    lines = io.StringIO(source).readlines()
    starts = statements(source)
    classes = []
    futures = []
    for index, (row, (indent, keyword)) in enumerate(starts):
        header = lines[row][indent:].rstrip()
        while classes and indent <= classes[-1][0]:
            classes.pop()
        # pycdc indents class members by four spaces. Only recreate classes
        # whose names were emitted, never a missing enclosing function.
        if indent != (classes[-1][0] + 4 if classes else 0):
            continue
        if (
            keyword == "from"
            and indent == 0
            and re.fullmatch(r"from __future__ import \w+(?:, \w+)*", header)
        ):
            futures.append(header + "\n")
            continue
        if keyword == "class" and (match := re.match(r"class (\w+)\b", header)):
            if match[1].isidentifier():
                classes.append((indent, match[1]))
            continue
        if keyword not in {"async", "def"} or not re.match(
            r"(?:async )?def \w+\(", header
        ):
            continue
        end = len(lines)
        for cursor in range(index + 1, len(starts)):
            next_row, (next_indent, _) = starts[cursor]
            if next_indent <= indent:
                end = next_row
                break
        wrappers = [" " * col + f"class {name}:\n" for col, name in classes]
        fragment = "".join([*futures, *wrappers, *lines[row:end]])
        try:
            compiled = compile(
                fragment, filename, "exec", dont_inherit=True, optimize=0
            )
        except (SyntaxError, ValueError, RecursionError):
            continue
        for code in scopes(compiled):
            if code.co_flags & inspect.CO_OPTIMIZED:
                yield code


def verified_fragments(output, filename, original):
    # An isolated definition cannot establish which occurrence of a repeated
    # name it represents. Exact fingerprints include name, flags, exception
    # table, constants, and all nested code, without assigning credit to wrappers.
    counts = Counter(code.co_qualname for code in scopes(original))
    unique = {
        code.co_qualname: code
        for code in scopes(original)
        if counts[code.co_qualname] == 1 and code.co_flags & inspect.CO_OPTIMIZED
    }
    return {
        (code.co_qualname, 1)
        for code in compiled_functions(output, filename)
        if (peer := unique.get(code.co_qualname)) is not None
        and fingerprint(code) == fingerprint(peer)
    }
