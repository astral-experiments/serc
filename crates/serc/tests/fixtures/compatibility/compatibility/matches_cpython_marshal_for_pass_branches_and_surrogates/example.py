def pass_elif(value):
    if value:
        pass
    elif check(value):
        value = update(value)
    return value

def surrogates():
    explicit_pair = "\ud800\udc00heythere"
    lone_surrogate = "\ud800"
    mixed = "�\ud800"
    scalar = "\U00010000heythere"
    return explicit_pair, lone_surrogate, mixed, scalar
