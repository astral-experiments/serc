def left():
    return 100 * "\udc00"

def right():
    return "\udc00" * 100

def mixed():
    return 2 * ("\ufffd" "\udc00")

def zero():
    return 0 * "\udc00"

def negative():
    return -1 * "\udc00"

def at_limit():
    return 4096 * "\udc00"

def over_limit():
    return 4097 * "\udc00"

def two_char_limit():
    return 2048 * "a\udc00"

def two_char_over_limit():
    return 2049 * "a\udc00"

def nested():
    return 2 * (3 * "\udc00")
