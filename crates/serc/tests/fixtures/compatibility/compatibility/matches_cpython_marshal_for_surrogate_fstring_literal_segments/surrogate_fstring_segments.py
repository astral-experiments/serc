def regular(first, second):
    return f"\ud83d{first}\udc0d{second}\udfff"

def replacement_order(value):
    return f"\ufffd\ud800{value}\udfff\ufffd"

def raw_then_regular(value):
    return fr"\ud800 \ufffd {value}" f"\udfff"

def regular_then_raw(value):
    return f"\ud800{value}" rf"\udfff \ufffd"

def raw_actual_replacement(value):
    return rf"\ud800 �{value}"

def plain_then_fstring(value):
    return "\ud83d" f"{value}\udc0d"

def format_spec(value):
    return f"{value:\ud800}"

def dynamic_format_spec(value, width):
    return f"{value:\ud800{width}\udfff}"

def raw_format_spec(value, width):
    return rf"{value:\ud800 �{width}}"

def adjacent_regular():
    return f"\ud800" f"\udfff"

def adjacent_raw_then_regular():
    return rf"\ud800 �" f"\udfff"

def adjacent_regular_then_raw():
    return f"\ud800" rf"\udfff �"

def adjacent_plain_then_fstring():
    return "\ud800" f"\udfff"

def large(value):
    return f"\ud800{value}a{value}b{value}c{value}d{value}e{value}f{value}g{value}h{value}i{value}j{value}k{value}l{value}m{value}n{value}o{value}p\udfff"
