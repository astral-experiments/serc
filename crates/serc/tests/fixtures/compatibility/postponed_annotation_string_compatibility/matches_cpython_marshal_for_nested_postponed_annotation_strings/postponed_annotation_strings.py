from __future__ import annotations

from typing import Annotated, Literal


top_plain: "Forward"
top_raw: r"Forward"
top_unicode: u"Forward"
top_upper_unicode: U"Forward"
top_late_unicode: "For" u"ward"


module_raw: Literal[r"type"]
module_upper_raw: Literal[R"type"]
module_adjacent: Annotated[str, "one" " two"]
module_backslash: Annotated[str, r"\noplayers\1"]
module_quotes: Annotated[str, "contains 'single' and \"double\" quotes"]
module_unicode: Annotated[str, u"one" " two"]
module_upper_unicode: Annotated[str, U"one" " two"]
module_late_unicode: Annotated[str, "one" u" two"]
module_simple_union: u"one" " two" | (None | Resource.Type)
module_right_union: A | (B | "forward")
module_string_attribute: "forward".Type | None


class Resource:
    raw: Literal[r"json"]
    adjacent: Annotated[str, "class" " value"]
    pattern: Annotated[str, r"(?i)^OL\d+M$"]
    quotes: Annotated[str, "class 'single' and \"double\" quotes"]
    unicode: Annotated[str, u"class" " value"]
    late_unicode: Annotated[str, "class" u" value"]
    upper_unicode: Annotated[str, U"class" " value"]


def function(
    raw: Literal[r"ok"],
    adjacent: Annotated[str, "function" " value"],
    pattern: Annotated[str, r"[A-Z0-9]+"],
    quotes: Annotated[str, "function 'single' and \"double\" quotes"],
    unicode: Annotated[str, u"function" " value"],
    late_unicode: Annotated[str, "function" u" value"],
    upper_unicode: Annotated[str, U"function" " value"],
) -> Literal[r"result"]:
    return raw


bytes_control: Literal[b"\xff"]
formatted_control: Annotated[str, f"value {1}"]
template_control: Annotated[str, t"value {1}"]
