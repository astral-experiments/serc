def triple_quoted():
    return """alpha
beta
gamma"""


def triple_bytes():
    return b"""alpha
beta"""


def implicit_concatenation(flag):
    if flag:
        return ("left "
                "right")
    return (b"fallback "
            b"bytes")


def parenthesized_string():
    return (
        "parenthesized"
    )


def parenthesized_scalar():
    return (
        42
    )


def one_line_literal():
    return "one line"


def multiline_nonliteral(value):
    return (
        value
    )
